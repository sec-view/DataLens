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
  raw_max_chars: usize,
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
  // To avoid huge allocations for extremely long lines, only collect a limited prefix of each line.
  // We still scan/consume the full line to compute the next cursor offset correctly.
  let max_chars_needed = preview_max_chars.max(raw_max_chars).max(1);
  // Worst-case UTF-8 is 4 bytes per char; add a small slack so truncation markers work reliably.
  let collect_limit_bytes = max_chars_needed
    .saturating_add(1)
    .saturating_mul(4)
    .saturating_add(64);

  for _ in 0..page_size {
    let start_offset = offset;
    let (mut prefix, n_total_bytes, truncated) = read_line_prefix_bytes(&mut reader, collect_limit_bytes)?;
    if n_total_bytes == 0 {
      break;
    }
    offset += n_total_bytes;

    // Trim common line terminators from the collected prefix. Note this does NOT imply the full
    // line ended here (it may have been truncated by `collect_limit_bytes`).
    while matches!(prefix.last(), Some(b'\n' | b'\r')) {
      prefix.pop();
    }

    let line_prefix = String::from_utf8_lossy(&prefix).to_string();

    let preview = truncate_chars_force_ellipsis(&line_prefix, preview_max_chars, truncated);
    // For very large JSONL records, we MUST NOT ship full raw via IPC (can exceed caps / freeze UI).
    // Provide a truncated raw with an ellipsis marker; UI can decide whether to load full content.
    let raw = if raw_max_chars == 0 {
      None
    } else {
      Some(truncate_chars_force_ellipsis(&line_prefix, raw_max_chars, truncated))
    };

    records.push(Record {
      id: line_no,
      preview,
      raw,
      meta: Some(RecordMeta {
        line_no,
        byte_offset: start_offset,
        byte_len: n_total_bytes,
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

fn truncate_chars_force_ellipsis(s: &str, max: usize, force_ellipsis: bool) -> String {
  if max == 0 {
    return if force_ellipsis { "…".into() } else { String::new() };
  }
  let mut out = String::new();
  for (i, ch) in s.chars().enumerate() {
    if i >= max {
      out.push_str("…");
      break;
    }
    out.push(ch);
  }
  if force_ellipsis && !out.ends_with('…') {
    out.push('…');
  }
  out
}

fn read_line_prefix_bytes(
  reader: &mut BufReader<File>,
  collect_limit_bytes: usize,
) -> Result<(Vec<u8>, u64, bool), CoreError> {
  let mut out: Vec<u8> = Vec::new();
  let mut total: u64 = 0;
  let mut truncated = false;

  loop {
    let buf = reader.fill_buf()?;
    if buf.is_empty() {
      // EOF (no more bytes)
      break;
    }

    // Find '\n' in current buffer without allocating.
    let mut nl_pos: Option<usize> = None;
    for (i, b) in buf.iter().enumerate() {
      if *b == b'\n' {
        nl_pos = Some(i);
        break;
      }
    }

    let take = match nl_pos {
      Some(i) => i + 1, // include '\n'
      None => buf.len(),
    };

    if !truncated {
      let remaining = collect_limit_bytes.saturating_sub(out.len());
      let to_copy = remaining.min(take);
      out.extend_from_slice(&buf[..to_copy]);
      if to_copy < take {
        truncated = true;
      }
    }

    reader.consume(take);
    total = total.saturating_add(take as u64);

    if nl_pos.is_some() {
      break;
    }
  }

  Ok((out, total, truncated))
}

