use std::{
  collections::HashMap,
  io::{Read, Seek, SeekFrom},
  path::{Path, PathBuf},
  sync::Arc,
  time::{SystemTime, UNIX_EPOCH},
};

use parking_lot::Mutex;
use thiserror::Error;
use uuid::Uuid;

use crate::{
  cursor::{decode_cursor, encode_cursor},
  export as export_impl,
  formats,
  models::{
    ExportFormat, ExportRequest, ExportResult, FileFormat, RecordMeta, RecordPage, SearchMode,
    SearchQuery, SearchResult, SessionInfo, StatsResult, Task, TaskInfo, TaskKind, JsonChildrenPage,
    JsonPathSegment, JsonNodeSummary, JsonChildrenPageOffset, JsonNodeSummaryOffset,
  },
  storage::{Storage, StorageOptions},
  tasks::{TaskManager, TaskManagerOptions},
};

#[derive(Debug, Error)]
pub enum CoreError {
  #[error("io error: {0}")]
  Io(#[from] std::io::Error),
  #[error("unsupported format: {0:?}")]
  UnsupportedFormat(FileFormat),
  #[error("unknown session: {0}")]
  UnknownSession(String),
  #[error("bad cursor token: {0}")]
  BadCursor(String),
  #[error("invalid argument: {0}")]
  InvalidArg(String),
  #[error("storage error: {0}")]
  Storage(String),
  #[error("task error: {0}")]
  Task(String),
}

#[derive(Debug, Clone)]
pub struct CoreOptions {
  pub default_page_size: usize,
  pub preview_max_chars: usize,
  pub raw_max_chars: usize,
  pub max_concurrent_tasks: usize,
  pub storage: StorageOptions,
}

impl Default for CoreOptions {
  fn default() -> Self {
    Self {
      default_page_size: 10,
      preview_max_chars: 300,
      raw_max_chars: 40_000,
      max_concurrent_tasks: 2,
      storage: StorageOptions::default(),
    }
  }
}

#[derive(Debug, Clone)]
struct SessionState {
  info: SessionInfo,
  format: FileFormat,
  last_page: Option<crate::models::RecordPage>,
}

#[derive(Clone)]
pub struct CoreEngine {
  options: CoreOptions,
  sessions: Arc<Mutex<HashMap<String, SessionState>>>,
  tasks: TaskManager,
  storage: Storage,
}

impl CoreEngine {
  pub fn new(options: CoreOptions) -> Result<Self, CoreError> {
    let storage = Storage::new(options.storage.clone()).map_err(|e| CoreError::Storage(e))?;
    let tasks = TaskManager::new(TaskManagerOptions {
      max_concurrent_tasks: options.max_concurrent_tasks,
    });
    Ok(Self {
      options,
      sessions: Arc::new(Mutex::new(HashMap::new())),
      tasks,
      storage,
    })
  }

  /// IPC API: open_file(path) -> { session, first_page }
  pub fn open_file(&self, path: impl AsRef<Path>) -> Result<(SessionInfo, RecordPage), CoreError> {
    self.open_file_with_progress(path, |_| {})
  }

  /// Like `open_file`, but reports progress (best-effort) for large / slow formats.
  ///
  /// The callback receives a coarse `pct_0_100` (0..=100). For formats where we can track bytes
  /// read (notably `.json` root arrays), it will update smoothly; otherwise it may jump.
  pub fn open_file_with_progress(
    &self,
    path: impl AsRef<Path>,
    mut on_progress_pct: impl FnMut(u8),
  ) -> Result<(SessionInfo, RecordPage), CoreError> {
    let path = path.as_ref().to_path_buf();
    let format = formats::detect_format(&path);
    match format {
      FileFormat::Jsonl | FileFormat::Csv | FileFormat::Json | FileFormat::Parquet => {}
      _ => return Err(CoreError::UnsupportedFormat(format)),
    }

    on_progress_pct(0);

    let session_id = Uuid::new_v4().to_string();
    let created_at_ms = now_ms();
    let info = SessionInfo {
      session_id: session_id.clone(),
      path: path.to_string_lossy().to_string(),
      format: format.clone(),
      created_at_ms,
    };

    // Persist recent
    let _ = self.storage.touch_recent(&info.path, None);

    // first page from cursor = 0
    let first_page = if format == FileFormat::Json {
      // Track progress by bytes for large JSON (best-effort).
      let total = std::fs::metadata(&path).ok().map(|m| m.len()).unwrap_or(0);
      let mut last_pct: u8 = 0;
      let (page, next) = crate::formats::read_json_page_with_progress(
        &path,
        crate::cursor::Cursor { offset: 0, line: 0 },
        self.options.default_page_size,
        self.options.preview_max_chars,
        self.options.raw_max_chars,
        Some(&mut |done, total_bytes, _stage| {
          let total_bytes = if total_bytes == 0 { total } else { total_bytes };
          if total_bytes == 0 {
            return;
          }
          let pct = ((done.saturating_mul(100)) / total_bytes).min(100) as u8;
          if pct != last_pct {
            last_pct = pct;
            on_progress_pct(pct);
          }
        }),
      )?;
      let next_cursor = next.map(encode_cursor);
      RecordPage {
        records: page.records,
        next_cursor,
        reached_eof: page.reached_eof,
      }
    } else {
      self.read_page(&path, format.clone(), None, self.options.default_page_size)?
    };

    let state = SessionState {
      info: info.clone(),
      format,
      last_page: Some(first_page.clone()),
    };
    self.sessions.lock().insert(session_id, state);
    on_progress_pct(100);
    Ok((info, first_page))
  }

