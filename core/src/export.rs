use std::{
  collections::BTreeSet,
  fs::File,
  io::{BufRead, BufReader, BufWriter, Read, Seek, SeekFrom, Write},
  path::{Path, PathBuf},
};

use serde_json::{Map, Value};

use crate::{
  engine::CoreError,
  models::{ExportFormat, ExportRequest, FileFormat},
  models::ExportResult,
  tasks::TaskManager,
};

pub(crate) fn export(
  tasks: &TaskManager,
  session_path: PathBuf,
  session_format: FileFormat,
  request: ExportRequest,
  out_format: ExportFormat,
  output_path: &Path,
) -> Result<ExportResult, CoreError> {
  if let Some(parent) = output_path.parent() {
    std::fs::create_dir_all(parent)?;
  }
  let out_file = File::create(output_path)?;
  let mut writer = BufWriter::new(out_file);

  // Special: export a subtree (or its children) from the current JSON record.
  if let ExportRequest::JsonSubtree {
    meta,
    path,
    include_root,
    children,
  } = request
  {
    if session_format != FileFormat::Json {
      return Err(CoreError::UnsupportedFormat(session_format));
    }
    if matches!(out_format, ExportFormat::Csv) {
      return Err(CoreError::InvalidArg(
        "json_subtree export only supports json/jsonl output".into(),
      ));
    }

    // Stream export for huge records (no full JSON parse in memory).
    let written = crate::formats::export_json_subtree_stream(
      &session_path,
      meta.byte_offset,
      &path,
      include_root,
      &children,
      out_format,
      &mut writer,
    )?;
    writer.flush()?;
    return Ok(ExportResult {
      output_path: output_path.to_string_lossy().to_string(),
      records_written: written,
    });
  }

  // Common: selection-based export from file/session.
  let ids: Vec<u64> = match request {
    ExportRequest::Selection { record_ids } => record_ids,
    ExportRequest::SearchTask { task_id } => tasks
      .get_search_task_hit_ids(&task_id)
      .map_err(CoreError::Task)?,
    ExportRequest::JsonSubtree { .. } => unreachable!("handled above"),
  };

  let ids = normalize_ids(ids);
  if ids.is_empty() {
    return Ok(ExportResult {
      output_path: output_path.to_string_lossy().to_string(),
      records_written: 0,
    });
  }

  let written = match (session_format, out_format) {
    // Raw line export (backward compatible behavior):
    (FileFormat::Jsonl, ExportFormat::Jsonl) => export_lines_passthrough(&session_path, &ids, &mut writer)?,
    (FileFormat::Jsonl, ExportFormat::Csv) => export_lines_passthrough(&session_path, &ids, &mut writer)?,
    (FileFormat::Csv, ExportFormat::Csv) => export_lines_passthrough(&session_path, &ids, &mut writer)?,

    // Conversions:
    (FileFormat::Jsonl, ExportFormat::Json) => export_jsonl_to_json_array(&session_path, &ids, &mut writer)?,
    (FileFormat::Csv, ExportFormat::Jsonl) => export_csv_to_jsonl(&session_path, &ids, &mut writer)?,
    (FileFormat::Csv, ExportFormat::Json) => export_csv_to_json(&session_path, &ids, &mut writer)?,
    (FileFormat::Json, ExportFormat::Jsonl) => export_json_to_jsonl(&session_path, &ids, &mut writer)?,
    (FileFormat::Json, ExportFormat::Json) => export_json_to_json(&session_path, &ids, &mut writer)?,
    (FileFormat::Parquet, ExportFormat::Jsonl) => export_parquet_to_jsonl(&session_path, &ids, &mut writer)?,
    (FileFormat::Parquet, ExportFormat::Json) => export_parquet_to_json(&session_path, &ids, &mut writer)?,

    (fmt, _) => return Err(CoreError::UnsupportedFormat(fmt)),
  };

  writer.flush()?;
  Ok(ExportResult {
    output_path: output_path.to_string_lossy().to_string(),
    records_written: written,
  })
}

fn normalize_ids(ids: Vec<u64>) -> Vec<u64> {
  let mut set = BTreeSet::new();
  for id in ids {
    set.insert(id);
  }
  set.into_iter().collect()
}

