use std::{
  fs::File,
  io::{BufRead, BufReader, Read, Seek, SeekFrom, Write},
  path::Path,
};

use crate::{
  cursor::Cursor,
  engine::CoreError,
  formats::LinesPageInternal,
  models::{
    ExportFormat, JsonChildItem, JsonChildrenPage, JsonNodeKind, JsonNodeSummary, JsonPathSegment,
    Record, RecordMeta, JsonChildItemOffset, JsonChildrenPageOffset, JsonNodeSummaryOffset,
  },
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

// --- Lazy JSON tree helpers (streaming) ---

/// List direct children under a JSON node selected by `path_segments`, starting from `cursor`.
///
/// This is designed for huge single-record JSON values where we cannot load/parse the full value
/// in the frontend. It scans the underlying bytes and only materializes small previews.
pub(crate) fn list_json_children_page(
  path: &Path,
  record_offset: u64,
  path_segments: &[JsonPathSegment],
  cursor: u64,
  limit: usize,
  preview_max_chars: usize,
) -> Result<JsonChildrenPage, CoreError> {
  let mut f = File::open(path)?;
  let file_len = f.metadata().ok().map(|m| m.len()).unwrap_or(0);
  if record_offset > file_len {
    return Err(CoreError::InvalidArg(format!(
      "offset {} beyond file len {}",
      record_offset, file_len
    )));
  }
  f.seek(SeekFrom::Start(record_offset))?;
  let mut reader = BufReader::with_capacity(1024 * 1024, f);
  let mut abs = record_offset;
  let total = file_len;
  let mut on_progress: Option<&mut dyn FnMut(u64, u64, &'static str)> = None;

  // Move reader to subtree start.
  seek_to_subtree(&mut reader, &mut abs, total, &mut on_progress, path_segments)?;
  skip_ws_and_nul(&mut reader, &mut abs, total, &mut on_progress)?;

  let first = peek_byte(&mut reader)?;
  let Some(first) = first else {
    return Ok(JsonChildrenPage {
      items: vec![],
      next_cursor: None,
      reached_end: true,
    });
  };

  match first {
    b'{' => list_object_children(&mut reader, &mut abs, total, &mut on_progress, cursor, limit, preview_max_chars),
    b'[' => list_array_children(&mut reader, &mut abs, total, &mut on_progress, cursor, limit, preview_max_chars),
    _ => Ok(JsonChildrenPage {
      items: vec![],
      next_cursor: None,
      reached_end: true,
    }),
  }
}

fn kind_from_first_byte(b: u8) -> JsonNodeKind {
  match b {
    b'{' => JsonNodeKind::Object,
    b'[' => JsonNodeKind::Array,
    b'"' => JsonNodeKind::String,
    b't' | b'f' => JsonNodeKind::Boolean,
    b'n' => JsonNodeKind::Null,
    b'-' | b'0'..=b'9' => JsonNodeKind::Number,
    _ => JsonNodeKind::Unknown,
  }
}

fn list_object_children(
  reader: &mut BufReader<File>,
  abs: &mut u64,
  total: u64,
  on_progress: &mut Option<&mut dyn FnMut(u64, u64, &'static str)>,
  cursor: u64,
  limit: usize,
  preview_max_chars: usize,
) -> Result<JsonChildrenPage, CoreError> {
  // consume '{'
  consume_byte(reader, abs, total, on_progress)?;
  skip_ws_and_nul(reader, abs, total, on_progress)?;
  if peek_byte(reader)? == Some(b'}') {
    consume_byte(reader, abs, total, on_progress)?;
    return Ok(JsonChildrenPage {
      items: vec![],
      next_cursor: None,
      reached_end: true,
    });
  }

  // Skip to cursor-th entry.
  let mut idx: u64 = 0;
  while idx < cursor {
    skip_ws_and_nul(reader, abs, total, on_progress)?;
    if peek_byte(reader)? == Some(b'}') {
      consume_byte(reader, abs, total, on_progress)?;
      return Ok(JsonChildrenPage {
        items: vec![],
        next_cursor: None,
        reached_end: true,
      });
    }
    // key
    let _k = read_json_string(reader, abs, total, on_progress)?;
    skip_ws_and_nul(reader, abs, total, on_progress)?;
    expect_byte(reader, abs, total, on_progress, b':')?;
    skip_ws_and_nul(reader, abs, total, on_progress)?;
    // value
    scan_one_json_value_with_stops(reader, abs, total, None, &[b',', b'}'], on_progress)?;
    skip_ws_and_nul(reader, abs, total, on_progress)?;
    match peek_byte(reader)? {
      Some(b',') => {
        consume_byte(reader, abs, total, on_progress)?;
        idx += 1;
        continue;
      }
      Some(b'}') => {
        consume_byte(reader, abs, total, on_progress)?;
        return Ok(JsonChildrenPage {
          items: vec![],
          next_cursor: None,
          reached_end: true,
        });
      }
      _ => {
        // tolerate weird separators; stop.
        return Ok(JsonChildrenPage {
          items: vec![],
          next_cursor: None,
          reached_end: true,
        });
      }
    }
  }

  // Collect up to limit entries.
  let mut out: Vec<JsonChildItem> = Vec::with_capacity(limit);
  let mut reached_end = false;

  for _ in 0..limit {
    skip_ws_and_nul(reader, abs, total, on_progress)?;
    match peek_byte(reader)? {
      Some(b'}') => {
        consume_byte(reader, abs, total, on_progress)?;
        reached_end = true;
        break;
      }
      None => {
        reached_end = true;
        break;
      }
      _ => {}
    }

    let key = read_json_string(reader, abs, total, on_progress)?;
    skip_ws_and_nul(reader, abs, total, on_progress)?;
    expect_byte(reader, abs, total, on_progress, b':')?;
    skip_ws_and_nul(reader, abs, total, on_progress)?;

    let first = peek_byte(reader)?.unwrap_or(b'?');
    let kind = kind_from_first_byte(first);
    let scanned = scan_one_json_value_with_stops(
      reader,
      abs,
      total,
      Some((preview_max_chars.max(64) * 4) as usize),
      &[b',', b'}'],
      on_progress,
    )?;
    let (preview, truncated) = preview_from_scan(scanned.captured, scanned.total_len_bytes, preview_max_chars);
    let preview = if truncated && !preview.ends_with('…') {
      format!("{preview}…")
    } else {
      preview
    };

    out.push(JsonChildItem {
      seg: JsonPathSegment::Key(key),
      kind,
      preview,
    });

    skip_ws_and_nul(reader, abs, total, on_progress)?;
    match peek_byte(reader)? {
      Some(b',') => {
        consume_byte(reader, abs, total, on_progress)?;
      }
      Some(b'}') => {
        consume_byte(reader, abs, total, on_progress)?;
        reached_end = true;
        break;
      }
      None => {
        reached_end = true;
        break;
      }
      _ => {}
    }
  }

  let next_cursor = if reached_end {
    None
  } else {
    Some(cursor.saturating_add(out.len() as u64))
  };

  Ok(JsonChildrenPage {
    items: out,
    next_cursor,
    reached_end,
  })
}

fn list_array_children(
  reader: &mut BufReader<File>,
  abs: &mut u64,
  total: u64,
  on_progress: &mut Option<&mut dyn FnMut(u64, u64, &'static str)>,
  cursor: u64,
  limit: usize,
  preview_max_chars: usize,
) -> Result<JsonChildrenPage, CoreError> {
  // consume '['
  consume_byte(reader, abs, total, on_progress)?;
  skip_ws_and_nul(reader, abs, total, on_progress)?;
  if peek_byte(reader)? == Some(b']') {
    consume_byte(reader, abs, total, on_progress)?;
    return Ok(JsonChildrenPage {
      items: vec![],
      next_cursor: None,
      reached_end: true,
    });
  }

  // Skip to cursor-th element.
  let mut idx: u64 = 0;
  while idx < cursor {
    skip_ws_and_nul(reader, abs, total, on_progress)?;
    if peek_byte(reader)? == Some(b']') {
      consume_byte(reader, abs, total, on_progress)?;
      return Ok(JsonChildrenPage {
        items: vec![],
        next_cursor: None,
        reached_end: true,
      });
    }
    scan_one_json_value_with_stops(reader, abs, total, None, &[b',', b']'], on_progress)?;
    skip_ws_and_nul(reader, abs, total, on_progress)?;
    match peek_byte(reader)? {
      Some(b',') => {
        consume_byte(reader, abs, total, on_progress)?;
        idx += 1;
      }
      Some(b']') => {
        consume_byte(reader, abs, total, on_progress)?;
        return Ok(JsonChildrenPage {
          items: vec![],
          next_cursor: None,
          reached_end: true,
        });
      }
      _ => {
        return Ok(JsonChildrenPage {
          items: vec![],
          next_cursor: None,
          reached_end: true,
        });
      }
    }
  }

  let mut out: Vec<JsonChildItem> = Vec::with_capacity(limit);
  let mut reached_end = false;
  let mut cur_idx = cursor;

  for _ in 0..limit {
    skip_ws_and_nul(reader, abs, total, on_progress)?;
    match peek_byte(reader)? {
      Some(b']') => {
        consume_byte(reader, abs, total, on_progress)?;
        reached_end = true;
        break;
      }
      None => {
        reached_end = true;
        break;
      }
      _ => {}
    }

    let first = peek_byte(reader)?.unwrap_or(b'?');
    let kind = kind_from_first_byte(first);
    let scanned = scan_one_json_value_with_stops(
      reader,
      abs,
      total,
      Some((preview_max_chars.max(64) * 4) as usize),
      &[b',', b']'],
      on_progress,
    )?;
    let (preview, truncated) = preview_from_scan(scanned.captured, scanned.total_len_bytes, preview_max_chars);
    let preview = if truncated && !preview.ends_with('…') {
      format!("{preview}…")
    } else {
      preview
    };

    out.push(JsonChildItem {
      seg: JsonPathSegment::Index(cur_idx),
      kind,
      preview,
    });
    cur_idx += 1;

    skip_ws_and_nul(reader, abs, total, on_progress)?;
    match peek_byte(reader)? {
      Some(b',') => {
        consume_byte(reader, abs, total, on_progress)?;
      }
      Some(b']') => {
        consume_byte(reader, abs, total, on_progress)?;
        reached_end = true;
        break;
      }
      None => {
        reached_end = true;
        break;
      }
      _ => {}
    }
  }

  let next_cursor = if reached_end {
    None
  } else {
    Some(cursor.saturating_add(out.len() as u64))
  };

  Ok(JsonChildrenPage {
    items: out,
    next_cursor,
    reached_end,
  })
}

fn preview_from_scan(captured: Vec<u8>, total_len_bytes: u64, preview_max_chars: usize) -> (String, bool) {
  let mut s = String::from_utf8_lossy(&captured).to_string();
  // Trim trailing NUL/whitespace for a cleaner preview.
  while s.ends_with('\0') || s.ends_with('\n') || s.ends_with('\r') {
    s.pop();
  }
  let truncated = (captured.len() as u64) < total_len_bytes;
  s = truncate_chars(&s, preview_max_chars);
  (s, truncated)
}

fn expect_byte(
  reader: &mut BufReader<File>,
  abs: &mut u64,
  total: u64,
  on_progress: &mut Option<&mut dyn FnMut(u64, u64, &'static str)>,
  want: u8,
) -> Result<(), CoreError> {
  let b = consume_byte(reader, abs, total, on_progress)?;
  if b != want {
    return Err(CoreError::InvalidArg(format!(
      "unexpected byte: got {} want {}",
      b as char, want as char
    )));
  }
  Ok(())
}

fn read_json_string(
  reader: &mut BufReader<File>,
  abs: &mut u64,
  total: u64,
  on_progress: &mut Option<&mut dyn FnMut(u64, u64, &'static str)>,
) -> Result<String, CoreError> {
  skip_ws_and_nul(reader, abs, total, on_progress)?;
  let first = peek_byte(reader)?.ok_or_else(|| CoreError::InvalidArg("unexpected EOF".into()))?;
  if first != b'"' {
    return Err(CoreError::InvalidArg("expected string".into()));
  }
  // Collect the full JSON string literal bytes (including quotes) and let serde_json decode it.
  let mut buf: Vec<u8> = Vec::with_capacity(64);
  let mut escape = false;
  loop {
    let b = consume_byte(reader, abs, total, on_progress)?;
    buf.push(b);
    if escape {
      escape = false;
      continue;
    }
    if b == b'\\' {
      escape = true;
      continue;
    }
    if b == b'"' && buf.len() > 1 {
      break;
    }
  }
  serde_json::from_slice::<String>(&buf)
    .map_err(|e| CoreError::InvalidArg(format!("invalid json string: {e}")))
}

/// Stream-export a JSON subtree (or selected direct children under it) without loading the full JSON
/// value into memory.
///
/// This replaces the previous `serde_json::from_str` based approach, and works for huge records.
pub(crate) fn export_json_subtree_stream(
  session_path: &Path,
  record_offset: u64,
  path: &[JsonPathSegment],
  include_root: bool,
  children: &[JsonPathSegment],
  out_format: ExportFormat,
  writer: &mut dyn Write,
) -> Result<u64, CoreError> {
  if matches!(out_format, ExportFormat::Csv) {
    return Err(CoreError::InvalidArg(
      "json_subtree export does not support csv output".into(),
    ));
  }

  let mut f = File::open(session_path)?;
  let file_len = f.metadata().ok().map(|m| m.len()).unwrap_or(0);
  if record_offset > file_len {
    return Err(CoreError::InvalidArg(format!(
      "offset {} beyond file len {}",
      record_offset, file_len
    )));
  }
  f.seek(SeekFrom::Start(record_offset))?;
  let mut reader = BufReader::with_capacity(1024 * 1024, f);
  let mut abs = record_offset;
  let total = file_len;
  let mut on_progress: Option<&mut dyn FnMut(u64, u64, &'static str)> = None;

  seek_to_subtree(&mut reader, &mut abs, total, &mut on_progress, path)?;
  skip_ws_and_nul(&mut reader, &mut abs, total, &mut on_progress)?;

  // If leaf or no children specified, treat as include_root.
  let include_root = include_root || children.is_empty();

  // UX tweak: exporting a single subtree as `.json` should output a single JSON value (not `[value]`).
  if include_root && matches!(out_format, ExportFormat::Json) && children.is_empty() {
    scan_one_json_value_to_writer(
      &mut reader,
      &mut abs,
      total,
      &[b',', b']', b'}'],
      &mut on_progress,
      Some(writer),
    )?;
    return Ok(1);
  }

  // Helper to emit a single value according to output format.
  let mut written: u64 = 0;
  let mut wrote_any = false;

  let begin_out = |w: &mut dyn Write| -> Result<(), CoreError> {
    if matches!(out_format, ExportFormat::Json) {
      w.write_all(b"[")?;
    }
    Ok(())
  };
  let end_out = |w: &mut dyn Write, wrote_any: bool| -> Result<(), CoreError> {
    if matches!(out_format, ExportFormat::Json) {
      if wrote_any {
        w.write_all(b"\n]")?;
      } else {
        w.write_all(b"]")?;
      }
    }
    Ok(())
  };
  let begin_item = |w: &mut dyn Write, wrote_any: bool| -> Result<(), CoreError> {
    match out_format {
      ExportFormat::Jsonl => Ok(()),
      ExportFormat::Json => {
        if wrote_any {
          w.write_all(b",\n")?;
        } else {
          w.write_all(b"\n")?;
        }
        Ok(())
      }
      ExportFormat::Csv => unreachable!(),
    }
  };
  let end_item = |w: &mut dyn Write| -> Result<(), CoreError> {
    if matches!(out_format, ExportFormat::Jsonl) {
      w.write_all(b"\n")?;
    }
    Ok(())
  };

  begin_out(writer)?;

  if include_root {
    begin_item(writer, wrote_any)?;
    scan_one_json_value_to_writer(&mut reader, &mut abs, total, &[b',', b']', b'}'], &mut on_progress, Some(writer))?;
    end_item(writer)?;
    wrote_any = true;
    written += 1;
    end_out(writer, wrote_any)?;
    return Ok(written);
  }

  // Export selected direct children under the subtree.
  skip_ws_and_nul(&mut reader, &mut abs, total, &mut on_progress)?;
  let first = peek_byte(&mut reader)?.ok_or_else(|| CoreError::InvalidArg("unexpected EOF".into()))?;

  // Build desired children sets.
  let mut want_keys: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
  let mut want_indices: std::collections::BTreeSet<u64> = std::collections::BTreeSet::new();
  for seg in children {
    match seg {
      JsonPathSegment::Key(k) => {
        want_keys.insert(k.clone());
      }
      JsonPathSegment::Index(i) => {
        want_indices.insert(*i);
      }
    }
  }

  match first {
    b'{' => {
      // consume '{'
      consume_byte(&mut reader, &mut abs, total, &mut on_progress)?;
      skip_ws_and_nul(&mut reader, &mut abs, total, &mut on_progress)?;
      if peek_byte(&mut reader)? == Some(b'}') {
        consume_byte(&mut reader, &mut abs, total, &mut on_progress)?;
      } else {
        loop {
          let key = read_json_string(&mut reader, &mut abs, total, &mut on_progress)?;
          skip_ws_and_nul(&mut reader, &mut abs, total, &mut on_progress)?;
          expect_byte(&mut reader, &mut abs, total, &mut on_progress, b':')?;
          skip_ws_and_nul(&mut reader, &mut abs, total, &mut on_progress)?;

          if want_keys.contains(&key) {
            begin_item(writer, wrote_any)?;
            scan_one_json_value_to_writer(&mut reader, &mut abs, total, &[b',', b'}'], &mut on_progress, Some(writer))?;
            end_item(writer)?;
            wrote_any = true;
            written += 1;
          } else {
            scan_one_json_value_to_writer(&mut reader, &mut abs, total, &[b',', b'}'], &mut on_progress, None)?;
          }

          skip_ws_and_nul(&mut reader, &mut abs, total, &mut on_progress)?;
          match peek_byte(&mut reader)? {
            Some(b',') => {
              consume_byte(&mut reader, &mut abs, total, &mut on_progress)?;
              skip_ws_and_nul(&mut reader, &mut abs, total, &mut on_progress)?;
              continue;
            }
            Some(b'}') => {
              consume_byte(&mut reader, &mut abs, total, &mut on_progress)?;
              break;
            }
            None => break,
            _ => break,
          }
        }
      }
    }
    b'[' => {
      consume_byte(&mut reader, &mut abs, total, &mut on_progress)?;
      skip_ws_and_nul(&mut reader, &mut abs, total, &mut on_progress)?;
      if peek_byte(&mut reader)? == Some(b']') {
        consume_byte(&mut reader, &mut abs, total, &mut on_progress)?;
      } else {
        let mut idx: u64 = 0;
        loop {
          skip_ws_and_nul(&mut reader, &mut abs, total, &mut on_progress)?;
          match peek_byte(&mut reader)? {
            Some(b']') => {
              consume_byte(&mut reader, &mut abs, total, &mut on_progress)?;
              break;
            }
            None => break,
            _ => {}
          }

          if want_indices.contains(&idx) {
            begin_item(writer, wrote_any)?;
            scan_one_json_value_to_writer(&mut reader, &mut abs, total, &[b',', b']'], &mut on_progress, Some(writer))?;
            end_item(writer)?;
            wrote_any = true;
            written += 1;
          } else {
            scan_one_json_value_to_writer(&mut reader, &mut abs, total, &[b',', b']'], &mut on_progress, None)?;
          }
          idx += 1;

          skip_ws_and_nul(&mut reader, &mut abs, total, &mut on_progress)?;
          match peek_byte(&mut reader)? {
            Some(b',') => {
              consume_byte(&mut reader, &mut abs, total, &mut on_progress)?;
              continue;
            }
            Some(b']') => {
              consume_byte(&mut reader, &mut abs, total, &mut on_progress)?;
              break;
            }
            None => break,
            _ => break,
          }
        }
      }
    }
    _ => {
      // Leaf: export as root.
      begin_item(writer, wrote_any)?;
      scan_one_json_value_to_writer(&mut reader, &mut abs, total, &[b',', b']', b'}'], &mut on_progress, Some(writer))?;
      end_item(writer)?;
      wrote_any = true;
      written += 1;
    }
  }

  end_out(writer, wrote_any)?;
  Ok(written)
}

/// Best-effort summary (kind + child count) for the selected subtree.
///
/// Counting can be expensive; we support caps to keep UI responsive.
pub(crate) fn json_node_summary(
  session_path: &Path,
  record_offset: u64,
  path_segments: &[JsonPathSegment],
  max_items: u64,
  max_scan_bytes: u64,
) -> Result<JsonNodeSummary, CoreError> {
  let mut f = File::open(session_path)?;
  let file_len = f.metadata().ok().map(|m| m.len()).unwrap_or(0);
  if record_offset > file_len {
    return Err(CoreError::InvalidArg(format!(
      "offset {} beyond file len {}",
      record_offset, file_len
    )));
  }
  f.seek(SeekFrom::Start(record_offset))?;
  let mut reader = BufReader::with_capacity(1024 * 1024, f);
  let mut abs = record_offset;
  let total = file_len;
  let mut on_progress: Option<&mut dyn FnMut(u64, u64, &'static str)> = None;

  seek_to_subtree(&mut reader, &mut abs, total, &mut on_progress, path_segments)?;
  skip_ws_and_nul(&mut reader, &mut abs, total, &mut on_progress)?;
  let first = peek_byte(&mut reader)?.unwrap_or(b'?');
  let kind = kind_from_first_byte(first);
  if kind != JsonNodeKind::Object && kind != JsonNodeKind::Array {
    return Ok(JsonNodeSummary {
      kind,
      child_count: None,
      complete: true,
    });
  }

  let start_abs = abs;
  let mut count: u64 = 0;
  let mut complete = true;

  if kind == JsonNodeKind::Object {
    consume_byte(&mut reader, &mut abs, total, &mut on_progress)?; // '{'
    loop {
      if abs.saturating_sub(start_abs) > max_scan_bytes || count >= max_items {
        complete = false;
        break;
      }
      skip_ws_and_nul(&mut reader, &mut abs, total, &mut on_progress)?;
      match peek_byte(&mut reader)? {
        Some(b'}') => {
          consume_byte(&mut reader, &mut abs, total, &mut on_progress)?;
          break;
        }
        None => break,
        _ => {}
      }
      // key
      skip_json_string_literal(&mut reader, &mut abs, total, &mut on_progress)?;
      skip_ws_and_nul(&mut reader, &mut abs, total, &mut on_progress)?;
      expect_byte(&mut reader, &mut abs, total, &mut on_progress, b':')?;
      skip_ws_and_nul(&mut reader, &mut abs, total, &mut on_progress)?;
      // value
      let _ = scan_one_json_value_with_stops(&mut reader, &mut abs, total, None, &[b',', b'}'], &mut on_progress)?;
      count += 1;
      skip_ws_and_nul(&mut reader, &mut abs, total, &mut on_progress)?;
      match peek_byte(&mut reader)? {
        Some(b',') => {
          consume_byte(&mut reader, &mut abs, total, &mut on_progress)?;
          continue;
        }
        Some(b'}') => {
          consume_byte(&mut reader, &mut abs, total, &mut on_progress)?;
          break;
        }
        None => break,
        _ => break,
      }
    }
  } else {
    consume_byte(&mut reader, &mut abs, total, &mut on_progress)?; // '['
    loop {
      if abs.saturating_sub(start_abs) > max_scan_bytes || count >= max_items {
        complete = false;
        break;
      }
      skip_ws_and_nul(&mut reader, &mut abs, total, &mut on_progress)?;
      match peek_byte(&mut reader)? {
        Some(b']') => {
          consume_byte(&mut reader, &mut abs, total, &mut on_progress)?;
          break;
        }
        None => break,
        _ => {}
      }
      let _ = scan_one_json_value_with_stops(&mut reader, &mut abs, total, None, &[b',', b']'], &mut on_progress)?;
      count += 1;
      skip_ws_and_nul(&mut reader, &mut abs, total, &mut on_progress)?;
      match peek_byte(&mut reader)? {
        Some(b',') => {
          consume_byte(&mut reader, &mut abs, total, &mut on_progress)?;
          continue;
        }
        Some(b']') => {
          consume_byte(&mut reader, &mut abs, total, &mut on_progress)?;
          break;
        }
        None => break,
        _ => break,
      }
    }
  }

  Ok(JsonNodeSummary {
    kind,
    child_count: Some(count),
    complete,
  })
}

/// Best-effort summary (kind + child count) for the node at `node_offset`.
pub(crate) fn json_node_summary_at_offset(
  session_path: &Path,
  node_offset: u64,
  max_items: u64,
  max_scan_bytes: u64,
) -> Result<JsonNodeSummaryOffset, CoreError> {
  let mut f = File::open(session_path)?;
  let file_len = f.metadata().ok().map(|m| m.len()).unwrap_or(0);
  if node_offset > file_len {
    return Err(CoreError::InvalidArg(format!(
      "offset {} beyond file len {}",
      node_offset, file_len
    )));
  }
  f.seek(SeekFrom::Start(node_offset))?;
  let mut reader = BufReader::with_capacity(1024 * 1024, f);
  let mut abs = node_offset;
  let total = file_len;
  let mut on_progress: Option<&mut dyn FnMut(u64, u64, &'static str)> = None;

  skip_ws_and_nul(&mut reader, &mut abs, total, &mut on_progress)?;
  let first = peek_byte(&mut reader)?.unwrap_or(b'?');
  let kind = kind_from_first_byte(first);
  if kind != JsonNodeKind::Object && kind != JsonNodeKind::Array {
    return Ok(JsonNodeSummaryOffset {
      kind,
      child_count: None,
      complete: true,
      node_offset,
    });
  }

  let start_abs = abs;
  let mut count: u64 = 0;
  let mut complete = true;

  if kind == JsonNodeKind::Object {
    consume_byte(&mut reader, &mut abs, total, &mut on_progress)?; // '{'
    loop {
      if abs.saturating_sub(start_abs) > max_scan_bytes || count >= max_items {
        complete = false;
        break;
      }
      skip_ws_and_nul(&mut reader, &mut abs, total, &mut on_progress)?;
      match peek_byte(&mut reader)? {
        Some(b'}') => {
          consume_byte(&mut reader, &mut abs, total, &mut on_progress)?;
          break;
        }
        None => break,
        _ => {}
      }
      // key
      skip_json_string_literal(&mut reader, &mut abs, total, &mut on_progress)?;
      skip_ws_and_nul(&mut reader, &mut abs, total, &mut on_progress)?;
      expect_byte(&mut reader, &mut abs, total, &mut on_progress, b':')?;
      skip_ws_and_nul(&mut reader, &mut abs, total, &mut on_progress)?;
      // value
      let _ = scan_one_json_value_with_stops(&mut reader, &mut abs, total, None, &[b',', b'}'], &mut on_progress)?;
      count += 1;
      skip_ws_and_nul(&mut reader, &mut abs, total, &mut on_progress)?;
      match peek_byte(&mut reader)? {
        Some(b',') => {
          consume_byte(&mut reader, &mut abs, total, &mut on_progress)?;
          continue;
        }
        Some(b'}') => {
          consume_byte(&mut reader, &mut abs, total, &mut on_progress)?;
          break;
        }
        None => break,
        _ => break,
      }
    }
  } else {
    consume_byte(&mut reader, &mut abs, total, &mut on_progress)?; // '['
    loop {
      if abs.saturating_sub(start_abs) > max_scan_bytes || count >= max_items {
        complete = false;
        break;
      }
      skip_ws_and_nul(&mut reader, &mut abs, total, &mut on_progress)?;
      match peek_byte(&mut reader)? {
        Some(b']') => {
          consume_byte(&mut reader, &mut abs, total, &mut on_progress)?;
          break;
        }
        None => break,
        _ => {}
      }
      let _ = scan_one_json_value_with_stops(&mut reader, &mut abs, total, None, &[b',', b']'], &mut on_progress)?;
      count += 1;
      skip_ws_and_nul(&mut reader, &mut abs, total, &mut on_progress)?;
      match peek_byte(&mut reader)? {
        Some(b',') => {
          consume_byte(&mut reader, &mut abs, total, &mut on_progress)?;
          continue;
        }
        Some(b']') => {
          consume_byte(&mut reader, &mut abs, total, &mut on_progress)?;
          break;
        }
        None => break,
        _ => break,
      }
    }
  }

  Ok(JsonNodeSummaryOffset {
    kind,
    child_count: Some(count),
    complete,
    node_offset,
  })
}

/// Offset-based JSON lazy tree (v2): list direct children at `node_offset`.
///
/// See `dh_core::models::JsonChildrenPageOffset` for cursor semantics.
pub(crate) fn list_json_children_page_at_offset(
  path: &Path,
  node_offset: u64,
  cursor_offset: Option<u64>,
  cursor_index: Option<u64>,
  limit: usize,
  preview_max_chars: usize,
) -> Result<JsonChildrenPageOffset, CoreError> {
  let mut f = File::open(path)?;
  let file_len = f.metadata().ok().map(|m| m.len()).unwrap_or(0);
  if node_offset > file_len {
    return Err(CoreError::InvalidArg(format!(
      "offset {} beyond file len {}",
      node_offset, file_len
    )));
  }
  f.seek(SeekFrom::Start(node_offset))?;
  let mut reader = BufReader::with_capacity(1024 * 1024, f);
  let mut abs = node_offset;
  let total = file_len;
  let mut on_progress: Option<&mut dyn FnMut(u64, u64, &'static str)> = None;

  skip_ws_and_nul(&mut reader, &mut abs, total, &mut on_progress)?;
  let first = peek_byte(&mut reader)?;
  let Some(first) = first else {
    return Ok(JsonChildrenPageOffset {
      items: vec![],
      next_cursor_offset: None,
      next_cursor_index: None,
      reached_end: true,
    });
  };

  match first {
    b'{' => list_object_children_at_offset(path, node_offset, cursor_offset, limit, preview_max_chars),
    b'[' => list_array_children_at_offset(
      path,
      node_offset,
      cursor_offset,
      cursor_index,
      limit,
      preview_max_chars,
    ),
    _ => Ok(JsonChildrenPageOffset {
      items: vec![],
      next_cursor_offset: None,
      next_cursor_index: None,
      reached_end: true,
    }),
  }
}

fn list_object_children_at_offset(
  path: &Path,
  node_offset: u64,
  cursor_offset: Option<u64>,
  limit: usize,
  preview_max_chars: usize,
) -> Result<JsonChildrenPageOffset, CoreError> {
  let mut f = File::open(path)?;
  let file_len = f.metadata().ok().map(|m| m.len()).unwrap_or(0);
  if node_offset > file_len {
    return Err(CoreError::InvalidArg(format!(
      "offset {} beyond file len {}",
      node_offset, file_len
    )));
  }
  f.seek(SeekFrom::Start(node_offset))?;
  let mut reader = BufReader::with_capacity(1024 * 1024, f);
  let mut abs = node_offset;
  let total = file_len;
  let mut on_progress: Option<&mut dyn FnMut(u64, u64, &'static str)> = None;

  skip_ws_and_nul(&mut reader, &mut abs, total, &mut on_progress)?;
  if peek_byte(&mut reader)? != Some(b'{') {
    return Ok(JsonChildrenPageOffset {
      items: vec![],
      next_cursor_offset: None,
      next_cursor_index: None,
      reached_end: true,
    });
  }

  consume_byte(&mut reader, &mut abs, total, &mut on_progress)?; // '{'
  skip_ws_and_nul(&mut reader, &mut abs, total, &mut on_progress)?;
  let first_entry_offset = abs;

  let want = cursor_offset.unwrap_or(first_entry_offset);
  if want > file_len {
    return Err(CoreError::InvalidArg(format!(
      "cursor_offset {} beyond file len {}",
      want, file_len
    )));
  }

  // Seek to cursor.
  let mut f2 = File::open(path)?;
  f2.seek(SeekFrom::Start(want))?;
  let mut reader = BufReader::with_capacity(1024 * 1024, f2);
  let mut abs = want;
  let total = file_len;
  let mut on_progress: Option<&mut dyn FnMut(u64, u64, &'static str)> = None;

  // Tolerate ws and leading comma.
  skip_ws_and_nul(&mut reader, &mut abs, total, &mut on_progress)?;
  if peek_byte(&mut reader)? == Some(b',') {
    consume_byte(&mut reader, &mut abs, total, &mut on_progress)?;
    skip_ws_and_nul(&mut reader, &mut abs, total, &mut on_progress)?;
  }

  let mut out: Vec<JsonChildItemOffset> = Vec::with_capacity(limit);
  let mut reached_end = false;
  let mut next_cursor_offset: Option<u64> = None;

  for _ in 0..limit {
    skip_ws_and_nul(&mut reader, &mut abs, total, &mut on_progress)?;
    match peek_byte(&mut reader)? {
      Some(b'}') => {
        reached_end = true;
        next_cursor_offset = None;
        break;
      }
      None => {
        reached_end = true;
        next_cursor_offset = None;
        break;
      }
      _ => {}
    }

    let key = read_json_string(&mut reader, &mut abs, total, &mut on_progress)?;
    skip_ws_and_nul(&mut reader, &mut abs, total, &mut on_progress)?;
    expect_byte(&mut reader, &mut abs, total, &mut on_progress, b':')?;
    skip_ws_and_nul(&mut reader, &mut abs, total, &mut on_progress)?;

    let value_offset = abs;
    let first = peek_byte(&mut reader)?.unwrap_or(b'?');
    let kind = kind_from_first_byte(first);
    let scanned = scan_one_json_value_with_stops(
      &mut reader,
      &mut abs,
      total,
      Some((preview_max_chars.max(64) * 4) as usize),
      &[b',', b'}'],
      &mut on_progress,
    )?;
    let (preview, truncated) = preview_from_scan(scanned.captured, scanned.total_len_bytes, preview_max_chars);
    let preview = if truncated && !preview.ends_with('…') {
      format!("{preview}…")
    } else {
      preview
    };

    out.push(JsonChildItemOffset {
      seg: JsonPathSegment::Key(key),
      kind,
      preview,
      value_offset,
    });

    skip_ws_and_nul(&mut reader, &mut abs, total, &mut on_progress)?;
    match peek_byte(&mut reader)? {
      Some(b',') => {
        consume_byte(&mut reader, &mut abs, total, &mut on_progress)?;
        skip_ws_and_nul(&mut reader, &mut abs, total, &mut on_progress)?;
        next_cursor_offset = Some(abs);
      }
      Some(b'}') => {
        reached_end = true;
        next_cursor_offset = None;
        break;
      }
      None => {
        reached_end = true;
        next_cursor_offset = None;
        break;
      }
      _ => {
        reached_end = true;
        next_cursor_offset = None;
        break;
      }
    }
  }

  if !reached_end && next_cursor_offset.is_none() {
    next_cursor_offset = Some(abs);
  }

  Ok(JsonChildrenPageOffset {
    items: out,
    next_cursor_offset: if reached_end { None } else { next_cursor_offset },
    next_cursor_index: None,
    reached_end,
  })
}

fn list_array_children_at_offset(
  path: &Path,
  node_offset: u64,
  cursor_offset: Option<u64>,
  cursor_index: Option<u64>,
  limit: usize,
  preview_max_chars: usize,
) -> Result<JsonChildrenPageOffset, CoreError> {
  let mut f = File::open(path)?;
  let file_len = f.metadata().ok().map(|m| m.len()).unwrap_or(0);
  if node_offset > file_len {
    return Err(CoreError::InvalidArg(format!(
      "offset {} beyond file len {}",
      node_offset, file_len
    )));
  }
  f.seek(SeekFrom::Start(node_offset))?;
  let mut reader = BufReader::with_capacity(1024 * 1024, f);
  let mut abs = node_offset;
  let total = file_len;
  let mut on_progress: Option<&mut dyn FnMut(u64, u64, &'static str)> = None;

  skip_ws_and_nul(&mut reader, &mut abs, total, &mut on_progress)?;
  if peek_byte(&mut reader)? != Some(b'[') {
    return Ok(JsonChildrenPageOffset {
      items: vec![],
      next_cursor_offset: None,
      next_cursor_index: None,
      reached_end: true,
    });
  }

  consume_byte(&mut reader, &mut abs, total, &mut on_progress)?; // '['
  skip_ws_and_nul(&mut reader, &mut abs, total, &mut on_progress)?;
  let first_elem_offset = abs;

  let want_off = cursor_offset.unwrap_or(first_elem_offset);
  if want_off > file_len {
    return Err(CoreError::InvalidArg(format!(
      "cursor_offset {} beyond file len {}",
      want_off, file_len
    )));
  }
  let mut cur_idx: u64 = cursor_index.unwrap_or(0);

  let mut f2 = File::open(path)?;
  f2.seek(SeekFrom::Start(want_off))?;
  let mut reader = BufReader::with_capacity(1024 * 1024, f2);
  let mut abs = want_off;
  let total = file_len;
  let mut on_progress: Option<&mut dyn FnMut(u64, u64, &'static str)> = None;

  // Tolerate ws and leading comma.
  skip_ws_and_nul(&mut reader, &mut abs, total, &mut on_progress)?;
  if peek_byte(&mut reader)? == Some(b',') {
    consume_byte(&mut reader, &mut abs, total, &mut on_progress)?;
    skip_ws_and_nul(&mut reader, &mut abs, total, &mut on_progress)?;
  }

  let mut out: Vec<JsonChildItemOffset> = Vec::with_capacity(limit);
  let mut reached_end = false;
  let mut next_cursor_offset: Option<u64> = None;
  let mut next_cursor_index: Option<u64> = None;

  for _ in 0..limit {
    skip_ws_and_nul(&mut reader, &mut abs, total, &mut on_progress)?;
    match peek_byte(&mut reader)? {
      Some(b']') => {
        reached_end = true;
        next_cursor_offset = None;
        next_cursor_index = None;
        break;
      }
      None => {
        reached_end = true;
        next_cursor_offset = None;
        next_cursor_index = None;
        break;
      }
      _ => {}
    }

    let value_offset = abs;
    let first = peek_byte(&mut reader)?.unwrap_or(b'?');
    let kind = kind_from_first_byte(first);
    let scanned = scan_one_json_value_with_stops(
      &mut reader,
      &mut abs,
      total,
      Some((preview_max_chars.max(64) * 4) as usize),
      &[b',', b']'],
      &mut on_progress,
    )?;
    let (preview, truncated) = preview_from_scan(scanned.captured, scanned.total_len_bytes, preview_max_chars);
    let preview = if truncated && !preview.ends_with('…') {
      format!("{preview}…")
    } else {
      preview
    };

    out.push(JsonChildItemOffset {
      seg: JsonPathSegment::Index(cur_idx),
      kind,
      preview,
      value_offset,
    });
    cur_idx += 1;

    skip_ws_and_nul(&mut reader, &mut abs, total, &mut on_progress)?;
    match peek_byte(&mut reader)? {
      Some(b',') => {
        consume_byte(&mut reader, &mut abs, total, &mut on_progress)?;
        skip_ws_and_nul(&mut reader, &mut abs, total, &mut on_progress)?;
        next_cursor_offset = Some(abs);
        next_cursor_index = Some(cur_idx);
      }
      Some(b']') => {
        reached_end = true;
        next_cursor_offset = None;
        next_cursor_index = None;
        break;
      }
      None => {
        reached_end = true;
        next_cursor_offset = None;
        next_cursor_index = None;
        break;
      }
      _ => {
        reached_end = true;
        next_cursor_offset = None;
        next_cursor_index = None;
        break;
      }
    }
  }

  if !reached_end && next_cursor_offset.is_none() {
    next_cursor_offset = Some(abs);
    next_cursor_index = Some(cur_idx);
  }

  Ok(JsonChildrenPageOffset {
    items: out,
    next_cursor_offset: if reached_end { None } else { next_cursor_offset },
    next_cursor_index: if reached_end { None } else { next_cursor_index },
    reached_end,
  })
}

fn skip_json_string_literal(
  reader: &mut BufReader<File>,
  abs: &mut u64,
  total: u64,
  on_progress: &mut Option<&mut dyn FnMut(u64, u64, &'static str)>,
) -> Result<(), CoreError> {
  skip_ws_and_nul(reader, abs, total, on_progress)?;
  let first = consume_byte(reader, abs, total, on_progress)?;
  if first != b'"' {
    return Err(CoreError::InvalidArg("expected string".into()));
  }
  let mut escape = false;
  loop {
    let b = consume_byte(reader, abs, total, on_progress)?;
    if escape {
      escape = false;
      continue;
    }
    if b == b'\\' {
      escape = true;
      continue;
    }
    if b == b'"' {
      break;
    }
  }
  Ok(())
}

fn scan_one_json_value_to_writer(
  reader: &mut BufReader<File>,
  abs: &mut u64,
  total: u64,
  stop_bytes: &[u8],
  on_progress: &mut Option<&mut dyn FnMut(u64, u64, &'static str)>,
  mut out: Option<&mut dyn Write>,
) -> Result<(), CoreError> {
  let mut in_string = false;
  let mut escape = false;
  let mut depth: i64 = 0;
  let mut started = false;

  loop {
    let b = match read_one(reader)? {
      None => break,
      Some(b) => b,
    };
    *abs += 1;
    maybe_emit_progress(*abs, total, "解析 JSON", on_progress);

    if !started {
      if is_ignorable_head_byte(b) {
        continue;
      }
      started = true;
    }

    // If we hit a top-level delimiter, do NOT include it.
    if !in_string && depth == 0 && stop_bytes.contains(&b) {
      unread_one(reader)?;
      *abs -= 1;
      break;
    }

    if let Some(w) = out.as_deref_mut() {
      w.write_all(&[b])?;
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
      _ => {}
    }

    // Primitive values end when depth==0 and next byte is delimiter/whitespace.
    if started && depth == 0 {
      if let Some(nb) = peek_byte(reader)? {
        if stop_bytes.contains(&nb) || nb.is_ascii_whitespace() || nb == 0 {
          break;
        }
      } else {
        break;
      }
    }
  }

  Ok(())
}

fn seek_to_subtree(
  reader: &mut BufReader<File>,
  abs: &mut u64,
  total: u64,
  on_progress: &mut Option<&mut dyn FnMut(u64, u64, &'static str)>,
  path: &[JsonPathSegment],
) -> Result<(), CoreError> {
  for seg in path {
    skip_ws_and_nul(reader, abs, total, on_progress)?;
    let b = peek_byte(reader)?.ok_or_else(|| CoreError::InvalidArg("unexpected EOF".into()))?;
    match (seg, b) {
      (JsonPathSegment::Key(want), b'{') => {
        // consume '{'
        consume_byte(reader, abs, total, on_progress)?;
        skip_ws_and_nul(reader, abs, total, on_progress)?;
        // empty object
        if peek_byte(reader)? == Some(b'}') {
          return Err(CoreError::InvalidArg("path not found (empty object)".into()));
        }
        loop {
          let key = read_json_string(reader, abs, total, on_progress)?;
          skip_ws_and_nul(reader, abs, total, on_progress)?;
          expect_byte(reader, abs, total, on_progress, b':')?;
          skip_ws_and_nul(reader, abs, total, on_progress)?;
          if &key == want {
            // positioned at value start for next segment
            break;
          } else {
            // skip value
            scan_one_json_value_with_stops(reader, abs, total, None, &[b',', b'}'], on_progress)?;
            skip_ws_and_nul(reader, abs, total, on_progress)?;
            match peek_byte(reader)? {
              Some(b',') => {
                consume_byte(reader, abs, total, on_progress)?;
                continue;
              }
              Some(b'}') => {
                return Err(CoreError::InvalidArg("path not found (key)".into()));
              }
              _ => return Err(CoreError::InvalidArg("path not found".into())),
            }
          }
        }
      }
      (JsonPathSegment::Index(want), b'[') => {
        consume_byte(reader, abs, total, on_progress)?;
        skip_ws_and_nul(reader, abs, total, on_progress)?;
        if peek_byte(reader)? == Some(b']') {
          return Err(CoreError::InvalidArg("path not found (empty array)".into()));
        }
        let mut idx: u64 = 0;
        loop {
          skip_ws_and_nul(reader, abs, total, on_progress)?;
          if peek_byte(reader)? == Some(b']') {
            return Err(CoreError::InvalidArg("path not found (index)".into()));
          }
          if idx == *want {
            // positioned at element start for next segment
            break;
          }
          scan_one_json_value_with_stops(reader, abs, total, None, &[b',', b']'], on_progress)?;
          skip_ws_and_nul(reader, abs, total, on_progress)?;
          match peek_byte(reader)? {
            Some(b',') => {
              consume_byte(reader, abs, total, on_progress)?;
              idx += 1;
              continue;
            }
            Some(b']') => return Err(CoreError::InvalidArg("path not found (index)".into())),
            _ => return Err(CoreError::InvalidArg("path not found".into())),
          }
        }
      }
      _ => {
        return Err(CoreError::InvalidArg("path does not match node kind".into()));
      }
    }
  }
  Ok(())
}

#[derive(Debug)]
struct ScannedAny {
  captured: Vec<u8>,
  total_len_bytes: u64,
}

fn scan_one_json_value_with_stops(
  reader: &mut BufReader<File>,
  abs: &mut u64,
  total: u64,
  capture_max_bytes: Option<usize>,
  stop_bytes: &[u8],
  on_progress: &mut Option<&mut dyn FnMut(u64, u64, &'static str)>,
) -> Result<ScannedAny, CoreError> {
  let mut captured = Vec::new();
  let mut total_len: u64 = 0;

  let mut in_string = false;
  let mut escape = false;
  let mut depth: i64 = 0;
  let mut started = false;

  loop {
    let b = match read_one(reader)? {
      None => break,
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
        // ignore head bytes before the actual value
        if let Some(max) = capture_max_bytes {
          if captured.len() > 0 {
            // keep behavior simple; allow captured whitespace at start if any
          } else if max > 0 {
            // no-op
          }
        }
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

    // If we hit a top-level delimiter, do NOT include it.
    if depth == 0 && stop_bytes.contains(&b) {
      unread_one(reader)?;
      *abs -= 1;
      total_len -= 1;
      break;
    }

    match b {
      b'"' => in_string = true,
      b'{' | b'[' => depth += 1,
      b'}' | b']' => {
        if depth > 0 {
          depth -= 1;
        }
      }
      _ => {}
    }

    // Primitive values end when depth==0 and next byte is delimiter/whitespace.
    if started && depth == 0 {
      if let Some(nb) = peek_byte(reader)? {
        if stop_bytes.contains(&nb) || nb.is_ascii_whitespace() || nb == 0 {
          break;
        }
      } else {
        break;
      }
    }
  }

  Ok(ScannedAny {
    captured,
    total_len_bytes: total_len,
  })
}

