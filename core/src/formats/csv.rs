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
/// - Still line-based for fast first screen (does NOT support multi-line quoted cells).
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
    let n = reader.read_until(b'\n', &mut buf)?;
    if n == 0 {
      break;
    }
    offset += n as u64;

    // Trim newline & CRLF
    if buf.ends_with(b"\n") {
      buf.pop();
      if buf.ends_with(b"\r") {
        buf.pop();
      }
    }

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
  let n = reader.read_until(b'\n', &mut buf)?;
  if n == 0 {
    return Ok(vec![]);
  }
  if buf.ends_with(b"\n") {
    buf.pop();
    if buf.ends_with(b"\r") {
      buf.pop();
    }
  }
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

/// Best-effort single-line CSV parser:
/// - Supports quotes and escaped quotes ("")
/// - Does not support multi-line quoted fields (by design for streaming preview)
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

