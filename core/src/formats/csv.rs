use std::{
  fs::File,
  io::{BufRead, BufReader, Seek, SeekFrom},
  path::Path,
};

use serde_json::{Map, Value};

use crate::{
  cursor::Cursor,
  engine::CoreError,
  formats::LinesPageInternal,
  models::{Record, RecordMeta},
};

/// CSV paging implementation:
/// - Record-based streaming (supports multi-line quoted cells).
/// - Additionally provides `Record.raw` as a JSON string, whose keys are the header row fields.
pub(crate) fn read_csv_page(
  path: &Path,
  cursor: Cursor,
  page_size: usize,
  preview_max_chars: usize,
  _raw_max_chars: usize, // unused: CSV always shows full content in detail view
) -> Result<(LinesPageInternal, Option<Cursor>), CoreError> {
  let headers = read_csv_header(path).unwrap_or_default();

  let mut file = File::open(path)?;
  let file_len = file.metadata().ok().map(|m| m.len()).unwrap_or(0);
  if cursor.offset > file_len {
    return Err(CoreError::BadCursor(format!(
      "offset {} beyond file len {}",
      cursor.offset, file_len
    )));
  }
  file.seek(SeekFrom::Start(cursor.offset))?;
  let mut reader = BufReader::new(file);

  let mut records = Vec::with_capacity(page_size);
  let mut offset = cursor.offset;
  let mut line_no = cursor.line;

  for _ in 0..page_size {
    let start_offset = offset;
    let mut buf = Vec::new();
    let (n, _terminated_by_newline) = read_csv_record_bytes(&mut reader, &mut buf)?;
    if n == 0 {
      break;
    }
    offset += n as u64;

    // Trim the *record terminator* (CRLF/LF) only.
    trim_record_terminator(&mut buf);

    let line = String::from_utf8_lossy(&buf).to_string();
    let preview = truncate_chars(&line, preview_max_chars);

    // Provide a JSON-like raw for details:
    // - header line keeps original raw text (backward compatible with existing behavior/tests)
    // - data line becomes {"colA":"...", "colB":"..."} with keys from header row
    // For CSV, always show full content in detail view (no truncation).
    // These files typically have reasonable line/cell lengths.
    let raw = if line_no == 0 {
      Some(line.clone())
    } else {
      let fields = parse_csv_line(&line);
      let mut obj = Map::new();
      for (i, h) in headers.iter().enumerate() {
        let v = fields.get(i).cloned().unwrap_or_default();
        obj.insert(h.clone(), Value::String(v));
      }
      if fields.len() > headers.len() {
        obj.insert(
          "__extra__".to_string(),
          Value::Array(
            fields[headers.len()..]
              .iter()
              .cloned()
              .map(Value::String)
              .collect(),
          ),
        );
      }
      let raw_json = serde_json::to_string(&Value::Object(obj))
        .unwrap_or_else(|_| format!(r#"{{"__raw__":"{}"}}"#, sanitize_json_string(&line)));
      Some(raw_json)
    };

    records.push(Record {
      id: line_no,
      preview,
      raw,
      meta: Some(RecordMeta {
        line_no,
        byte_offset: start_offset,
        byte_len: n as u64,
      }),
    });
    line_no += 1;
  }

  let reached_eof = records.is_empty() || offset >= file_len;
  let next = if reached_eof {
    None
  } else {
    Some(Cursor {
      offset,
      line: cursor.line + records.len() as u64,
    })
  };

  Ok((
    LinesPageInternal {
      records,
      reached_eof,
    },
    next,
  ))
}

fn read_csv_header(path: &Path) -> Result<Vec<String>, CoreError> {
  let file = File::open(path)?;
  let mut reader = BufReader::new(file);
  let mut buf = Vec::new();
  let (n, _terminated_by_newline) = read_csv_record_bytes(&mut reader, &mut buf)?;
  if n == 0 {
    return Ok(vec![]);
  }
  trim_record_terminator(&mut buf);
  let mut line = String::from_utf8_lossy(&buf).to_string();
  // Strip UTF-8 BOM if present
  if line.starts_with('\u{feff}') {
    line = line.trim_start_matches('\u{feff}').to_string();
  }
  let mut headers = parse_csv_line(&line);
  // Normalize empty headers to generic names.
  for (i, h) in headers.iter_mut().enumerate() {
    if h.trim().is_empty() {
      *h = format!("col_{i}");
    }
  }
  if headers.is_empty() {
    // Fallback: still provide at least one header so detail view is not empty.
    headers.push("col_0".to_string());
  }
  Ok(headers)
}

/// Read a single CSV *record* into `out`, streaming from `reader`.
///
/// Unlike `read_until('\n')`, this treats newlines inside quoted fields as part of the record,
/// and only ends the record when it sees a line break while **not** inside quotes.
///
/// Returns:
/// - bytes consumed from reader (including the record terminator if present)
/// - whether the record ended due to a newline terminator (as opposed to EOF)
fn read_csv_record_bytes<R: BufRead>(reader: &mut R, out: &mut Vec<u8>) -> Result<(usize, bool), CoreError> {
  out.clear();

  let mut in_quotes = false;
  let mut at_field_start = true;
  let mut consumed = 0usize;
  let mut terminated_by_newline = false;

  loop {
    let mut chunk = Vec::new();
    let n = reader.read_until(b'\n', &mut chunk)?;
    if n == 0 {
      break; // EOF
    }
    consumed += n;

    // Update quote state based on bytes before the '\n' (if present).
    let scan_slice = if chunk.ends_with(b"\n") {
      &chunk[..chunk.len().saturating_sub(1)]
    } else {
      chunk.as_slice()
    };
    update_csv_quote_state(&mut in_quotes, &mut at_field_start, scan_slice);

    out.extend_from_slice(&chunk);

    if chunk.ends_with(b"\n") && !in_quotes {
      terminated_by_newline = true;
      break;
    }

    // If read_until hit EOF without a trailing '\n', we have a (possibly unterminated) last record.
    if !chunk.ends_with(b"\n") {
      break;
    }
  }

  Ok((consumed, terminated_by_newline))
}

fn update_csv_quote_state(in_quotes: &mut bool, at_field_start: &mut bool, bytes: &[u8]) {
  let mut i = 0usize;
  while i < bytes.len() {
    let b = bytes[i];

    if *in_quotes {
      if b == b'"' {
        // Escaped quote inside quoted field: ""
        if i + 1 < bytes.len() && bytes[i + 1] == b'"' {
          i += 2;
          continue;
        }
        *in_quotes = false;
      }
      i += 1;
      continue;
    }

    match b {
      b',' => {
        *at_field_start = true;
      }
      // Allow leading spaces/tabs before an opening quote.
      b' ' | b'\t' if *at_field_start => {}
      b'"' if *at_field_start => {
        *in_quotes = true;
        *at_field_start = false;
      }
      _ => {
        *at_field_start = false;
      }
    }
    i += 1;
  }
}

fn trim_record_terminator(buf: &mut Vec<u8>) {
  // Trim LF
  if buf.ends_with(b"\n") {
    buf.pop();
    // Trim CR before LF
    if buf.ends_with(b"\r") {
      buf.pop();
    }
  }
}

/// Best-effort single-line CSV parser:
/// - Supports quotes and escaped quotes ("")
/// - Works fine with multi-line records as long as the record text is provided in full
fn parse_csv_line(line: &str) -> Vec<String> {
  let mut out: Vec<String> = Vec::new();
  let mut cur = String::new();
  let mut in_quotes = false;
  let mut chars = line.chars().peekable();

  while let Some(ch) = chars.next() {
    match ch {
      '"' => {
        if in_quotes && matches!(chars.peek(), Some('"')) {
          // Escaped quote
          cur.push('"');
          let _ = chars.next();
        } else {
          in_quotes = !in_quotes;
        }
      }
      ',' if !in_quotes => {
        out.push(cur);
        cur = String::new();
      }
      _ => cur.push(ch),
    }
  }
  out.push(cur);

  out
}

fn truncate_chars(s: &str, max: usize) -> String {
  if max == 0 {
    return String::new();
  }
  let mut out = String::new();
  for (i, ch) in s.chars().enumerate() {
    if i >= max {
      out.push_str("…");
      break;
    }
    out.push(ch);
  }
  out
}

fn sanitize_json_string(s: &str) -> String {
  // Minimal escaping for fallback JSON construction (only used in error paths).
  s.replace('\\', "\\\\").replace('"', "\\\"")
}