  /// IPC API: next_page(session_id, cursor, page_size) -> RecordPage
  pub fn next_page(
    &self,
    session_id: &str,
    cursor: Option<&str>,
    page_size: usize,
  ) -> Result<RecordPage, CoreError> {
    let (path, format) = {
      let sessions = self.sessions.lock();
      let s = sessions
        .get(session_id)
        .ok_or_else(|| CoreError::UnknownSession(session_id.to_string()))?;
      (PathBuf::from(&s.info.path), s.format.clone())
    };
    let page = self.read_page(&path, format, cursor, page_size)?;
    if let Some(s) = self.sessions.lock().get_mut(session_id) {
      s.last_page = Some(page.clone());
    }
    Ok(page)
  }

  /// IPC API: search(session_id, query, mode) -> SearchResult
  ///
  /// - current_page: runs synchronously over last returned page (open_file/next_page)
  /// - scan_all: starts a cancellable background task and returns task info
  pub fn search(&self, session_id: &str, query: SearchQuery) -> Result<SearchResult, CoreError> {
    let (path, format, last_page) = {
      let sessions = self.sessions.lock();
      let s = sessions
        .get(session_id)
        .ok_or_else(|| CoreError::UnknownSession(session_id.to_string()))?;
      (
        PathBuf::from(&s.info.path),
        s.format.clone(),
        s.last_page.clone(),
      )
    };

    match query.mode {
      SearchMode::CurrentPage => {
        let lp = last_page.ok_or_else(|| CoreError::InvalidArg("no page cached".into()))?;
        Ok(formats::search_current_page(&lp, &query))
      }
      SearchMode::ScanAll => {
        let task = self
          .tasks
          .start_search_scan_all(path, format, query, self.options.preview_max_chars)?;
        Ok(SearchResult {
          mode: SearchMode::ScanAll,
          hits: vec![],
          task: Some(TaskInfo {
            id: task.id.clone(),
            kind: TaskKind::SearchScanAll,
            cancellable: true,
          }),
          truncated: false,
        })
      }
      SearchMode::Indexed => Err(CoreError::InvalidArg(
        "indexed search not implemented (M4)".into(),
      )),
    }
  }

  /// Poll a background task status.
  pub fn get_task(&self, task_id: &str) -> Result<Task, CoreError> {
    self.tasks.get_task(task_id).map_err(CoreError::Task)
  }

  pub fn cancel_task(&self, task_id: &str) -> Result<(), CoreError> {
    self.tasks.cancel_task(task_id).map_err(CoreError::Task)
  }

  /// Fetch accumulated hits from a scan_all search task, in pages.
  pub fn search_task_hits_page(
    &self,
    task_id: &str,
    cursor: Option<&str>,
    page_size: usize,
  ) -> Result<crate::models::RecordPage, CoreError> {
    self.tasks
      .search_task_hits_page(task_id, cursor, page_size)
      .map_err(CoreError::Task)
  }

  /// IPC API: export(session_id, selection, format, output_path) -> ExportResult
  pub fn export(
    &self,
    session_id: &str,
    request: ExportRequest,
    format: ExportFormat,
    output_path: impl AsRef<Path>,
  ) -> Result<ExportResult, CoreError> {
    let (path, file_format) = {
      let sessions = self.sessions.lock();
      let s = sessions
        .get(session_id)
        .ok_or_else(|| CoreError::UnknownSession(session_id.to_string()))?;
      (PathBuf::from(&s.info.path), s.format.clone())
    };
    export_impl::export(&self.tasks, path, file_format, request, format, output_path.as_ref())
  }

