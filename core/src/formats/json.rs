use std::{
  fs::File,
  io::{BufRead, BufReader, Read, Seek, SeekFrom},
  path::Path,
};

use crate::{
  cursor::Cursor,
  engine::CoreError,
  formats::LinesPageInternal,
  models::{Record, RecordMeta},
};

/// Read the full JSON value starting at (or after) `offset`.
///
/// This is used for the UI "详情" view when `Record.raw` was truncated.
pub(crate) fn read_json_value_at_offset(
  path: &Path,
  offset: u64,
  max_bytes: u64,
) -> Result<String, CoreError> {
  let mut f = File::open(path)?;
  let file_len = f.metadata().ok().map(|m| m.len()).unwrap_or(0);
  if offset > file_len {
    return Err(CoreError::InvalidArg(format!(
      "offset {} beyond file len {}",
      offset, file_len
    )));
  }
  f.seek(SeekFrom::Start(offset))?;
  let mut reader = BufReader::with_capacity(1024 * 1024, f);

  let mut abs = offset;
  let total = file_len;
  let mut on_progress: Option<&mut dyn FnMut(u64, u64, &'static str)> = None;

  let mut out: Vec<u8> = Vec::new();
  let mut total_len: u64 = 0;

  let mut in_string = false;
  let mut escape = false;
  let mut depth: i64 = 0;
  let mut started = false;

  loop {
    let b = match read_one(&mut reader)? {
      None => {
        if !started {
          return Err(CoreError::InvalidArg("unexpected EOF at offset".into()));
        }
        break;
      }
      Some(b) => b,
    };
    abs += 1;
    total_len += 1;
    maybe_emit_progress(abs, total, "解析 JSON", &mut on_progress);

    if !started {
      if is_ignorable_head_byte(b) {
        continue;
      }
      started = true;
    }

    if total_len > max_bytes {
      return Err(CoreError::InvalidArg(format!(
        "json value too large: {} bytes (max {})",
        total_len, max_bytes
      )));
    }
    out.push(b);

    if in_string {
      if escape {
        escape = false;
        continue;
      }
      if b == b'\\' {
        escape = true;
        continue;
      }
      if b == b'"' {
        in_string = false;
      }
      continue;
    }

    match b {
      b'"' => in_string = true,
      b'{' | b'[' => depth += 1,
      b'}' | b']' => {
        if depth > 0 {
          depth -= 1;
        }
      }
      b',' => {
        if depth == 0 {
          // Comma delim ends the current value; push back and do NOT include comma.
          unread_one(&mut reader)?;
          out.pop();
          break;
        }
      }
      _ => {}
    }

    // Primitive values end when depth==0 and next byte is delimiter/whitespace.
    if started && depth == 0 {
      if let Some(nb) = peek_byte(&mut reader)? {
        if nb == b',' || nb == b']' || nb.is_ascii_whitespace() || nb == 0 {
          break;
        }
      } else {
        break;
      }
    }
  }

  Ok(String::from_utf8_lossy(&out).to_string())
}

/// Read a page from a `.json` file.
///
/// Performance notes:
/// - The previous implementation loaded the full file and deserialized the full JSON document, which
///   can be very slow / memory heavy for large JSON arrays.
/// - This implementation is **streaming** for the common case of `[...]` (root array): it scans
///   JSON items by tracking nesting depth and string escapes, and only materializes **one page**
///   of (possibly-truncated) raw text for preview.
/// - Cursor uses `offset` as the next element's byte offset (fast seek), and `line` as the record id.
pub(crate) fn read_json_page(
  path: &Path,
  cursor: Cursor,
  page_size: usize,
  preview_max_chars: usize,
  raw_max_chars: usize,
) -> Result<(LinesPageInternal, Option<Cursor>), CoreError> {
  read_json_page_with_progress(path, cursor, page_size, preview_max_chars, raw_max_chars, None)
}

pub(crate) fn read_json_page_with_progress(
  path: &Path,
  cursor: Cursor,
  page_size: usize,
  preview_max_chars: usize,
  raw_max_chars: usize,
  mut on_progress: Option<&mut dyn FnMut(u64, u64, &'static str)>,
) -> Result<(LinesPageInternal, Option<Cursor>), CoreError> {
  let mut file = File::open(path)?;
  let total = file.metadata().ok().map(|m| m.len()).unwrap_or(0);
  if cursor.offset > total {
    return Err(CoreError::BadCursor(format!(
      "offset {} beyond file len {}",
      cursor.offset, total
    )));
  }
  file.seek(SeekFrom::Start(cursor.offset))?;
  let mut reader = BufReader::with_capacity(1024 * 1024, file);

  let mut abs = cursor.offset;
  let mut next_id = cursor.line;

  // If we're at file start (offset 0), try to skip BOM/whitespace and an optional root `[` so we can
  // page a root-array without deserializing the entire document.
  if cursor.offset == 0 {
    skip_bom_and_ws(&mut reader, &mut abs, total, &mut on_progress)?;
    // If the next non-ws is '[', consume it and proceed scanning elements inside the array.
    if peek_byte(&mut reader)? == Some(b'[') {
      consume_byte(&mut reader, &mut abs, total, &mut on_progress)?; // '['
    }
    // skip whitespace after '['
    skip_ws_and_nul(&mut reader, &mut abs, total, &mut on_progress)?;
    // empty array => EOF
    if peek_byte(&mut reader)? == Some(b']') {
      consume_byte(&mut reader, &mut abs, total, &mut on_progress)?; // ']'
      return Ok((LinesPageInternal { records: vec![], reached_eof: true }, None));
    }
  } else {
    // If resuming from a cursor offset, be tolerant: skip whitespace and a leading comma.
    skip_ws_and_nul(&mut reader, &mut abs, total, &mut on_progress)?;
    if peek_byte(&mut reader)? == Some(b',') {
      consume_byte(&mut reader, &mut abs, total, &mut on_progress)?;
      skip_ws_and_nul(&mut reader, &mut abs, total, &mut on_progress)?;
    }
  }

  // Skip items until reaching cursor.line (needed when cursor.offset is 0 but line != 0; kept for
  // backward compatibility with older cursors).
  if cursor.offset == 0 && cursor.line > 0 {
    for _ in 0..cursor.line {
      if scan_one_json_value(&mut reader, &mut abs, total, None, 0, &mut on_progress)?.is_none() {
        return Ok((LinesPageInternal { records: vec![], reached_eof: true }, None));
      }
      skip_ws_and_nul(&mut reader, &mut abs, total, &mut on_progress)?;
      match peek_byte(&mut reader)? {
        Some(b',') => {
          consume_byte(&mut reader, &mut abs, total, &mut on_progress)?;
          skip_ws_and_nul(&mut reader, &mut abs, total, &mut on_progress)?;
        }
        Some(b']') | None => break,
        _ => {}
      }
    }
    next_id = cursor.line;
  }

  let mut records = Vec::with_capacity(page_size);
  let mut reached_eof = false;
  let mut next_cursor: Option<Cursor> = None;

  for _ in 0..page_size {
    skip_ws_and_nul(&mut reader, &mut abs, total, &mut on_progress)?;
    let start_offset = abs;

    // End of root array (or file end)
    match peek_byte(&mut reader)? {
      None => {
        reached_eof = true;
        next_cursor = None;
        break;
      }
      Some(b']') => {
        consume_byte(&mut reader, &mut abs, total, &mut on_progress)?;
        reached_eof = true;
        next_cursor = None;
        break;
      }
      Some(b',') => {
        // tolerate stray comma
        consume_byte(&mut reader, &mut abs, total, &mut on_progress)?;
        continue;
      }
      _ => {}
    }

    // Capture up to raw_max_chars chars (approx by bytes) for preview/raw.
    let capture_max_bytes = (raw_max_chars.max(preview_max_chars) * 4).max(1024) as usize;
    let scanned = scan_one_json_value(
      &mut reader,
      &mut abs,
      total,
      Some(capture_max_bytes),
      raw_max_chars,
      &mut on_progress,
    )?;
    let Some(scanned) = scanned else {
      reached_eof = true;
      next_cursor = None;
      break;
    };

    let raw_full = String::from_utf8_lossy(&scanned.captured).to_string();
    let raw = Some(truncate_chars(&raw_full, raw_max_chars));
    let preview = truncate_chars(&raw_full, preview_max_chars);

    records.push(Record {
      id: next_id,
      preview,
      raw,
      meta: Some(RecordMeta {
        line_no: next_id,
        byte_offset: start_offset,
        byte_len: scanned.total_len_bytes,
      }),
    });
    next_id += 1;

    // After a value, we may see whitespace, comma, or closing bracket.
    skip_ws_and_nul(&mut reader, &mut abs, total, &mut on_progress)?;
    match peek_byte(&mut reader)? {
      Some(b',') => {
        consume_byte(&mut reader, &mut abs, total, &mut on_progress)?;
        // next element starts after comma + ws
        let mut tmp_abs = abs;
        skip_ws_and_nul(&mut reader, &mut tmp_abs, total, &mut on_progress)?;
        next_cursor = Some(Cursor {
          offset: tmp_abs,
          line: next_id,
        });
        // keep abs in sync with tmp_abs (we actually consumed ws in the reader)
        abs = tmp_abs;
      }
      Some(b']') => {
        // next cursor none; but still reached eof after consuming ']'
        consume_byte(&mut reader, &mut abs, total, &mut on_progress)?;
        reached_eof = true;
        next_cursor = None;
        break;
      }
      None => {
        reached_eof = true;
        next_cursor = None;
        break;
      }
      _ => {
        // For non-array / weird separators: we still attempt to continue by setting next cursor to current abs.
        next_cursor = Some(Cursor { offset: abs, line: next_id });
      }
    }
  }

  if records.is_empty() {
    reached_eof = true;
    next_cursor = None;
  } else if next_cursor.is_none() && !reached_eof {
    next_cursor = Some(Cursor { offset: abs, line: next_id });
  }

  Ok((LinesPageInternal { records, reached_eof }, next_cursor))
}

#[derive(Debug)]
struct ScannedValue {
  captured: Vec<u8>,
  total_len_bytes: u64,
}

/// Scan one JSON value from the current reader position.
///
/// - Returns `None` if EOF before any non-ws bytes.
/// - Tracks JSON nesting depth and string escaping to find the end of the value.
/// - `capture_max_bytes`: capture up to N bytes for preview/raw. If None, capture nothing.
fn scan_one_json_value(
  reader: &mut BufReader<File>,
  abs: &mut u64,
  total: u64,
  capture_max_bytes: Option<usize>,
  _raw_max_chars: usize,
  on_progress: &mut Option<&mut dyn FnMut(u64, u64, &'static str)>,
) -> Result<Option<ScannedValue>, CoreError> {
  let mut captured = Vec::new();
  let mut total_len: u64 = 0;

  let mut in_string = false;
  let mut escape = false;
  let mut depth: i64 = 0;
  let mut started = false;

  loop {
    let b = match read_one(reader)? {
      None => {
        if !started {
          return Ok(None);
        }
        break;
      }
      Some(b) => b,
    };
    *abs += 1;
    total_len += 1;
    maybe_emit_progress(*abs, total, "解析 JSON", on_progress);

    if let Some(max) = capture_max_bytes {
      if captured.len() < max {
        captured.push(b);
      }
    }

    if !started {
      if is_ignorable_head_byte(b) {
        continue;
      }
      started = true;
    }

    if in_string {
      if escape {
        escape = false;
        continue;
      }
      if b == b'\\' {
        escape = true;
        continue;
      }
      if b == b'"' {
        in_string = false;
      }
      continue;
    }

    match b {
      b'"' => in_string = true,
      b'{' | b'[' => depth += 1,
      b'}' | b']' => {
        if depth > 0 {
          depth -= 1;
        }
      }
      b',' => {
        if depth == 0 {
          // Comma delim ends the current value; push back.
          unread_one(reader)?;
          *abs -= 1;
          total_len -= 1;
          break;
        }
      }
      _ => {}
    }

    // Primitive values (number/true/false/null) end when depth==0 and next byte is delimiter/whitespace.
    if started && depth == 0 {
      // Peek to decide if we should stop (without consuming).
      if let Some(nb) = peek_byte(reader)? {
        if nb == b',' || nb == b']' || nb.is_ascii_whitespace() || nb == 0 {
          break;
        }
      } else {
        break;
      }
    }
  }

  Ok(Some(ScannedValue {
    captured,
    total_len_bytes: total_len,
  }))
}

fn maybe_emit_progress(
  done: u64,
  total: u64,
  stage: &'static str,
  on_progress: &mut Option<&mut dyn FnMut(u64, u64, &'static str)>,
) {
  if let Some(cb) = on_progress.as_deref_mut() {
    // throttle: emit roughly every 256KiB, and always when done==total
    cb(done, total, stage);
  }
}

fn skip_bom_and_ws(
  reader: &mut BufReader<File>,
  abs: &mut u64,
  total: u64,
  on_progress: &mut Option<&mut dyn FnMut(u64, u64, &'static str)>,
) -> Result<(), CoreError> {
  // UTF-8 BOM: EF BB BF
  let b1 = peek_n(reader, 3)?;
  if b1.as_slice() == [0xEF, 0xBB, 0xBF] {
    for _ in 0..3 {
      consume_byte(reader, abs, total, on_progress)?;
    }
  }
  skip_ws_and_nul(reader, abs, total, on_progress)?;
  Ok(())
}

fn skip_ws_and_nul(
  reader: &mut BufReader<File>,
  abs: &mut u64,
  total: u64,
  on_progress: &mut Option<&mut dyn FnMut(u64, u64, &'static str)>,
) -> Result<(), CoreError> {
  loop {
    match peek_byte(reader)? {
      Some(b) if is_ignorable_head_byte(b) => {
        consume_byte(reader, abs, total, on_progress)?;
      }
      _ => break,
    }
  }
  Ok(())
}

fn consume_byte(
  reader: &mut BufReader<File>,
  abs: &mut u64,
  total: u64,
  on_progress: &mut Option<&mut dyn FnMut(u64, u64, &'static str)>,
) -> Result<u8, CoreError> {
  let b = read_one(reader)?.ok_or_else(|| CoreError::InvalidArg("unexpected EOF".into()))?;
  *abs += 1;
  maybe_emit_progress(*abs, total, "读取 JSON", on_progress);
  Ok(b)
}

fn read_one(reader: &mut BufReader<File>) -> Result<Option<u8>, CoreError> {
  let mut buf = [0u8; 1];
  match reader.read(&mut buf)? {
    0 => Ok(None),
    _ => Ok(Some(buf[0])),
  }
}

fn unread_one(reader: &mut BufReader<File>) -> Result<(), CoreError> {
  // BufReader provides `fill_buf`/`consume`, but not unconsume. We can use `Seek` to step back by 1
  // on the underlying file, then clear the buffer by re-creating the reader would be expensive.
  // Instead, leverage `std::io::Seek` on BufReader itself.
  reader.seek(SeekFrom::Current(-1))?;
  Ok(())
}

fn peek_byte(reader: &mut BufReader<File>) -> Result<Option<u8>, CoreError> {
  let buf = reader.fill_buf()?;
  if buf.is_empty() {
    Ok(None)
  } else {
    Ok(Some(buf[0]))
  }
}

fn peek_n(reader: &mut BufReader<File>, n: usize) -> Result<Vec<u8>, CoreError> {
  let buf = reader.fill_buf()?;
  Ok(buf.iter().take(n).copied().collect())
}

fn is_ignorable_head_byte(b: u8) -> bool {
  // For head we allow regular ASCII whitespace and NUL padding.
  b == 0 || b == b' ' || b == b'\n' || b == b'\r' || b == b'\t'
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

