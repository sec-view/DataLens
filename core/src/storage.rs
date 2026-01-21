use std::{
  fs,
  path::{Path, PathBuf},
  time::{SystemTime, UNIX_EPOCH},
};

use rusqlite::{params, Connection};

#[derive(Debug, Clone)]
pub struct StorageOptions {
  /// Path to SQLite file. If None, defaults to ~/.datasets-helper/storage.sqlite (or %USERPROFILE% on Windows).
  pub sqlite_path: Option<PathBuf>,
}

impl Default for StorageOptions {
  fn default() -> Self {
    Self { sqlite_path: None }
  }
}

#[derive(Clone)]
pub struct Storage {
  path: PathBuf,
}

#[derive(Debug, Clone)]
pub struct RecentFile {
  pub path: String,
  pub display_name: String,
  pub last_opened_at_ms: i64,
  pub exists: bool,
  pub pinned: bool,
}

impl Storage {
  pub fn new(opts: StorageOptions) -> Result<Self, String> {
    let path = opts
      .sqlite_path
      .unwrap_or_else(default_sqlite_path)
      .to_path_buf();

    if let Some(parent) = path.parent() {
      fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }

    let conn = Connection::open(&path).map_err(|e| e.to_string())?;
    migrate(&conn).map_err(|e| e.to_string())?;
    Ok(Self { path })
  }

  fn open(&self) -> Result<Connection, String> {
    Connection::open(&self.path).map_err(|e| e.to_string())
  }

  /// Add/update a recent file entry.
  pub fn touch_recent(&self, path: &str, pinned: Option<bool>) -> Result<(), String> {
    let conn = self.open()?;
    let now = now_ms();
    let display_name = Path::new(path)
      .file_name()
      .and_then(|s| s.to_str())
      .unwrap_or(path)
      .to_string();
    let exists = Path::new(path).exists();

    conn
      .execute(
        r#"
INSERT INTO recent_files(path, display_name, last_opened_at, exists_flag, pinned)
VALUES(?1, ?2, ?3, ?4, COALESCE(?5, 0))
ON CONFLICT(path) DO UPDATE SET
  display_name=excluded.display_name,
  last_opened_at=excluded.last_opened_at,
  exists_flag=excluded.exists_flag,
  pinned=COALESCE(?5, pinned)
        "#,
        params![path, display_name, now, exists as i32, pinned.map(|b| b as i32)],
      )
      .map_err(|e| e.to_string())?;
    Ok(())
  }

  pub fn list_recent(&self, limit: usize) -> Result<Vec<RecentFile>, String> {
    let conn = self.open()?;
    let mut stmt = conn
      .prepare(
        r#"
SELECT path, display_name, last_opened_at, exists_flag, pinned
FROM recent_files
ORDER BY pinned DESC, last_opened_at DESC
LIMIT ?1
        "#,
      )
      .map_err(|e| e.to_string())?;

    let rows = stmt
      .query_map(params![limit as i64], |row| {
        Ok(RecentFile {
          path: row.get(0)?,
          display_name: row.get(1)?,
          last_opened_at_ms: row.get(2)?,
          exists: row.get::<_, i64>(3)? != 0,
          pinned: row.get::<_, i64>(4)? != 0,
        })
      })
      .map_err(|e| e.to_string())?;

    let mut out = Vec::new();
    for r in rows {
      out.push(r.map_err(|e| e.to_string())?);
    }
    Ok(out)
  }

  pub fn set_setting_json(&self, key: &str, value_json: &str) -> Result<(), String> {
    let conn = self.open()?;
    conn
      .execute(
        r#"
INSERT INTO settings(key, value_json)
VALUES(?1, ?2)
ON CONFLICT(key) DO UPDATE SET value_json=excluded.value_json
        "#,
        params![key, value_json],
      )
      .map_err(|e| e.to_string())?;
    Ok(())
  }

  pub fn get_setting_json(&self, key: &str) -> Result<Option<String>, String> {
    let conn = self.open()?;
    let mut stmt = conn
      .prepare("SELECT value_json FROM settings WHERE key=?1")
      .map_err(|e| e.to_string())?;
    let mut rows = stmt.query(params![key]).map_err(|e| e.to_string())?;
    if let Some(row) = rows.next().map_err(|e| e.to_string())? {
      let v: String = row.get(0).map_err(|e| e.to_string())?;
      Ok(Some(v))
    } else {
      Ok(None)
    }
  }
}

fn migrate(conn: &Connection) -> Result<(), rusqlite::Error> {
  conn.execute_batch(
    r#"
CREATE TABLE IF NOT EXISTS recent_files(
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  path TEXT NOT NULL UNIQUE,
  display_name TEXT NOT NULL,
  last_opened_at INTEGER NOT NULL,
  exists_flag INTEGER NOT NULL,
  pinned INTEGER NOT NULL DEFAULT 0
);

CREATE TABLE IF NOT EXISTS settings(
  key TEXT PRIMARY KEY,
  value_json TEXT NOT NULL
);
    "#,
  )?;
  Ok(())
}

fn default_sqlite_path() -> PathBuf {
  // Keep it simple & cross-platform without extra deps.
  // - macOS/Linux: $HOME/.datasets-helper/storage.sqlite
  // - Windows: %USERPROFILE%\.datasets-helper\storage.sqlite
  let base = std::env::var_os("HOME")
    .or_else(|| std::env::var_os("USERPROFILE"))
    .map(PathBuf::from)
    .unwrap_or_else(|| PathBuf::from("."));
  base.join(".datasets-helper").join("storage.sqlite")
}

fn now_ms() -> i64 {
  SystemTime::now()
    .duration_since(UNIX_EPOCH)
    .unwrap_or_default()
    .as_millis() as i64
}

