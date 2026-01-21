use std::path::PathBuf;

use dh_core::{
  CoreEngine, ExportFormat, ExportRequest, ExportResult, RecordPage, SearchQuery, SearchResult,
  RecordMeta, SessionInfo, Task,
};
use serde::{Deserialize, Serialize};
use std::sync::mpsc;
use std::path::Path;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PathKind {
  File,
  Dir,
  Missing,
  Other,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OpenFileResponse {
  pub session: SessionInfo,
  pub first_page: RecordPage,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OpenFileProgressPayload {
  pub request_id: String,
  pub pct_0_100: u8,
  pub stage: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FsNodeKind {
  Dir,
  File,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FsNode {
  pub name: String,
  pub path: String,
  pub kind: FsNodeKind,
  /// Whether the node is selectable (i.e., file format is supported by the app).
  pub supported: bool,
  /// Present only for directories.
  pub children: Option<Vec<FsNode>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FolderTreeResponse {
  pub root: FsNode,
  /// True if we stopped scanning due to limits.
  pub truncated: bool,
  /// Number of nodes returned (including directories and files).
  pub total_nodes: u32,
}

#[tauri::command]
pub fn path_kind(path: String) -> Result<PathKind, String> {
  let p = PathBuf::from(&path);
  let meta = match std::fs::metadata(&p) {
    Ok(m) => m,
    Err(e) => {
      // Normalize "not found" to Missing; other errors bubble up.
      if matches!(e.kind(), std::io::ErrorKind::NotFound) {
        return Ok(PathKind::Missing);
      }
      return Err(format!("stat failed: {path}: {e}"));
    }
  };

  if meta.is_dir() {
    Ok(PathKind::Dir)
  } else if meta.is_file() {
    Ok(PathKind::File)
  } else {
    Ok(PathKind::Other)
  }
}

fn is_supported_path(path: &Path) -> bool {
  let ext = path
    .extension()
    .and_then(|s| s.to_str())
    .unwrap_or("")
    .to_ascii_lowercase();
  matches!(ext.as_str(), "jsonl" | "csv" | "json" | "parquet")
}

fn scan_dir_inner(
  dir: &Path,
  depth: u32,
  max_depth: u32,
  max_nodes: u32,
  nodes_used: &mut u32,
  truncated: &mut bool,
) -> Vec<FsNode> {
  if *nodes_used >= max_nodes {
    *truncated = true;
    return vec![];
  }
  if depth >= max_depth {
    return vec![];
  }

  let mut entries: Vec<std::fs::DirEntry> = match std::fs::read_dir(dir) {
    Ok(rd) => rd.filter_map(Result::ok).collect(),
    Err(_) => return vec![],
  };

  // Sort: dirs first, then by name (case-insensitive).
  entries.sort_by(|a, b| {
    let a_is_dir = a.file_type().map(|t| t.is_dir()).unwrap_or(false);
    let b_is_dir = b.file_type().map(|t| t.is_dir()).unwrap_or(false);
    match (a_is_dir, b_is_dir) {
      (true, false) => std::cmp::Ordering::Less,
      (false, true) => std::cmp::Ordering::Greater,
      _ => a
        .file_name()
        .to_string_lossy()
        .to_ascii_lowercase()
        .cmp(&b.file_name().to_string_lossy().to_ascii_lowercase()),
    }
  });

  let mut out: Vec<FsNode> = Vec::new();
  for ent in entries {
    if *nodes_used >= max_nodes {
      *truncated = true;
      break;
    }
    let p = ent.path();
    let name = ent.file_name().to_string_lossy().to_string();
    let file_type = match ent.file_type() {
      Ok(t) => t,
      Err(_) => continue,
    };

    if file_type.is_dir() {
      *nodes_used += 1;
      let children = scan_dir_inner(&p, depth + 1, max_depth, max_nodes, nodes_used, truncated);
      out.push(FsNode {
        name,
        path: p.to_string_lossy().to_string(),
        kind: FsNodeKind::Dir,
        supported: false,
        children: Some(children),
      });
    } else if file_type.is_file() {
      *nodes_used += 1;
      out.push(FsNode {
        name,
        path: p.to_string_lossy().to_string(),
        kind: FsNodeKind::File,
        supported: is_supported_path(&p),
        children: None,
      });
    } else {
      // Skip symlinks/other special files for now.
    }
  }

  out
}

#[tauri::command]
pub fn scan_folder_tree(
  path: String,
  max_depth: Option<u32>,
  max_nodes: Option<u32>,
) -> Result<FolderTreeResponse, String> {
  let p = PathBuf::from(&path);
  if !p.exists() {
    return Err(format!("folder not found: {}", path));
  }
  if !p.is_dir() {
    return Err(format!("not a folder: {}", path));
  }

  let max_depth = max_depth.unwrap_or(64);
  let max_nodes = max_nodes.unwrap_or(20_000);

  let mut nodes_used: u32 = 0;
  let mut truncated = false;
  let name = p
    .file_name()
    .map(|s| s.to_string_lossy().to_string())
    .unwrap_or_else(|| path.clone());

  // Root is a directory node (counts as 1).
  nodes_used += 1;
  let children = scan_dir_inner(&p, 0, max_depth, max_nodes, &mut nodes_used, &mut truncated);

  Ok(FolderTreeResponse {
    root: FsNode {
      name,
      path,
      kind: FsNodeKind::Dir,
      supported: false,
      children: Some(children),
    },
    truncated,
    total_nodes: nodes_used,
  })
}

#[tauri::command]
pub async fn open_file(
  window: tauri::Window,
  engine: tauri::State<'_, CoreEngine>,
  path: String,
  request_id: Option<String>,
) -> Result<OpenFileResponse, String> {
  let request_id = request_id.unwrap_or_else(|| "default".to_string());
  let engine = engine.inner().clone();

  // Only show progress bar for large files (default: 50MB).
  const PROGRESS_MIN_BYTES: u64 = 50 * 1024 * 1024;
  let file_len = std::fs::metadata(&path).ok().map(|m| m.len()).unwrap_or(0);
  let enable_progress = file_len >= PROGRESS_MIN_BYTES;

  if !enable_progress {
    let worker = tauri::async_runtime::spawn_blocking(move || {
      let (session, first_page) = engine.open_file(path).map_err(|e| e.to_string())?;
      Ok::<_, String>((session, first_page))
    });
    let (session, first_page) = worker
      .await
      .map_err(|e| format!("open_file task join error: {e}"))??;
    return Ok(OpenFileResponse { session, first_page });
  }

  let (tx, rx) = mpsc::channel::<OpenFileProgressPayload>();
  let window2 = window.clone();
  let forward = std::thread::spawn(move || {
    while let Ok(p) = rx.recv() {
      let _ = window2.emit("open_file_progress", p);
    }
  });

  let path2 = path.clone();
  let request_id2 = request_id.clone();
  let tx2 = tx.clone();
  let worker = tauri::async_runtime::spawn_blocking(move || {
    let mut last_pct: u8 = 255;
    let (session, first_page) = engine
      .open_file_with_progress(path2, |pct| {
        // throttle by pct step
        if pct == last_pct {
          return;
        }
        last_pct = pct;
        let _ = tx2.send(OpenFileProgressPayload {
          request_id: request_id2.clone(),
          pct_0_100: pct,
          stage: "载入中".into(),
        });
      })
      .map_err(|e| e.to_string())?;
    Ok::<_, String>((session, first_page))
  });

  let (session, first_page) = worker
    .await
    .map_err(|e| format!("open_file task join error: {e}"))??;

  // Close forwarder
  drop(tx);
  let _ = forward.join();

  Ok(OpenFileResponse { session, first_page })
}

#[tauri::command]
pub fn next_page(
  engine: tauri::State<'_, CoreEngine>,
  session_id: String,
  cursor: Option<String>,
  page_size: Option<u32>,
) -> Result<RecordPage, String> {
  let page_size = page_size.unwrap_or(0) as usize;
  engine
    .next_page(&session_id, cursor.as_deref(), page_size)
    .map_err(|e| e.to_string())
}

#[tauri::command]
pub fn get_record_raw(
  engine: tauri::State<'_, CoreEngine>,
  session_id: String,
  meta: RecordMeta,
) -> Result<String, String> {
  let t0 = std::time::Instant::now();
  let off = meta.byte_offset;
  let len = meta.byte_len;
  let res = engine.get_record_raw(&session_id, meta).map_err(|e| e.to_string());
  match &res {
    Ok(s) => {
      eprintln!(
        "[get_record_raw] ok session_id={} offset={} byte_len={} -> chars={} in {:?}",
        session_id,
        off,
        len,
        s.chars().count(),
        t0.elapsed()
      );
    }
    Err(e) => {
      eprintln!(
        "[get_record_raw] err session_id={} offset={} byte_len={} in {:?}: {}",
        session_id,
        off,
        len,
        t0.elapsed(),
        e
      );
    }
  }
  res
}

#[tauri::command]
pub fn search(
  engine: tauri::State<'_, CoreEngine>,
  session_id: String,
  query: SearchQuery,
) -> Result<SearchResult, String> {
  engine.search(&session_id, query).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn get_task(engine: tauri::State<'_, CoreEngine>, task_id: String) -> Result<Task, String> {
  engine.get_task(&task_id).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn search_task_hits_page(
  engine: tauri::State<'_, CoreEngine>,
  task_id: String,
  cursor: Option<String>,
  page_size: Option<u32>,
) -> Result<RecordPage, String> {
  let page_size = page_size.unwrap_or(0) as usize;
  engine
    .search_task_hits_page(&task_id, cursor.as_deref(), page_size)
    .map_err(|e| e.to_string())
}

#[tauri::command]
pub fn cancel_task(engine: tauri::State<'_, CoreEngine>, task_id: String) -> Result<(), String> {
  engine.cancel_task(&task_id).map_err(|e| e.to_string())
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExportArgs {
  pub session_id: String,
  pub request: ExportRequest,
  pub format: ExportFormat,
  /// output file path
  pub output_path: String,
}

#[tauri::command]
pub fn export(engine: tauri::State<'_, CoreEngine>, args: ExportArgs) -> Result<ExportResult, String> {
  let out = PathBuf::from(args.output_path);
  engine
    .export(&args.session_id, args.request, args.format, out)
    .map_err(|e| e.to_string())
}

