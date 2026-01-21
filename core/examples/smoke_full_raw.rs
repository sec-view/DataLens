use std::path::PathBuf;

use dh_core::{CoreEngine, CoreOptions, StorageOptions};

fn main() -> Result<(), String> {
  let path = std::env::args()
    .nth(1)
    .ok_or_else(|| "usage: cargo run --example smoke_full_raw -- <path-to-json>".to_string())?;
  let path = PathBuf::from(path);

  let dir = tempfile::tempdir().map_err(|e| e.to_string())?;
  let sqlite = dir.path().join("smoke.sqlite");

  let eng = CoreEngine::new(CoreOptions {
    default_page_size: 1,
    preview_max_chars: 120,
    raw_max_chars: 8_000,
    max_concurrent_tasks: 1,
    storage: StorageOptions {
      sqlite_path: Some(sqlite),
    },
  })
  .map_err(|e| e.to_string())?;

  let (session, p1) = eng.open_file(&path).map_err(|e| e.to_string())?;
  let r0 = p1.records.first().ok_or_else(|| "no records".to_string())?;
  println!("format={:?}", session.format);
  println!("page.raw.len={}", r0.raw.as_deref().unwrap_or("").chars().count());
  println!("page.raw.ends_with_ellipsis={}", r0.raw.as_deref().unwrap_or("").ends_with('…'));
  println!("meta={:?}", r0.meta);

  let meta = r0.meta.clone().ok_or_else(|| "no meta".to_string())?;
  let full = eng
    .get_record_raw(&session.session_id, meta)
    .map_err(|e| e.to_string())?;

  println!("full.len(chars)={}", full.chars().count());
  println!("full.ends_with_ellipsis={}", full.ends_with('…'));
  let tail: String = full.chars().rev().take(40).collect::<Vec<_>>().into_iter().rev().collect();
  println!("full.tail(40)={tail}");
  Ok(())
}

