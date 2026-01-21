use std::{path::PathBuf, thread, time::Duration};

use dh_core::{
  CoreEngine, CoreOptions, ExportFormat, ExportRequest, JsonPathSegment, SearchMode, SearchQuery,
  StorageOptions,
};

fn engine_with_sqlite(sqlite_path: PathBuf) -> CoreEngine {
  CoreEngine::new(CoreOptions {
    default_page_size: 2,
    preview_max_chars: 50,
    raw_max_chars: 200,
    max_concurrent_tasks: 2,
    storage: StorageOptions {
      sqlite_path: Some(sqlite_path),
    },
  })
  .unwrap()
}

#[test]
fn open_next_page_cursor_no_dup_no_drop() {
  let dir = tempfile::tempdir().unwrap();
  let sqlite = dir.path().join("t.sqlite");
  let file = dir.path().join("a.jsonl");
  std::fs::write(
    &file,
    "a\nb\nc\nd\n", // 4 lines
  )
  .unwrap();

  let eng = engine_with_sqlite(sqlite);
  let (_session, p1) = eng.open_file(&file).unwrap();
  assert_eq!(p1.records.len(), 2);
  assert_eq!(p1.records[0].id, 0);
  assert_eq!(p1.records[1].id, 1);

  let cursor = p1.next_cursor.clone().unwrap();
  let sid = _session.session_id.clone();
  let p2 = eng.next_page(&sid, Some(&cursor), 2).unwrap();
  assert_eq!(p2.records.len(), 2);
  assert_eq!(p2.records[0].id, 2);
  assert_eq!(p2.records[1].id, 3);
  assert!(p2.reached_eof);
}

#[test]
fn crlf_and_non_utf8_tolerant() {
  let dir = tempfile::tempdir().unwrap();
  let sqlite = dir.path().join("t.sqlite");
  let file = dir.path().join("a.csv");
  // "a\r\n" + 0xff 0xfe + "\r\n"
  let mut bytes = Vec::new();
  bytes.extend_from_slice(b"a\r\n");
  bytes.extend_from_slice(&[0xff, 0xfe, b'x', b'\r', b'\n']);
  std::fs::write(&file, bytes).unwrap();

  let eng = engine_with_sqlite(sqlite);
  let (session, p1) = eng.open_file(&file).unwrap();
  let _ = session;
  assert_eq!(p1.records.len(), 2);
  // raw line should have CRLF trimmed; non-utf8 becomes replacement chars.
  assert_eq!(p1.records[0].raw.as_deref().unwrap(), "a");
  assert!(p1.records[1].preview.contains('x'));
}

#[test]
fn search_current_page_works() {
  let dir = tempfile::tempdir().unwrap();
  let sqlite = dir.path().join("t.sqlite");
  let file = dir.path().join("a.jsonl");
  std::fs::write(&file, "hello\nworld\n").unwrap();

  let eng = engine_with_sqlite(sqlite);
  let (session, _p1) = eng.open_file(&file).unwrap();
  let res = eng
    .search(
      &session.session_id,
      SearchQuery {
        text: "wor".into(),
        mode: SearchMode::CurrentPage,
        case_sensitive: false,
        max_hits: 100,
      },
    )
    .unwrap();
  assert_eq!(res.hits.len(), 1);
  assert_eq!(res.hits[0].id, 1);
}

#[test]
fn scan_all_search_and_export_selection() {
  let dir = tempfile::tempdir().unwrap();
  let sqlite = dir.path().join("t.sqlite");
  let file = dir.path().join("a.jsonl");
  std::fs::write(&file, "aa\nbb\naa\n").unwrap();

  let eng = engine_with_sqlite(sqlite);
  let (session, _p1) = eng.open_file(&file).unwrap();

  let r = eng
    .search(
      &session.session_id,
      SearchQuery {
        text: "aa".into(),
        mode: SearchMode::ScanAll,
        case_sensitive: true,
        max_hits: 100,
      },
    )
    .unwrap();
  let task_id = r.task.unwrap().id;

  // Wait a bit for background task to finish (tiny file).
  for _ in 0..50 {
    let t = eng.get_task(&task_id).unwrap();
    if t.finished {
      break;
    }
    thread::sleep(Duration::from_millis(10));
  }

  let hits_page = eng.search_task_hits_page(&task_id, None, 10).unwrap();
  assert_eq!(hits_page.records.len(), 2);
  assert_eq!(hits_page.records[0].id, 0);
  assert_eq!(hits_page.records[1].id, 2);

  // Export selection
  let out = dir.path().join("out.jsonl");
  let ex = eng
    .export(
      &session.session_id,
      ExportRequest::Selection {
        record_ids: vec![2],
      },
      ExportFormat::Jsonl,
      &out,
    )
    .unwrap();
  assert_eq!(ex.records_written, 1);
  let out_s = std::fs::read_to_string(out).unwrap();
  assert_eq!(out_s, "aa\n");
}

