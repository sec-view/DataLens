use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum FileFormat {
  Jsonl,
  Csv,
  Json,
  Parquet,
  Unknown,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionInfo {
  pub session_id: String,
  pub path: String,
  pub format: FileFormat,
  pub created_at_ms: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RecordMeta {
  pub line_no: u64,
  pub byte_offset: u64,
  pub byte_len: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Record {
  pub id: u64,
  pub preview: String,
  pub raw: Option<String>,
  pub meta: Option<RecordMeta>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RecordPage {
  pub records: Vec<Record>,
  pub next_cursor: Option<String>,
  pub reached_eof: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SearchMode {
  CurrentPage,
  ScanAll,
  Indexed,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchQuery {
  pub text: String,
  pub mode: SearchMode,
  pub case_sensitive: bool,
  /// For scan_all: max number of hits to keep in memory.
  pub max_hits: u64,
}

impl Default for SearchQuery {
  fn default() -> Self {
    Self {
      text: String::new(),
      mode: SearchMode::CurrentPage,
      case_sensitive: false,
      max_hits: 10_000,
    }
  }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchResult {
  pub mode: SearchMode,
  pub hits: Vec<Record>,
  /// For scan_all: a background task id you can poll/cancel/fetch hits from.
  pub task: Option<TaskInfo>,
  pub truncated: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum TaskKind {
  SearchScanAll,
  Export,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskInfo {
  pub id: String,
  pub kind: TaskKind,
  pub cancellable: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Task {
  pub id: String,
  pub kind: TaskKind,
  pub started_at_ms: i64,
  pub progress_0_100: u8,
  pub cancellable: bool,
  pub finished: bool,
  pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ExportFormat {
  Json,
  Jsonl,
  Csv,
}

/// A JSON path segment used by the UI to refer to a subtree.
///
/// This is intentionally "untagged" so the IPC payload can be a simple
/// array like `["foo", 0, "bar"]`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(untagged)]
pub enum JsonPathSegment {
  Key(String),
  Index(u64),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "type")]
pub enum ExportRequest {
  /// Export these record ids (line numbers) from the session file.
  Selection { record_ids: Vec<u64> },
  /// Export results produced by a scan_all search task.
  SearchTask { task_id: String },
  /// Export a subtree (or selected children under it) from the CURRENT record.
  ///
  /// - `meta` points to the underlying record in the source file (JSON record).
  /// - `path` selects a subtree within that record (empty means root of that record).
  /// - If `include_root` is true: export the subtree value itself.
  /// - Otherwise: export the selected direct children under the subtree (`children`).
  JsonSubtree {
    meta: RecordMeta,
    path: Vec<JsonPathSegment>,
    include_root: bool,
    children: Vec<JsonPathSegment>,
  },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExportResult {
  pub output_path: String,
  pub records_written: u64,
}

// --- JSON lazy tree (for huge records) ---

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum JsonNodeKind {
  Object,
  Array,
  String,
  Number,
  Boolean,
  Null,
  Unknown,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JsonChildItem {
  /// Key or index under the current node.
  pub seg: JsonPathSegment,
  /// Best-effort value kind (derived from the first non-ws byte).
  pub kind: JsonNodeKind,
  /// Best-effort preview (truncated).
  pub preview: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JsonChildrenPage {
  pub items: Vec<JsonChildItem>,
  /// Next cursor for paging children (0-based). `None` means no more.
  pub next_cursor: Option<u64>,
  pub reached_end: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JsonNodeSummary {
  pub kind: JsonNodeKind,
  /// For object/array: number of direct children counted so far.
  /// For leaf nodes: None.
  pub child_count: Option<u64>,
  /// True if counting reached the real end; false if we stopped due to limits.
  pub complete: bool,
}

// --- JSON lazy tree v2: offset-based (fast seek, no repeated path scans) ---

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JsonChildItemOffset {
  /// Key or index under the current node.
  pub seg: JsonPathSegment,
  /// Best-effort value kind (derived from the first non-ws byte).
  pub kind: JsonNodeKind,
  /// Best-effort preview (truncated).
  pub preview: String,
  /// Absolute byte offset (in the session file) where this child's value starts.
  pub value_offset: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JsonChildrenPageOffset {
  pub items: Vec<JsonChildItemOffset>,
  /// Absolute byte offset for the next page cursor within the same parent node.
  /// `None` means no more.
  pub next_cursor_offset: Option<u64>,
  /// For arrays: the next element index corresponding to `next_cursor_offset`.
  /// For objects: always `None`.
  pub next_cursor_index: Option<u64>,
  pub reached_end: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JsonNodeSummaryOffset {
  pub kind: JsonNodeKind,
  pub child_count: Option<u64>,
  pub complete: bool,
  /// Absolute byte offset of the node value (echoed back for convenience/debugging).
  pub node_offset: u64,
}

/// Reserved for M3 (DuckDB stats).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StatsResult {
  pub message: String,
}

