use std::{
  collections::HashMap,
  fs::File,
  io::{BufRead, BufReader, Read, Seek, SeekFrom},
  path::PathBuf,
  sync::{
    atomic::{AtomicBool, AtomicU8, AtomicUsize, Ordering},
    Arc,
  },
  thread,
  time::{SystemTime, UNIX_EPOCH},
};

use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::{
  engine::CoreError,
  models::{FileFormat, Record, RecordMeta, RecordPage, SearchQuery, Task, TaskKind},
  search_match::PreparedSearch,
};

#[derive(Debug, Clone)]
pub struct TaskManagerOptions {
  pub max_concurrent_tasks: usize,
}

#[derive(Clone)]
pub struct TaskManager {
  opts: TaskManagerOptions,
  tasks: Arc<Mutex<HashMap<String, Arc<TaskState>>>>,
  running: Arc<AtomicUsize>,
}

#[derive(Debug)]
struct TaskState {
  id: String,
  kind: TaskKind,
  started_at_ms: i64,
  cancellable: bool,

  progress: AtomicU8,
  finished: AtomicBool,
  cancelled: AtomicBool,
  error: Mutex<Option<String>>,

  // For search_scan_all
  search_hits: Mutex<Vec<SearchHit>>,
  truncated: AtomicBool,
}

#[derive(Debug, Clone)]
struct SearchHit {
  line_no: u64,
  byte_offset: u64,
  byte_len: u64,
  preview: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct IndexCursor {
  idx: u64,
}

pub(crate) struct StartedTask {
  pub id: String,
}

impl TaskManager {
  pub fn new(opts: TaskManagerOptions) -> Self {
    Self {
      opts,
      tasks: Arc::new(Mutex::new(HashMap::new())),
      running: Arc::new(AtomicUsize::new(0)),
    }
  }

  pub(crate) fn start_search_scan_all(
    &self,
    path: PathBuf,
    format: FileFormat,
    query: SearchQuery,
    preview_max_chars: usize,
  ) -> Result<StartedTask, CoreError> {
    match format {
      FileFormat::Jsonl | FileFormat::Csv | FileFormat::Json | FileFormat::Parquet => {}
      other => return Err(CoreError::UnsupportedFormat(other)),
    }
    if query.text.is_empty() {
      return Err(CoreError::InvalidArg("query.text is empty".into()));
    }

    // Concurrency limit
    let now_running = self.running.load(Ordering::SeqCst);
    if now_running >= self.opts.max_concurrent_tasks {
      return Err(CoreError::Task(format!(
        "too many concurrent tasks (max {})",
        self.opts.max_concurrent_tasks
      )));
    }
    self.running.fetch_add(1, Ordering::SeqCst);

    let id = Uuid::new_v4().to_string();
    let state = Arc::new(TaskState {
      id: id.clone(),
      kind: TaskKind::SearchScanAll,
      started_at_ms: now_ms(),
      cancellable: true,
      progress: AtomicU8::new(0),
      finished: AtomicBool::new(false),
      cancelled: AtomicBool::new(false),
      error: Mutex::new(None),
      search_hits: Mutex::new(Vec::new()),
      truncated: AtomicBool::new(false),
    });
    self.tasks.lock().insert(id.clone(), state.clone());

    let tasks_map = self.tasks.clone();
    let running = self.running.clone();

    thread::spawn(move || {
      let res = run_search_scan_all(&state, path, format, query, preview_max_chars);
      if let Err(e) = res {
        *state.error.lock() = Some(e);
      }
      state.finished.store(true, Ordering::SeqCst);
      state.progress.store(100, Ordering::SeqCst);
      running.fetch_sub(1, Ordering::SeqCst);

      // Best-effort: drop finished tasks with no error & no hits? keep for now.
      let _ = tasks_map;
    });

    Ok(StartedTask { id })
  }

  pub fn get_task(&self, task_id: &str) -> Result<Task, String> {
    let t = self
      .tasks
      .lock()
      .get(task_id)
      .cloned()
      .ok_or_else(|| "unknown task".to_string())?;
    let err = t.error.lock().clone();
    Ok(Task {
      id: t.id.clone(),
      kind: t.kind.clone(),
      started_at_ms: t.started_at_ms,
      progress_0_100: t.progress.load(Ordering::SeqCst),
      cancellable: t.cancellable,
      finished: t.finished.load(Ordering::SeqCst),
      error: err,
    })
  }