#[test]
fn export_csv_to_jsonl_and_json() {
  let dir = tempfile::tempdir().unwrap();
  let sqlite = dir.path().join("t.sqlite");
  let file = dir.path().join("a.csv");
  std::fs::write(&file, "id,name,score\n1,Alice,98\n2,Bob,87\n").unwrap();

  let eng = engine_with_sqlite(sqlite);
  let (session, _p1) = eng.open_file(&file).unwrap();

  // jsonl
  let out1 = dir.path().join("out.jsonl");
  let ex1 = eng
    .export(
      &session.session_id,
      ExportRequest::Selection {
        record_ids: vec![1, 2],
      },
      ExportFormat::Jsonl,
      &out1,
    )
    .unwrap();
  assert_eq!(ex1.records_written, 2);
  let s1 = std::fs::read_to_string(out1).unwrap();
  assert!(s1.contains(r#""id":"1""#));
  assert!(s1.contains(r#""name":"Alice""#));

  // json
  let out2 = dir.path().join("out.json");
  let ex2 = eng
    .export(
      &session.session_id,
      ExportRequest::Selection {
        record_ids: vec![1],
      },
      ExportFormat::Json,
      &out2,
    )
    .unwrap();
  assert_eq!(ex2.records_written, 1);
  let s2 = std::fs::read_to_string(out2).unwrap();
  assert!(s2.trim_start().starts_with('['));
  assert!(s2.contains(r#""name":"Alice""#));
}

#[test]
fn export_parquet_to_jsonl() {
  let dir = tempfile::tempdir().unwrap();
  let sqlite = dir.path().join("t.sqlite");
  let file = dir.path().join("a.parquet");

  let conn = duckdb::Connection::open_in_memory().unwrap();
  let _ = conn.execute_batch("LOAD parquet;");
  conn
    .execute_batch(
      "CREATE TABLE t(x VARCHAR, y INTEGER);
       INSERT INTO t VALUES ('hello', 1), ('world', 2);",
    )
    .unwrap();
  conn
    .execute(
      "COPY (SELECT * FROM t ORDER BY y) TO ? (FORMAT PARQUET);",
      duckdb::params![file.to_string_lossy().to_string()],
    )
    .unwrap();

  let eng = engine_with_sqlite(sqlite);
  let (session, _p1) = eng.open_file(&file).unwrap();

  let out = dir.path().join("out.jsonl");
  let ex = eng
    .export(
      &session.session_id,
      ExportRequest::Selection { record_ids: vec![1] },
      ExportFormat::Jsonl,
      &out,
    )
    .unwrap();
  assert_eq!(ex.records_written, 1);
  let s = std::fs::read_to_string(out).unwrap();
  assert!(s.contains(r#""x":"world""#));
}

#[test]
fn export_json_subtree_root_and_children() {
  let dir = tempfile::tempdir().unwrap();
  let sqlite = dir.path().join("t.sqlite");
  let file = dir.path().join("a.json");
  std::fs::write(&file, r#"[{"a":{"b":[1,2,3]}}]"#).unwrap();

  let eng = engine_with_sqlite(sqlite);
  let (session, p1) = eng.open_file(&file).unwrap();
  let meta = p1.records[0].meta.clone().unwrap();

  // export subtree root: [1,2,3]
  let out1 = dir.path().join("subtree.jsonl");
  let ex1 = eng
    .export(
      &session.session_id,
      ExportRequest::JsonSubtree {
        meta: meta.clone(),
        path: vec![JsonPathSegment::Key("a".into()), JsonPathSegment::Key("b".into())],
        include_root: true,
        children: vec![],
      },
      ExportFormat::Jsonl,
      &out1,
    )
    .unwrap();
  assert_eq!(ex1.records_written, 1);
  let s1 = std::fs::read_to_string(out1).unwrap();
  assert_eq!(s1.trim(), "[1,2,3]");

  // export selected child: 2
  let out2 = dir.path().join("child.jsonl");
  let ex2 = eng
    .export(
      &session.session_id,
      ExportRequest::JsonSubtree {
        meta,
        path: vec![JsonPathSegment::Key("a".into()), JsonPathSegment::Key("b".into())],
        include_root: false,
        children: vec![JsonPathSegment::Index(1)],
      },
      ExportFormat::Jsonl,
      &out2,
    )
    .unwrap();
  assert_eq!(ex2.records_written, 1);
  let s2 = std::fs::read_to_string(out2).unwrap();
  assert_eq!(s2.trim(), "2");
}

#[test]
fn scan_all_search_json_root_array_works() {
  let dir = tempfile::tempdir().unwrap();
  let sqlite = dir.path().join("t.sqlite");
  let file = dir.path().join("a.json");
  std::fs::write(&file, "[{\"x\":\"hello\"},{\"x\":\"world\"}]").unwrap();

  let eng = engine_with_sqlite(sqlite);
  let (session, _p1) = eng.open_file(&file).unwrap();

  let r = eng
    .search(
      &session.session_id,
      SearchQuery {
        text: "world".into(),
        mode: SearchMode::ScanAll,
        case_sensitive: true,
        max_hits: 100,
      },
    )
    .unwrap();
  let task_id = r.task.unwrap().id;

  for _ in 0..100 {
    let t = eng.get_task(&task_id).unwrap();
    if t.finished {
      break;
    }
    thread::sleep(Duration::from_millis(10));
  }

  let hits_page = eng.search_task_hits_page(&task_id, None, 10).unwrap();
  assert_eq!(hits_page.records.len(), 1);
  assert_eq!(hits_page.records[0].id, 1);
  let meta = hits_page.records[0].meta.clone().unwrap();
  // For json scan_all we should return an element start byte offset.
  assert!(meta.byte_offset > 0);

  let raw = eng.get_record_raw(&session.session_id, meta).unwrap();
  assert!(raw.contains("world"));
}

#[test]
fn scan_all_search_parquet_works() {
  let dir = tempfile::tempdir().unwrap();
  let sqlite = dir.path().join("t.sqlite");
  let file = dir.path().join("a.parquet");

  // Build a tiny parquet file using DuckDB.
  let conn = duckdb::Connection::open_in_memory().unwrap();
  let _ = conn.execute_batch("LOAD parquet;");
  conn
    .execute_batch(
      "CREATE TABLE t(x VARCHAR, y INTEGER);
       INSERT INTO t VALUES ('hello', 1), ('world', 2);",
    )
    .unwrap();
  conn
    .execute(
      "COPY (SELECT * FROM t ORDER BY y) TO ? (FORMAT PARQUET);",
      duckdb::params![file.to_string_lossy().to_string()],
    )
    .unwrap();

  let eng = engine_with_sqlite(sqlite);
  let (session, _p1) = eng.open_file(&file).unwrap();

  let r = eng
    .search(
      &session.session_id,
      SearchQuery {
        text: "world".into(),
        mode: SearchMode::ScanAll,
        case_sensitive: true,
        max_hits: 100,
      },
    )
    .unwrap();
  let task_id = r.task.unwrap().id;

  for _ in 0..200 {
    let t = eng.get_task(&task_id).unwrap();
    if t.finished {
      break;
    }
    thread::sleep(Duration::from_millis(10));
  }

  let hits_page = eng.search_task_hits_page(&task_id, None, 10).unwrap();
  assert_eq!(hits_page.records.len(), 1);
  assert_eq!(hits_page.records[0].id, 1);
  let meta = hits_page.records[0].meta.clone().unwrap();

  let raw = eng.get_record_raw(&session.session_id, meta).unwrap();
  assert!(raw.contains("world"));
}


#[test]
fn parquet_open_returns_helpful_error_for_invalid_parquet() {
  let dir = tempfile::tempdir().unwrap();
  let sqlite = dir.path().join("t.sqlite");
  let file = dir.path().join("a.parquet");
  std::fs::write(&file, b"not a parquet").unwrap();

  let eng = engine_with_sqlite(sqlite);
  let err = eng.open_file(&file).unwrap_err().to_string();
  // We should surface a readable message (not "unsupported format") even without any external CLI.
  assert!(err.to_lowercase().contains("parquet") || err.to_lowercase().contains("duckdb"));
}

#[test]
fn json_array_paging_works() {
  let dir = tempfile::tempdir().unwrap();
  let sqlite = dir.path().join("t.sqlite");
  let file = dir.path().join("a.json");
  std::fs::write(&file, "[\n  {\"x\":1},\n  {\"x\":2},\n  {\"x\":3}\n]\n").unwrap();

  let eng = engine_with_sqlite(sqlite);
  let (session, p1) = eng.open_file(&file).unwrap();
  assert_eq!(session.format, dh_core::FileFormat::Json);
  assert_eq!(p1.records.len(), 2);
  assert_eq!(p1.records[0].id, 0);
  assert_eq!(p1.records[1].id, 1);

  let cursor = p1.next_cursor.clone().unwrap();
  let p2 = eng.next_page(&session.session_id, Some(&cursor), 2).unwrap();
  assert_eq!(p2.records.len(), 1);
  assert_eq!(p2.records[0].id, 2);
  assert!(p2.reached_eof);
}

#[test]
fn json_object_root_is_single_record() {
  let dir = tempfile::tempdir().unwrap();
  let sqlite = dir.path().join("t.sqlite");
  let file = dir.path().join("a.json");
  std::fs::write(&file, "{\n  \"x\": 1,\n  \"y\": \"ok\"\n}\n").unwrap();

  let eng = engine_with_sqlite(sqlite);
  let (session, p1) = eng.open_file(&file).unwrap();
  assert_eq!(session.format, dh_core::FileFormat::Json);
  assert_eq!(p1.records.len(), 1);
  assert_eq!(p1.records[0].id, 0);
  assert!(p1.reached_eof);
}

#[test]
fn json_multiple_top_level_values_are_supported() {
  let dir = tempfile::tempdir().unwrap();
  let sqlite = dir.path().join("t.sqlite");
  let file = dir.path().join("a.json");
  // Common "NDJSON" / JSON stream content saved as .json
  std::fs::write(&file, "{\"x\":1}\n{\"x\":2}\n{\"x\":3}\n").unwrap();

  let eng = engine_with_sqlite(sqlite);
  let (session, p1) = eng.open_file(&file).unwrap();
  assert_eq!(session.format, dh_core::FileFormat::Json);
  assert_eq!(p1.records.len(), 2);
  assert_eq!(p1.records[0].id, 0);
  assert_eq!(p1.records[1].id, 1);

  let cursor = p1.next_cursor.clone().unwrap();
  let p2 = eng.next_page(&session.session_id, Some(&cursor), 2).unwrap();
  assert_eq!(p2.records.len(), 1);
  assert_eq!(p2.records[0].id, 2);
  assert!(p2.reached_eof);
}

#[test]
fn json_trailing_nul_bytes_are_ignored() {
  let dir = tempfile::tempdir().unwrap();
  let sqlite = dir.path().join("t.sqlite");
  let file = dir.path().join("a.json");
  let mut bytes = Vec::new();
  bytes.extend_from_slice(b"[{\"x\":1},{\"x\":2}]\n");
  bytes.extend_from_slice(&[0, 0, 0, 0]);
  std::fs::write(&file, bytes).unwrap();

  let eng = engine_with_sqlite(sqlite);
  let (_session, p1) = eng.open_file(&file).unwrap();
  assert_eq!(p1.records.len(), 2);
  assert!(p1.reached_eof);
}