  /// IPC API: json_list_children(session_id, meta, path, cursor, limit) -> JsonChildrenPage
  ///
  /// Designed for huge single-record JSON values: list direct children under a selected subtree
  /// without materializing the full JSON string.
  pub fn json_list_children(
    &self,
    session_id: &str,
    meta: RecordMeta,
    path: Vec<JsonPathSegment>,
    cursor: Option<u64>,
    limit: usize,
  ) -> Result<JsonChildrenPage, CoreError> {
    let (path_buf, format) = {
      let sessions = self.sessions.lock();
      let s = sessions
        .get(session_id)
        .ok_or_else(|| CoreError::UnknownSession(session_id.to_string()))?;
      (PathBuf::from(&s.info.path), s.format.clone())
    };
    if format != FileFormat::Json {
      return Err(CoreError::UnsupportedFormat(format));
    }
    let cursor = cursor.unwrap_or(0);
    let limit = if limit == 0 { 50 } else { limit };
    crate::formats::list_json_children_page(
      &path_buf,
      meta.byte_offset,
      &path,
      cursor,
      limit,
      self.options.preview_max_chars,
    )
  }

  /// IPC API: json_node_summary(session_id, meta, path) -> JsonNodeSummary
  ///
  /// Returns node kind and (best-effort) child count. Counting may stop early due to caps.
  pub fn json_node_summary(
    &self,
    session_id: &str,
    meta: RecordMeta,
    path: Vec<JsonPathSegment>,
    max_items: Option<u64>,
    max_scan_bytes: Option<u64>,
  ) -> Result<JsonNodeSummary, CoreError> {
    let (path_buf, format) = {
      let sessions = self.sessions.lock();
      let s = sessions
        .get(session_id)
        .ok_or_else(|| CoreError::UnknownSession(session_id.to_string()))?;
      (PathBuf::from(&s.info.path), s.format.clone())
    };
    if format != FileFormat::Json {
      return Err(CoreError::UnsupportedFormat(format));
    }
    let max_items = max_items.unwrap_or(200_000);
    let max_scan_bytes = max_scan_bytes.unwrap_or(64 * 1024 * 1024);
    crate::formats::json_node_summary(&path_buf, meta.byte_offset, &path, max_items, max_scan_bytes)
  }

  /// IPC API (v2): json_list_children_at_offset(session_id, meta, node_offset, cursor_offset, limit)
  ///
  /// This is a faster variant for huge records: the frontend navigates by absolute byte offsets
  /// returned by the backend, so expanding deep nodes does not rescan the path from record start.
  pub fn json_list_children_at_offset(
    &self,
    session_id: &str,
    meta: RecordMeta,
    node_offset: u64,
    cursor_offset: Option<u64>,
    cursor_index: Option<u64>,
    limit: usize,
  ) -> Result<JsonChildrenPageOffset, CoreError> {
    let (path_buf, format) = {
      let sessions = self.sessions.lock();
      let s = sessions
        .get(session_id)
        .ok_or_else(|| CoreError::UnknownSession(session_id.to_string()))?;
      (PathBuf::from(&s.info.path), s.format.clone())
    };
    // Allow JSONL records to reuse the same "parse one JSON value at offset" streaming tree.
    if format != FileFormat::Json && format != FileFormat::Jsonl {
      return Err(CoreError::UnsupportedFormat(format));
    }
    // Basic safety: node_offset must be >= record_offset (we only support offsets within the record).
    if node_offset < meta.byte_offset {
      return Err(CoreError::InvalidArg(format!(
        "node_offset {} is before record_offset {}",
        node_offset, meta.byte_offset
      )));
    }
    let limit = if limit == 0 { 50 } else { limit };
    crate::formats::list_json_children_page_at_offset(
      &path_buf,
      node_offset,
      cursor_offset,
      cursor_index,
      limit,
      self.options.preview_max_chars,
    )
  }

  /// IPC API (v2): json_node_summary_at_offset(session_id, meta, node_offset)
  pub fn json_node_summary_at_offset(
    &self,
    session_id: &str,
    meta: RecordMeta,
    node_offset: u64,
    max_items: Option<u64>,
    max_scan_bytes: Option<u64>,
  ) -> Result<JsonNodeSummaryOffset, CoreError> {
    let (path_buf, format) = {
      let sessions = self.sessions.lock();
      let s = sessions
        .get(session_id)
        .ok_or_else(|| CoreError::UnknownSession(session_id.to_string()))?;
      (PathBuf::from(&s.info.path), s.format.clone())
    };
    // Allow JSONL records to reuse the same "parse one JSON value at offset" streaming tree.
    if format != FileFormat::Json && format != FileFormat::Jsonl {
      return Err(CoreError::UnsupportedFormat(format));
    }
    if node_offset < meta.byte_offset {
      return Err(CoreError::InvalidArg(format!(
        "node_offset {} is before record_offset {}",
        node_offset, meta.byte_offset
      )));
    }
    let max_items = max_items.unwrap_or(200_000);
    let max_scan_bytes = max_scan_bytes.unwrap_or(64 * 1024 * 1024);
    crate::formats::json_node_summary_at_offset(&path_buf, node_offset, max_items, max_scan_bytes)
  }