  pub fn cancel_task(&self, task_id: &str) -> Result<(), String> {
    let t = self
      .tasks
      .lock()
      .get(task_id)
      .cloned()
      .ok_or_else(|| "unknown task".to_string())?;
    if !t.cancellable {
      return Err("task not cancellable".into());
    }
    t.cancelled.store(true, Ordering::SeqCst);
    Ok(())
  }

  pub fn search_task_hits_page(
    &self,
    task_id: &str,
    cursor: Option<&str>,
    page_size: usize,
  ) -> Result<RecordPage, String> {
    let t = self
      .tasks
      .lock()
      .get(task_id)
      .cloned()
      .ok_or_else(|| "unknown task".to_string())?;
    if t.kind != TaskKind::SearchScanAll {
      return Err("task is not search_scan_all".into());
    }

    let idx = decode_index_cursor(cursor).map_err(|e| e.to_string())?.idx as usize;
    let page_size = if page_size == 0 { 50 } else { page_size };

    let hits = t.search_hits.lock();
    let slice = hits.iter().skip(idx).take(page_size);

    let mut records = Vec::new();
    for h in slice {
      records.push(Record {
        id: h.line_no,
        preview: h.preview.clone(),
        raw: None,
        meta: Some(RecordMeta {
          line_no: h.line_no,
          byte_offset: h.byte_offset,
          byte_len: h.byte_len,
        }),
      });
    }

    let next_idx = idx + records.len();
    let reached_eof = next_idx >= hits.len();
    let next_cursor = if reached_eof {
      None
    } else {
      Some(encode_index_cursor(IndexCursor {
        idx: next_idx as u64,
      }))
    };

    Ok(RecordPage {
      records,
      next_cursor,
      reached_eof,
    })
  }

