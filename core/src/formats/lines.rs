use std::{
  fs::File,
  io::{BufRead, BufReader, Seek, SeekFrom},
  path::Path,
};

use crate::{
  cursor::Cursor,
  engine::CoreError,
  formats::LinesPageInternal,
  models::{Record, RecordMeta},
};

pub(crate) fn read_lines_page(
  path: &Path,
  cursor: Cursor,
  page_size: usize,
  preview_max_chars: usize,
  _raw_max_chars: usize, // unused: JSONL always shows full content in detail view
) -> Result<(LinesPageInternal, Option<Cursor>), CoreError> {
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
    // For JSONL, always show full content in detail view (no truncation).
    // These files typically have reasonable line lengths.
    let raw = Some(line.clone());

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