fn export_lines_passthrough(
  path: &Path,
  ids: &[u64],
  writer: &mut BufWriter<File>,
) -> Result<u64, CoreError> {
  let mut wanted_idx = 0usize;
  let mut written = 0u64;

  let in_file = File::open(path)?;
  let mut reader = BufReader::new(in_file);

  let mut line_no = 0u64;
  loop {
    if wanted_idx >= ids.len() {
      break;
    }
    let mut buf = Vec::new();
    let n = reader.read_until(b'\n', &mut buf)?;
    if n == 0 {
      break;
    }

    if ids[wanted_idx] == line_no {
      normalize_line_ending(&mut buf);
      writer.write_all(&buf)?;
      written += 1;
      wanted_idx += 1;
    }
    line_no += 1;
  }
  Ok(written)
}

fn normalize_line_ending(buf: &mut Vec<u8>) {
  if buf.ends_with(b"\n") {
    // ok
  } else {
    buf.push(b'\n');
  }
  // Strip CR before LF if present.
  if buf.len() >= 2 && buf[buf.len() - 2] == b'\r' && buf[buf.len() - 1] == b'\n' {
    buf.remove(buf.len() - 2);
  }
}

fn export_jsonl_to_json_array(
  path: &Path,
  ids: &[u64],
  writer: &mut BufWriter<File>,
) -> Result<u64, CoreError> {
  let in_file = File::open(path)?;
  let mut reader = BufReader::new(in_file);

  writer.write_all(b"[")?;
  let mut wrote_any = false;

  let mut wanted_idx = 0usize;
  let mut line_no = 0u64;
  loop {
    if wanted_idx >= ids.len() {
      break;
    }
    let mut buf = Vec::new();
    let n = reader.read_until(b'\n', &mut buf)?;
    if n == 0 {
      break;
    }

    if ids[wanted_idx] == line_no {
      // Trim newline/CRLF, keep the JSON text bytes as-is.
      if buf.ends_with(b"\n") {
        buf.pop();
        if buf.ends_with(b"\r") {
          buf.pop();
        }
      }

      if wrote_any {
        writer.write_all(b",\n")?;
      } else {
        writer.write_all(b"\n")?;
        wrote_any = true;
      }
      writer.write_all(&buf)?;
      wanted_idx += 1;
    }
    line_no += 1;
  }

  if wrote_any {
    writer.write_all(b"\n]")?;
  } else {
    writer.write_all(b"]")?;
  }
  Ok(wanted_idx as u64)
}

// --- CSV -> JSON/JSONL ---

fn export_csv_to_jsonl(path: &Path, ids: &[u64], writer: &mut BufWriter<File>) -> Result<u64, CoreError> {
  let headers = read_csv_header(path).unwrap_or_default();
  let in_file = File::open(path)?;
  let mut reader = BufReader::new(in_file);

  let mut wanted_idx = 0usize;
  let mut line_no = 0u64;
  let mut written = 0u64;

  loop {
    if wanted_idx >= ids.len() {
      break;
    }
    let mut buf = Vec::new();
    let n = reader.read_until(b'\n', &mut buf)?;
    if n == 0 {
      break;
    }
    if ids[wanted_idx] != line_no {
      line_no += 1;
      continue;
    }

    // For csv->jsonl: skip header row (line 0) even if selected.
    if line_no == 0 {
      wanted_idx += 1;
      line_no += 1;
      continue;
    }

    // Trim newline & CRLF
    if buf.ends_with(b"\n") {
      buf.pop();
      if buf.ends_with(b"\r") {
        buf.pop();
      }
    }
    let line = String::from_utf8_lossy(&buf).to_string();
    let obj = csv_line_to_object(&headers, &line);
    let s = serde_json::to_string(&obj)
      .map_err(|e| CoreError::InvalidArg(format!("CSV 转 JSON 失败：{e}")))?;
    writer.write_all(s.as_bytes())?;
    writer.write_all(b"\n")?;
    written += 1;

    wanted_idx += 1;
    line_no += 1;
  }

  Ok(written)
}