  /// Reserved for M3.
  pub fn get_stats(&self, _session_id: &str) -> Result<StatsResult, CoreError> {
    Ok(StatsResult {
      message: "not implemented (M3)".into(),
    })
  }

  pub fn storage(&self) -> &Storage {
    &self.storage
  }

  fn read_page(
    &self,
    path: &Path,
    format: FileFormat,
    cursor: Option<&str>,
    page_size: usize,
  ) -> Result<RecordPage, CoreError> {
    let page_size = if page_size == 0 {
      self.options.default_page_size
    } else {
      page_size
    };
    let c = decode_cursor(cursor)?;
    let (page, next) = match format {
      FileFormat::Jsonl => formats::read_lines_page(
        path,
        c,
        page_size,
        self.options.preview_max_chars,
        self.options.raw_max_chars,
      )?,
      FileFormat::Csv => formats::read_csv_page(
        path,
        c,
        page_size,
        self.options.preview_max_chars,
        self.options.raw_max_chars,
      )?,
      FileFormat::Json => formats::read_json_page(
        path,
        c,
        page_size,
        self.options.preview_max_chars,
        self.options.raw_max_chars,
      )?,
      FileFormat::Parquet => formats::read_parquet_page(
        path,
        c,
        page_size,
        self.options.preview_max_chars,
        self.options.raw_max_chars,
      )?,
      _ => return Err(CoreError::UnsupportedFormat(format)),
    };
    let next_cursor = next.map(encode_cursor);
    Ok(RecordPage {
      records: page.records,
      next_cursor,
      reached_eof: page.reached_eof,
    })
  }

  /// IPC API: get_record_raw(session_id, meta) -> String
  ///
  /// This is primarily used when `Record.raw` is truncated (for UI performance) but the user
  /// wants to view/parse the full underlying record.
  pub fn get_record_raw(&self, session_id: &str, meta: RecordMeta) -> Result<String, CoreError> {
    let (path, format) = {
      let sessions = self.sessions.lock();
      let s = sessions
        .get(session_id)
        .ok_or_else(|| CoreError::UnknownSession(session_id.to_string()))?;
      (PathBuf::from(&s.info.path), s.format.clone())
    };

    match format {
      FileFormat::Json | FileFormat::Jsonl | FileFormat::Csv | FileFormat::Parquet => {}
      other => return Err(CoreError::UnsupportedFormat(other)),
    }

    const MAX_RECORD_BYTES: u64 = 50 * 1024 * 1024; // 50MB safety cap
    // For `.json` we ignore `meta.byte_len` and rescan to the end of the value to avoid relying
    // on potentially truncated lengths.
    if format == FileFormat::Json {
      return crate::formats::read_json_value_at_offset(&path, meta.byte_offset, MAX_RECORD_BYTES);
    }
    if format == FileFormat::Parquet {
      // For get_record_raw, we want the full content without truncation.
      // Use a very large value to effectively disable per-cell char limits.
      const FULL_RAW_MAX_CHARS: usize = 100_000_000;
      return crate::formats::read_parquet_row_raw(&path, meta.line_no, FULL_RAW_MAX_CHARS);
    }

    if meta.byte_len > MAX_RECORD_BYTES {
      return Err(CoreError::InvalidArg(format!(
        "record too large: {} bytes (max {})",
        meta.byte_len, MAX_RECORD_BYTES
      )));
    }

    let file_len = std::fs::metadata(&path).ok().map(|m| m.len()).unwrap_or(0);
    if meta.byte_offset > file_len {
      return Err(CoreError::InvalidArg(format!(
        "byte_offset {} beyond file len {}",
        meta.byte_offset, file_len
      )));
    }
    if meta.byte_offset.saturating_add(meta.byte_len) > file_len {
      return Err(CoreError::InvalidArg(format!(
        "range [{}..{}) beyond file len {}",
        meta.byte_offset,
        meta.byte_offset.saturating_add(meta.byte_len),
        file_len
      )));
    }

    let mut f = std::fs::File::open(&path)?;
    f.seek(SeekFrom::Start(meta.byte_offset))?;
    let mut buf = vec![0u8; meta.byte_len as usize];
    f.read_exact(&mut buf)?;

    // Trim common line terminators (for .jsonl/.csv) without touching valid JSON bytes.
    while matches!(buf.last(), Some(b'\n' | b'\r' | 0)) {
      buf.pop();
    }

    Ok(String::from_utf8_lossy(&buf).to_string())
  }
}

fn now_ms() -> i64 {
  SystemTime::now()
    .duration_since(UNIX_EPOCH)
    .unwrap_or_default()
    .as_millis() as i64
}

// (reserved for future internal use)