  pub(crate) fn get_search_task_hit_ids(&self, task_id: &str) -> Result<Vec<u64>, String> {
    let t = self
      .tasks
      .lock()
      .get(task_id)
      .cloned()
      .ok_or_else(|| "unknown task".to_string())?;
    if t.kind != TaskKind::SearchScanAll {
      return Err("task is not search_scan_all".into());
    }
    let hits = t.search_hits.lock();
    Ok(hits.iter().map(|h| h.line_no).collect())
  }
}

fn run_search_scan_all(
  state: &TaskState,
  path: PathBuf,
  format: FileFormat,
  query: SearchQuery,
  preview_max_chars: usize,
) -> Result<(), String> {
  match format {
    FileFormat::Jsonl | FileFormat::Csv => run_search_scan_all_lines(state, path, query, preview_max_chars),
    FileFormat::Json => run_search_scan_all_json_root_array(state, path, query, preview_max_chars),
    FileFormat::Parquet => run_search_scan_all_parquet(state, path, query, preview_max_chars),
    other => Err(format!("unsupported format for scan_all: {other:?}")),
  }
}

fn run_search_scan_all_lines(
  state: &TaskState,
  path: PathBuf,
  query: SearchQuery,
  preview_max_chars: usize,
) -> Result<(), String> {
  let mut file = File::open(&path).map_err(|e| e.to_string())?;
  let file_len = file.metadata().ok().map(|m| m.len()).unwrap_or(0);
  file.seek(SeekFrom::Start(0)).map_err(|e| e.to_string())?;
  let mut reader = BufReader::new(file);

  let prepared = PreparedSearch::new(&query).ok_or_else(|| "query.text is empty".to_string())?;

  let mut offset = 0u64;
  let mut line_no = 0u64;
  loop {
    if state.cancelled.load(Ordering::SeqCst) {
      state.finished.store(true, Ordering::SeqCst);
      return Ok(());
    }

    let start_offset = offset;
    let mut buf = Vec::new();
    let n = reader.read_until(b'\n', &mut buf).map_err(|e| e.to_string())?;
    if n == 0 {
      break;
    }
    offset += n as u64;

    if buf.ends_with(b"\n") {
      buf.pop();
      if buf.ends_with(b"\r") {
        buf.pop();
      }
    }
    let line = String::from_utf8_lossy(&buf).to_string();
    let hay = if query.case_sensitive {
      line.clone()
    } else {
      line.to_lowercase()
    };

    if prepared.matches_in_hay(&hay) {
      push_hit(state, &query, SearchHit {
        line_no,
        byte_offset: start_offset,
        byte_len: n as u64,
        preview: truncate_chars(&line, preview_max_chars),
      });
    }

    line_no += 1;
    if file_len > 0 {
      let p = ((offset as f64 / file_len as f64) * 100.0).floor() as i32;
      let p = p.clamp(0, 99) as u8;
      state.progress.store(p, Ordering::SeqCst);
    }
  }
  Ok(())
}

fn push_hit(state: &TaskState, query: &SearchQuery, hit: SearchHit) {
  let mut hits = state.search_hits.lock();
  if (hits.len() as u64) < query.max_hits {
    hits.push(hit);
  } else {
    state.truncated.store(true, Ordering::SeqCst);
  }
}

fn run_search_scan_all_json_root_array(
  state: &TaskState,
  path: PathBuf,
  query: SearchQuery,
  preview_max_chars: usize,
) -> Result<(), String> {
  const MAX_JSON_VALUE_BYTES: usize = 50 * 1024 * 1024; // keep consistent with get_record_raw safety cap

  let mut file = File::open(&path).map_err(|e| e.to_string())?;
  let file_len = file.metadata().ok().map(|m| m.len()).unwrap_or(0);
  file.seek(SeekFrom::Start(0)).map_err(|e| e.to_string())?;
  let mut reader = BufReader::with_capacity(1024 * 1024, file);

  let prepared = PreparedSearch::new(&query).ok_or_else(|| "query.text is empty".to_string())?;

  let mut abs: u64 = 0;
  // Skip BOM + whitespace
  skip_bom_and_ws(&mut reader, &mut abs).map_err(|e| e.to_string())?;
  // Enforce root array
  match peek_byte(&mut reader).map_err(|e| e.to_string())? {
    Some(b'[') => {
      consume_one(&mut reader, &mut abs).map_err(|e| e.to_string())?;
    }
    _ => {
      return Err("scan_all for .json only supports root array: file must start with '[' (after BOM/whitespace)".into());
    }
  }
  skip_ws_and_nul(&mut reader, &mut abs).map_err(|e| e.to_string())?;

  // Empty array => done
  if peek_byte(&mut reader).map_err(|e| e.to_string())? == Some(b']') {
    return Ok(());
  }

  let mut idx: u64 = 0;
  loop {
    if state.cancelled.load(Ordering::SeqCst) {
      state.finished.store(true, Ordering::SeqCst);
      return Ok(());
    }

    skip_ws_and_nul(&mut reader, &mut abs).map_err(|e| e.to_string())?;
    if peek_byte(&mut reader).map_err(|e| e.to_string())? == Some(b',') {
      consume_one(&mut reader, &mut abs).map_err(|e| e.to_string())?;
      continue;
    }
    match peek_byte(&mut reader).map_err(|e| e.to_string())? {
      Some(b']') | None => break,
      _ => {}
    }

    let start_offset = abs;
    let (value_bytes, value_len) =
      scan_one_json_value_full(&mut reader, &mut abs, MAX_JSON_VALUE_BYTES).map_err(|e| e.to_string())?;

    let text = String::from_utf8_lossy(&value_bytes).to_string();
    let hay = if query.case_sensitive {
      text.clone()
    } else {
      text.to_lowercase()
    };
    if prepared.matches_in_hay(&hay) {
      push_hit(
        state,
        &query,
        SearchHit {
          line_no: idx,
          byte_offset: start_offset,
          byte_len: value_len as u64,
          preview: truncate_chars(&text, preview_max_chars),
        },
      );
    }

    idx += 1;

    // Progress by bytes read (best-effort)
    if file_len > 0 {
      let p = ((abs as f64 / file_len as f64) * 100.0).floor() as i32;
      let p = p.clamp(0, 99) as u8;
      state.progress.store(p, Ordering::SeqCst);
    }

    // After value: whitespace, comma or closing bracket.
    skip_ws_and_nul(&mut reader, &mut abs).map_err(|e| e.to_string())?;
    match peek_byte(&mut reader).map_err(|e| e.to_string())? {
      Some(b',') => {
        consume_one(&mut reader, &mut abs).map_err(|e| e.to_string())?;
      }
      Some(b']') => break,
      None => break,
      _ => {}
    }
  }

  Ok(())
}

fn run_search_scan_all_parquet(
  state: &TaskState,
  path: PathBuf,
  query: SearchQuery,
  preview_max_chars: usize,
) -> Result<(), String> {
  let prepared = PreparedSearch::new(&query).ok_or_else(|| "query.text is empty".to_string())?;

  let path_str = path
    .to_str()
    .ok_or_else(|| "invalid path encoding".to_string())?
    .to_string();

  let conn = duckdb::Connection::open_in_memory()
    .map_err(|e| format!("DuckDB 初始化失败：{e}"))?;
  let _ = conn.execute_batch("LOAD parquet;");

  // Best-effort total row count for progress.
  let total_rows: u64 = conn
    .query_row(
      "SELECT count(*) FROM read_parquet(?)",
      duckdb::params![path_str.as_str()],
      |r| r.get::<usize, i64>(0),
    )
    .map(|n| n.max(0) as u64)
    .unwrap_or(0);

  const CHUNK: u64 = 2048;
  let mut offset: u64 = 0;

  loop {
    if state.cancelled.load(Ordering::SeqCst) {
      state.finished.store(true, Ordering::SeqCst);
      return Ok(());
    }
    if state.truncated.load(Ordering::SeqCst) {
      break;
    }

    let limit_i64 = i64::try_from(CHUNK).map_err(|_| "invalid parquet chunk size".to_string())?;
    let offset_i64 = i64::try_from(offset).map_err(|_| "invalid parquet offset".to_string())?;

    let mut stmt = conn
      .prepare("SELECT * FROM read_parquet(?) LIMIT ? OFFSET ?")
      .map_err(|e| format!("DuckDB 准备语句失败：{e}"))?;

    let mut rows = stmt
      .query(duckdb::params![path_str.as_str(), limit_i64, offset_i64])
      .map_err(|e| format!("Parquet 读取失败：{e}"))?;

    let mut got_any = false;
    let mut row_idx = offset;
    while let Some(row) = rows.next().map_err(|e| format!("Parquet 读取失败：{e}"))? {
      got_any = true;
      if state.cancelled.load(Ordering::SeqCst) {
        state.finished.store(true, Ordering::SeqCst);
        return Ok(());
      }

      let col_count = row.as_ref().column_count();
      let mut cols = Vec::with_capacity(col_count);
      for i in 0..col_count {
        let v: duckdb::types::Value = row
          .get(i)
          .map_err(|e| format!("Parquet 读取失败：{e}"))?;
        cols.push(sanitize_cell(&value_to_string(&v)));
      }
      let line = cols.join("\t");
      let hay = if query.case_sensitive {
        line.clone()
      } else {
        line.to_lowercase()
      };
      if prepared.matches_in_hay(&hay) {
        push_hit(
          state,
          &query,
          SearchHit {
            line_no: row_idx,
            byte_offset: row_idx, // not a real byte offset; kept for backwards-compat meta shape
            byte_len: 0,
            preview: truncate_chars(&line, preview_max_chars),
          },
        );
      }

      row_idx += 1;
      if total_rows > 0 {
        let p = (((row_idx.min(total_rows)) as f64 / total_rows as f64) * 100.0).floor() as i32;
        let p = p.clamp(0, 99) as u8;
        state.progress.store(p, Ordering::SeqCst);
      }
      if state.truncated.load(Ordering::SeqCst) {
        break;
      }
    }

    if !got_any {
      break;
    }
    offset += CHUNK;
  }

  Ok(())
}

// ---------------- JSON scanning helpers (root-array only) ----------------

fn peek_byte(reader: &mut BufReader<File>) -> Result<Option<u8>, std::io::Error> {
  let buf = reader.fill_buf()?;
  if buf.is_empty() {
    Ok(None)
  } else {
    Ok(Some(buf[0]))
  }
}

fn consume_one(reader: &mut BufReader<File>, abs: &mut u64) -> Result<u8, std::io::Error> {
  let mut buf = [0u8; 1];
  let n = reader.read(&mut buf)?;
  if n == 0 {
    return Err(std::io::Error::new(std::io::ErrorKind::UnexpectedEof, "unexpected EOF"));
  }
  *abs += 1;
  Ok(buf[0])
}

fn skip_bom_and_ws(reader: &mut BufReader<File>, abs: &mut u64) -> Result<(), std::io::Error> {
  // UTF-8 BOM: EF BB BF
  let buf = reader.fill_buf()?;
  if buf.len() >= 3 && buf[0] == 0xEF && buf[1] == 0xBB && buf[2] == 0xBF {
    reader.consume(3);
    *abs += 3;
  }
  skip_ws_and_nul(reader, abs)?;
  Ok(())
}

fn skip_ws_and_nul(reader: &mut BufReader<File>, abs: &mut u64) -> Result<(), std::io::Error> {
  loop {
    match peek_byte(reader)? {
      Some(b) if b == 0 || b.is_ascii_whitespace() => {
        consume_one(reader, abs)?;
      }
      _ => break,
    }
  }
  Ok(())
}

fn scan_one_json_value_full(
  reader: &mut BufReader<File>,
  abs: &mut u64,
  max_bytes: usize,
) -> Result<(Vec<u8>, usize), std::io::Error> {
  let mut out: Vec<u8> = Vec::new();
  let mut total_len: usize = 0;

  let mut in_string = false;
  let mut escape = false;
  let mut depth: i64 = 0;
  let mut started = false;

  loop {
    let b = match peek_byte(reader)? {
      None => {
        if !started {
          return Err(std::io::Error::new(std::io::ErrorKind::UnexpectedEof, "EOF before value"));
        }
        break;
      }
      Some(_) => consume_one(reader, abs)?,
    };
    total_len += 1;
    if total_len > max_bytes {
      return Err(std::io::Error::new(
        std::io::ErrorKind::InvalidData,
        format!("json value too large: {} bytes (max {})", total_len, max_bytes),
      ));
    }
    out.push(b);

    if !started {
      if b == 0 || b.is_ascii_whitespace() {
        // allow whitespace before value (should have been skipped, but be tolerant)
        out.pop();
        total_len -= 1;
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
          // delimiter ends value; unread by stepping back one byte
          reader.seek(SeekFrom::Current(-1))?;
          *abs -= 1;
          out.pop();
          total_len -= 1;
          break;
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

  Ok((out, total_len))
}

fn sanitize_cell(s: &str) -> String {
  s.replace(&['\n', '\r', '\t'][..], " ")
}

fn value_to_string(v: &duckdb::types::Value) -> String {
  use duckdb::types::Value;
  match v {
    Value::Null => "null".into(),
    Value::Boolean(b) => b.to_string(),
    Value::TinyInt(x) => x.to_string(),
    Value::SmallInt(x) => x.to_string(),
    Value::Int(x) => x.to_string(),
    Value::BigInt(x) => x.to_string(),
    Value::HugeInt(x) => x.to_string(),
    Value::UTinyInt(x) => x.to_string(),
    Value::USmallInt(x) => x.to_string(),
    Value::UInt(x) => x.to_string(),
    Value::UBigInt(x) => x.to_string(),
    Value::Float(x) => x.to_string(),
    Value::Double(x) => x.to_string(),
    Value::Decimal(d) => d.to_string(),
    Value::Timestamp(unit, v) => format!("timestamp({unit:?},{v})"),
    Value::Text(s) => s.clone(),
    Value::Blob(b) => {
      use base64::Engine as _;
      let encoded = base64::engine::general_purpose::STANDARD.encode(b);
      format!("blob(base64:{encoded})")
    }
    Value::Date32(days) => format!("date32({days})"),
    Value::Time64(unit, v) => format!("time64({unit:?},{v})"),
    Value::Interval { months, days, nanos } => format!("interval({months}m,{days}d,{nanos}n)"),
    Value::List(xs) | Value::Array(xs) => {
      let inner = xs.iter().map(value_to_string).collect::<Vec<_>>().join(", ");
      format!("[{inner}]")
    }
    Value::Enum(s) => s.clone(),
    Value::Struct(map) => {
      let inner = map
        .iter()
        .map(|(k, v)| format!("{k}: {}", value_to_string(v)))
        .collect::<Vec<_>>()
        .join(", ");
      format!("{{{inner}}}")
    }
    Value::Map(map) => {
      let inner = map
        .iter()
        .map(|(k, v)| format!("{}: {}", value_to_string(k), value_to_string(v)))
        .collect::<Vec<_>>()
        .join(", ");
      format!("{{{inner}}}")
    }
    Value::Union(v) => value_to_string(v),
  }
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

fn now_ms() -> i64 {
  SystemTime::now()
    .duration_since(UNIX_EPOCH)
    .unwrap_or_default()
    .as_millis() as i64
}

fn encode_index_cursor(c: IndexCursor) -> String {
  use base64::Engine as _;
  let json = serde_json::to_vec(&c).expect("index cursor serialize");
  base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(json)
}

fn decode_index_cursor(token: Option<&str>) -> Result<IndexCursor, CoreError> {
  match token {
    None => Ok(IndexCursor { idx: 0 }),
    Some(t) if t.is_empty() => Ok(IndexCursor { idx: 0 }),
    Some(t) => {
      use base64::Engine as _;
      let bytes = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(t)
        .map_err(|e| CoreError::BadCursor(e.to_string()))?;
      serde_json::from_slice(&bytes).map_err(|e| CoreError::BadCursor(e.to_string()))
    }
  }
}