fn export_csv_to_json(path: &Path, ids: &[u64], writer: &mut BufWriter<File>) -> Result<u64, CoreError> {
  let headers = read_csv_header(path).unwrap_or_default();
  let in_file = File::open(path)?;
  let mut reader = BufReader::new(in_file);

  writer.write_all(b"[")?;
  let mut wrote_any = false;

  let mut wanted_idx = 0usize;
  let mut line_no = 0u64;
  let mut written = 0u64;

  loop {
    if wanted_idx >= ids.len() {
      break;
    }
    let mut buf = Vec::new();
    let n = reader.read_until(b'\n', &mut buf)?;
    if n == 0 {
      break;
    }
    if ids[wanted_idx] != line_no {
      line_no += 1;
      continue;
    }

    if line_no == 0 {
      wanted_idx += 1;
      line_no += 1;
      continue;
    }

    if buf.ends_with(b"\n") {
      buf.pop();
      if buf.ends_with(b"\r") {
        buf.pop();
      }
    }
    let line = String::from_utf8_lossy(&buf).to_string();
    let obj = csv_line_to_object(&headers, &line);
    let s = serde_json::to_string(&obj)
      .map_err(|e| CoreError::InvalidArg(format!("CSV 转 JSON 失败：{e}")))?;

    if wrote_any {
      writer.write_all(b",\n")?;
    } else {
      writer.write_all(b"\n")?;
      wrote_any = true;
    }
    writer.write_all(s.as_bytes())?;
    written += 1;

    wanted_idx += 1;
    line_no += 1;
  }

  if wrote_any {
    writer.write_all(b"\n]")?;
  } else {
    writer.write_all(b"]")?;
  }
  Ok(written)
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
  for (i, h) in headers.iter_mut().enumerate() {
    if h.trim().is_empty() {
      *h = format!("col_{i}");
    }
  }
  if headers.is_empty() {
    headers.push("col_0".to_string());
  }
  Ok(headers)
}

