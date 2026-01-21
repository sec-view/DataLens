use std::path::Path;

use base64::Engine as _;
use serde_json::{Map, Value};

use crate::{
  cursor::Cursor,
  engine::CoreError,
  formats::LinesPageInternal,
  models::{Record, RecordMeta},
};

/// Parquet paging implementation via embedded DuckDB (no external CLI dependency).
///
/// Cursor semantics:
/// - `cursor.line` is used as row offset (0-based).
/// - `cursor.offset` is ignored.
pub(crate) fn read_parquet_page(
  path: &Path,
  cursor: Cursor,
  page_size: usize,
  preview_max_chars: usize,
  raw_max_chars: usize,
) -> Result<(LinesPageInternal, Option<Cursor>), CoreError> {
  let offset = cursor.line;
  let path_str = path
    .to_str()
    .ok_or_else(|| CoreError::InvalidArg("invalid path encoding".into()))?;

  let offset_i64 = i64::try_from(offset).map_err(|_| {
    CoreError::InvalidArg(format!("invalid cursor offset for parquet: {offset}"))
  })?;
  let limit_i64 = i64::try_from(page_size)
    .map_err(|_| CoreError::InvalidArg(format!("invalid page_size: {page_size}")))?;

  let mut records = Vec::with_capacity(page_size);
  let mut row_idx = offset;

  let conn = duckdb::Connection::open_in_memory()
    .map_err(|e| CoreError::InvalidArg(format!("DuckDB 初始化失败：{e}")))?;

  // Some builds require explicitly loading the parquet extension even when compiled with it.
  // Ignore errors to be tolerant across versions/builds.
  let _ = conn.execute_batch("LOAD parquet;");

  let mut stmt = conn
    .prepare("SELECT * FROM read_parquet(?) LIMIT ? OFFSET ?")
    .map_err(|e| CoreError::InvalidArg(format!("DuckDB 准备语句失败：{e}")))?;

  let mut rows = stmt
    .query(duckdb::params![path_str, limit_i64, offset_i64])
    .map_err(|e| CoreError::InvalidArg(format!("Parquet 读取失败：{e}")))?;

  let cell_max = raw_max_chars.min(2000).max(64);

  while let Some(row) = rows
    .next()
    .map_err(|e| CoreError::InvalidArg(format!("Parquet 读取失败：{e}")))?
  {
    let col_count = row.as_ref().column_count();
    let mut cols = Vec::with_capacity(col_count);
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

      cols.push(sanitize_cell(&value_to_string(&v)));
      obj.insert(key, duckdb_value_to_json(&v, cell_max));
    }

    let line = cols.join("\t");
    let preview = truncate_chars(&line, preview_max_chars);

    // Provide JSON-like raw for the detail view (keys are parquet column names).
    // Keep JSON valid (do NOT truncate the entire JSON string).
    let json_raw = serde_json::to_string(&Value::Object(obj))
      .unwrap_or_else(|_| format!(r#"{{"__raw__":"{}"}}"#, sanitize_json_string(&line)));
    let raw = Some(json_raw);

    records.push(Record {
      id: row_idx,
      preview,
      raw,
      // We don't have stable offsets without internal parquet indexing; omit meta.
      meta: None::<RecordMeta>,
    });
    row_idx += 1;
  }

  // Heuristic: if we got fewer rows than requested, assume eof.
  let reached_eof = records.len() < page_size;
  let next = if reached_eof {
    None
  } else {
    Some(Cursor {
      offset: 0,
      line: offset + records.len() as u64,
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

/// Read a single parquet row (0-based) and return it as a JSON string.
///
/// This is used by `get_record_raw` for scan_all hits (which only carry `RecordMeta`).
pub(crate) fn read_parquet_row_raw(
  path: &Path,
  row_idx: u64,
  raw_max_chars: usize,
) -> Result<String, CoreError> {
  let path_str = path
    .to_str()
    .ok_or_else(|| CoreError::InvalidArg("invalid path encoding".into()))?;

  let offset_i64 = i64::try_from(row_idx).map_err(|_| {
    CoreError::InvalidArg(format!("invalid row index for parquet: {row_idx}"))
  })?;

  let conn = duckdb::Connection::open_in_memory()
    .map_err(|e| CoreError::InvalidArg(format!("DuckDB 初始化失败：{e}")))?;
  let _ = conn.execute_batch("LOAD parquet;");

  let mut stmt = conn
    .prepare("SELECT * FROM read_parquet(?) LIMIT 1 OFFSET ?")
    .map_err(|e| CoreError::InvalidArg(format!("DuckDB 准备语句失败：{e}")))?;

  let mut rows = stmt
    .query(duckdb::params![path_str, offset_i64])
    .map_err(|e| CoreError::InvalidArg(format!("Parquet 读取失败：{e}")))?;

  let Some(row) = rows
    .next()
    .map_err(|e| CoreError::InvalidArg(format!("Parquet 读取失败：{e}")))?
  else {
    return Err(CoreError::InvalidArg(format!(
      "parquet row out of range: {row_idx}"
    )));
  };

  let col_count = row.as_ref().column_count();
  let mut obj = Map::with_capacity(col_count);
  let cell_max = raw_max_chars.min(2000).max(64);
  for i in 0..col_count {
    let key = row
      .as_ref()
      .column_name(i)
      .map(|s| s.to_string())
      .unwrap_or_else(|_| format!("col_{i}"));
    let v: duckdb::types::Value = row
      .get(i)
      .map_err(|e| CoreError::InvalidArg(format!("Parquet 读取失败：{e}")))?;
    obj.insert(key, duckdb_value_to_json(&v, cell_max));
  }

  serde_json::to_string(&Value::Object(obj))
    .map_err(|e| CoreError::InvalidArg(format!("Parquet 行序列化失败：{e}")))
}

fn sanitize_cell(s: &str) -> String {
  // Keep the output line-based and tab-separated for preview.
  s.replace(&['\n', '\r', '\t'][..], " ")
}

fn sanitize_json_string(s: &str) -> String {
  // Minimal escaping for fallback JSON construction (only used in error paths).
  s.replace('\\', "\\\\").replace('"', "\\\"")
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

fn duckdb_value_to_json(v: &duckdb::types::Value, cell_max: usize) -> Value {
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
      .unwrap_or_else(|| Value::String(truncate_chars(&x.to_string(), cell_max))),
    V::Double(x) => serde_json::Number::from_f64(*x)
      .map(Value::Number)
      .unwrap_or_else(|| Value::String(truncate_chars(&x.to_string(), cell_max))),

    V::Text(s) => Value::String(truncate_chars(s, cell_max)),

    // Keep other types stable by stringifying (still readable in JsonTree).
    other => Value::String(truncate_chars(&value_to_string(other), cell_max)),
  }
}

