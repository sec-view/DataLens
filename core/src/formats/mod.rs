use std::path::Path;

use crate::{
  cursor::Cursor,
  engine::CoreError,
  models::{FileFormat, Record, RecordPage, SearchQuery, SearchResult},
  search_match::PreparedSearch,
};

pub(crate) fn detect_format(path: &Path) -> FileFormat {
  let ext = path
    .extension()
    .and_then(|s| s.to_str())
    .unwrap_or("")
    .to_ascii_lowercase();
  match ext.as_str() {
    "jsonl" => FileFormat::Jsonl,
    "csv" => FileFormat::Csv,
    "json" => FileFormat::Json,
    "parquet" => FileFormat::Parquet,
    _ => FileFormat::Unknown,
  }
}

#[derive(Debug, Clone)]
pub(crate) struct LinesPageInternal {
  pub records: Vec<Record>,
  pub reached_eof: bool,
}

pub(crate) fn read_lines_page(
  path: &Path,
  cursor: Cursor,
  page_size: usize,
  preview_max_chars: usize,
  raw_max_chars: usize,
) -> Result<(LinesPageInternal, Option<Cursor>), CoreError> {
  crate::formats::lines::read_lines_page(path, cursor, page_size, preview_max_chars, raw_max_chars)
}

pub(crate) fn read_csv_page(
  path: &Path,
  cursor: Cursor,
  page_size: usize,
  preview_max_chars: usize,
  raw_max_chars: usize,
) -> Result<(LinesPageInternal, Option<Cursor>), CoreError> {
  crate::formats::csv::read_csv_page(path, cursor, page_size, preview_max_chars, raw_max_chars)
}

pub(crate) fn read_json_page(
  path: &Path,
  cursor: Cursor,
  page_size: usize,
  preview_max_chars: usize,
  raw_max_chars: usize,
) -> Result<(LinesPageInternal, Option<Cursor>), CoreError> {
  crate::formats::json::read_json_page(path, cursor, page_size, preview_max_chars, raw_max_chars)
}

pub(crate) fn read_json_page_with_progress(
  path: &Path,
  cursor: Cursor,
  page_size: usize,
  preview_max_chars: usize,
  raw_max_chars: usize,
  on_progress: Option<&mut dyn FnMut(u64, u64, &'static str)>,
) -> Result<(LinesPageInternal, Option<Cursor>), CoreError> {
  crate::formats::json::read_json_page_with_progress(
    path,
    cursor,
    page_size,
    preview_max_chars,
    raw_max_chars,
    on_progress,
  )
}

/// Read a single JSON value starting at (or after) `offset` and return its full text.
///
/// Used by the UI when a record's `raw` was truncated for performance.
pub(crate) fn read_json_value_at_offset(
  path: &Path,
  offset: u64,
  max_bytes: u64,
) -> Result<String, CoreError> {
  crate::formats::json::read_json_value_at_offset(path, offset, max_bytes)
}

pub(crate) fn read_parquet_page(
  path: &Path,
  cursor: Cursor,
  page_size: usize,
  preview_max_chars: usize,
  raw_max_chars: usize,
) -> Result<(LinesPageInternal, Option<Cursor>), CoreError> {
  crate::formats::parquet::read_parquet_page(path, cursor, page_size, preview_max_chars, raw_max_chars)
}

/// Read a single row from a parquet file (by 0-based row index) and return a JSON string.
///
/// Used by the UI when opening a record from scan_all hits (or when paging raw is truncated).
pub(crate) fn read_parquet_row_raw(
  path: &Path,
  row_idx: u64,
  raw_max_chars: usize,
) -> Result<String, CoreError> {
  crate::formats::parquet::read_parquet_row_raw(path, row_idx, raw_max_chars)
}

pub(crate) fn search_current_page(page: &RecordPage, query: &SearchQuery) -> SearchResult {
  let prepared = match PreparedSearch::new(query) {
    Some(p) => p,
    None => {
      return SearchResult {
        mode: crate::models::SearchMode::CurrentPage,
        hits: vec![],
        task: None,
        truncated: false,
      };
    }
  };

  let mut hits = Vec::new();
  for r in &page.records {
    // Match the same "display content" the UI uses: preview + raw (if present).
    let text = if let Some(raw) = &r.raw {
      format!("{}\n{}", r.preview, raw)
    } else {
      r.preview.clone()
    };
    let hay = if query.case_sensitive { text } else { text.to_lowercase() };
    if prepared.matches_in_hay(&hay) {
      hits.push(r.clone());
    }
  }

  SearchResult {
    mode: crate::models::SearchMode::CurrentPage,
    hits,
    task: None,
    truncated: false,
  }
}

mod lines;
mod csv;
mod json;
mod parquet;
// parquet reader implemented with embedded DuckDB (no external CLI dependency)

