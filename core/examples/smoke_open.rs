use std::path::PathBuf;

use dh_core::{CoreEngine, CoreOptions, StorageOptions};

fn main() -> Result<(), String> {
  let path = std::env::args()
    .nth(1)
    .ok_or_else(|| "usage: cargo run -p dh_core --example smoke_open -- <path-to-file>".to_string())?;
  let path = PathBuf::from(path);

  let dir = tempfile::tempdir().map_err(|e| e.to_string())?;
  let sqlite = dir.path().join("smoke.sqlite");

  let eng = CoreEngine::new(CoreOptions {
    default_page_size: 5,
    preview_max_chars: 120,
    raw_max_chars: 2_000,
    max_concurrent_tasks: 1,
    storage: StorageOptions {
      sqlite_path: Some(sqlite),
    },
  })
  .map_err(|e| e.to_string())?;

  let (session, p1) = eng.open_file(&path).map_err(|e| e.to_string())?;
  println!("format={:?}", session.format);
  println!("records={}", p1.records.len());
  if let Some(r0) = p1.records.first() {
    println!("first.id={}", r0.id);
    println!("first.preview={}", r0.preview);
  }
  Ok(())
}