fn parse_csv_line(line: &str) -> Vec<String> {
  let mut out: Vec<String> = Vec::new();
  let mut cur = String::new();
  let mut in_quotes = false;
  let mut chars = line.chars().peekable();

  while let Some(ch) = chars.next() {
    match ch {
      '"' => {
        if in_quotes && matches!(chars.peek(), Some('"')) {
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

fn csv_line_to_object(headers: &[String], line: &str) -> Value {
  let fields = parse_csv_line(line);
  let mut obj = Map::new();
  for (i, h) in headers.iter().enumerate() {
    let v = fields.get(i).cloned().unwrap_or_default();
    obj.insert(h.clone(), Value::String(v));
  }
  if fields.len() > headers.len() {
    obj.insert(
      "__extra__".to_string(),
      Value::Array(fields[headers.len()..].iter().cloned().map(Value::String).collect()),
    );
  }
  Value::Object(obj)
}

// --- JSON (.json) -> JSON/JSONL ---

fn export_json_to_jsonl(path: &Path, ids: &[u64], writer: &mut BufWriter<File>) -> Result<u64, CoreError> {
  export_json_stream(path, ids, ExportFormat::Jsonl, writer)
}

fn export_json_to_json(path: &Path, ids: &[u64], writer: &mut BufWriter<File>) -> Result<u64, CoreError> {
  export_json_stream(path, ids, ExportFormat::Json, writer)
}

fn export_json_stream(
  path: &Path,
  ids: &[u64],
  out_format: ExportFormat,
  writer: &mut BufWriter<File>,
) -> Result<u64, CoreError> {
  let mut f = File::open(path)?;
  f.seek(SeekFrom::Start(0))?;
  let mut reader = BufReader::with_capacity(1024 * 1024, f);

  let mut wanted_idx = 0usize;
  let mut cur_idx = 0u64;
  let mut written = 0u64;

  // Detect optional root array.
  skip_bom_and_ws(&mut reader)?;
  let mut in_array = false;
  if peek_byte(&mut reader)? == Some(b'[') {
    in_array = true;
    consume_byte(&mut reader)?;
    skip_ws_and_nul(&mut reader)?;
    if peek_byte(&mut reader)? == Some(b']') {
      consume_byte(&mut reader)?;
      return Ok(0);
    }
  }

  if matches!(out_format, ExportFormat::Json) {
    writer.write_all(b"[")?;
  }
  let mut wrote_any = false;

  loop {
    if wanted_idx >= ids.len() {
      break;
    }

    skip_ws_and_nul(&mut reader)?;

    // End of root array or file.
    match peek_byte(&mut reader)? {
      None => break,
      Some(b']') if in_array => {
        consume_byte(&mut reader)?;
        break;
      }
      Some(b',') if in_array => {
        consume_byte(&mut reader)?;
        continue;
      }
      _ => {}
    }

    let want_this = ids[wanted_idx] == cur_idx;
    if want_this {
      match out_format {
        ExportFormat::Jsonl => {
          scan_one_json_value(&mut reader, Some(writer))?;
          writer.write_all(b"\n")?;
          written += 1;
        }
        ExportFormat::Json => {
          if wrote_any {
            writer.write_all(b",\n")?;
          } else {
            writer.write_all(b"\n")?;
            wrote_any = true;
          }
          scan_one_json_value(&mut reader, Some(writer))?;
          written += 1;
        }
        ExportFormat::Csv => unreachable!("handled earlier"),
      }
      wanted_idx += 1;
    } else {
      // Skip the value without buffering it.
      scan_one_json_value(&mut reader, None)?;
    }

    cur_idx += 1;

    // After a value, tolerate comma / end bracket / ws.
    skip_ws_and_nul(&mut reader)?;
    if in_array {
      if peek_byte(&mut reader)? == Some(b',') {
        consume_byte(&mut reader)?;
      } else if peek_byte(&mut reader)? == Some(b']') {
        consume_byte(&mut reader)?;
        break;
      }
    } else {
      // single-root JSON value: only one record (id 0)
      break;
    }
  }

  if matches!(out_format, ExportFormat::Json) {
    if wrote_any {
      writer.write_all(b"\n]")?;
    } else {
      writer.write_all(b"]")?;
    }
  }
  Ok(written)
}

fn scan_one_json_value(
  reader: &mut BufReader<File>,
  mut out: Option<&mut BufWriter<File>>,
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

    if !started {
      if is_ignorable_head_byte(b) {
        continue;
      }
      started = true;
    }

    // IMPORTANT: If we hit a top-level comma delimiter, we must NOT include it in the value.
    if !in_string && depth == 0 && b == b',' {
      unread_one(reader)?;
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
        if nb == b',' || nb == b']' || nb.is_ascii_whitespace() || nb == 0 {
          break;
        }
      } else {
        break;
      }
    }
  }

  Ok(())
}

fn export_parquet_to_jsonl(path: &Path, ids: &[u64], writer: &mut BufWriter<File>) -> Result<u64, CoreError> {
  export_parquet(path, ids, ExportFormat::Jsonl, writer)
}

fn export_parquet_to_json(path: &Path, ids: &[u64], writer: &mut BufWriter<File>) -> Result<u64, CoreError> {
  export_parquet(path, ids, ExportFormat::Json, writer)
}

fn export_parquet(
  path: &Path,
  ids: &[u64],
  out_format: ExportFormat,
  writer: &mut BufWriter<File>,
) -> Result<u64, CoreError> {
  let path_str = path
    .to_str()
    .ok_or_else(|| CoreError::InvalidArg("invalid path encoding".into()))?;

  let conn = duckdb::Connection::open_in_memory()
    .map_err(|e| CoreError::InvalidArg(format!("DuckDB 初始化失败：{e}")))?;
  let _ = conn.execute_batch("LOAD parquet;");

  let mut stmt = conn
    .prepare("SELECT * FROM read_parquet(?) LIMIT 1 OFFSET ?")
    .map_err(|e| CoreError::InvalidArg(format!("DuckDB 准备语句失败：{e}")))?;

  if matches!(out_format, ExportFormat::Json) {
    writer.write_all(b"[")?;
  }
  let mut wrote_any = false;
  let mut written = 0u64;

  for row_idx in ids {
    let offset_i64 = i64::try_from(*row_idx)
      .map_err(|_| CoreError::InvalidArg(format!("invalid row index for parquet: {row_idx}")))?;

    let mut rows = stmt
      .query(duckdb::params![path_str, offset_i64])
      .map_err(|e| CoreError::InvalidArg(format!("Parquet 读取失败：{e}")))?;
    let Some(row) = rows
      .next()
      .map_err(|e| CoreError::InvalidArg(format!("Parquet 读取失败：{e}")))?
    else {
      // out of range -> skip
      continue;
    };

    let col_count = row.as_ref().column_count();
    let mut obj = Map::with_capacity(col_count);
    for i in 0..col_count {
      let key = row
        .as_ref()
        .column_name(i)
        .map(|s| s.to_string())
        .unwrap_or_else(|_| format!("col_{i}"));
      let v: duckdb::types::Value = row
        .get(i)
        .map_err(|e| CoreError::InvalidArg(format!("Parquet 读取失败：{e}")))?;
      obj.insert(key, duckdb_value_to_json(&v));
    }
    let value = Value::Object(obj);
    let line = serde_json::to_string(&value)
      .map_err(|e| CoreError::InvalidArg(format!("Parquet 行序列化失败：{e}")))?;

    match out_format {
      ExportFormat::Jsonl => {
        writer.write_all(line.as_bytes())?;
        writer.write_all(b"\n")?;
      }
      ExportFormat::Json => {
        if wrote_any {
          writer.write_all(b",\n")?;
        } else {
          writer.write_all(b"\n")?;
          wrote_any = true;
        }
        writer.write_all(line.as_bytes())?;
      }
      ExportFormat::Csv => unreachable!("handled earlier"),
    }
    written += 1;
  }

  if matches!(out_format, ExportFormat::Json) {
    if wrote_any {
      writer.write_all(b"\n]")?;
    } else {
      writer.write_all(b"]")?;
    }
  }
  Ok(written)
}

fn duckdb_value_to_json(v: &duckdb::types::Value) -> Value {
  use duckdb::types::Value as V;
  match v {
    V::Null => Value::Null,
    V::Boolean(b) => Value::Bool(*b),

    V::TinyInt(x) => Value::Number((*x as i64).into()),
    V::SmallInt(x) => Value::Number((*x as i64).into()),
    V::Int(x) => Value::Number((*x as i64).into()),
    V::BigInt(x) => Value::Number((*x).into()),

    V::UTinyInt(x) => Value::Number((*x as u64).into()),
    V::USmallInt(x) => Value::Number((*x as u64).into()),
    V::UInt(x) => Value::Number((*x as u64).into()),
    V::UBigInt(x) => Value::Number((*x).into()),

    V::Float(x) => serde_json::Number::from_f64(*x as f64)
      .map(Value::Number)
      .unwrap_or_else(|| Value::String(x.to_string())),
    V::Double(x) => serde_json::Number::from_f64(*x)
      .map(Value::Number)
      .unwrap_or_else(|| Value::String(x.to_string())),

    V::Text(s) => Value::String(s.clone()),

    other => Value::String(format!("{other:?}")),
  }
}

// --- json_subtree helpers ---

// --- tiny JSON scanner helpers (streaming) ---

fn skip_bom_and_ws(reader: &mut BufReader<File>) -> Result<(), CoreError> {
  let b = peek_n(reader, 3)?;
  if b.as_slice() == [0xEF, 0xBB, 0xBF] {
    for _ in 0..3 {
      consume_byte(reader)?;
    }
  }
  skip_ws_and_nul(reader)?;
  Ok(())
}

fn skip_ws_and_nul(reader: &mut BufReader<File>) -> Result<(), CoreError> {
  loop {
    match peek_byte(reader)? {
      Some(b) if is_ignorable_head_byte(b) => {
        consume_byte(reader)?;
      }
      _ => break,
    }
  }
  Ok(())
}

fn consume_byte(reader: &mut BufReader<File>) -> Result<u8, CoreError> {
  read_one(reader)?.ok_or_else(|| CoreError::InvalidArg("unexpected EOF".into()))
}

fn read_one(reader: &mut BufReader<File>) -> Result<Option<u8>, CoreError> {
  let mut buf = [0u8; 1];
  match reader.read(&mut buf)? {
    0 => Ok(None),
    _ => Ok(Some(buf[0])),
  }
}

fn unread_one(reader: &mut BufReader<File>) -> Result<(), CoreError> {
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
  b == 0 || b == b' ' || b == b'\n' || b == b'\r' || b == b'\t'
}

