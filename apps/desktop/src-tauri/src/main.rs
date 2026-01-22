#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod commands;
#[cfg(target_os = "macos")]
mod macos_open;

use dh_core::{CoreEngine, CoreOptions};

use tauri::Manager;

fn collect_open_paths_from_argv() -> Vec<String> {
  // On macOS, "Open with" / file association often passes the file path(s)
  // as process arguments on cold start.
  std::env::args_os()
    .skip(1)
    .filter_map(|a| {
      let p = std::path::PathBuf::from(&a);
      if p.exists() {
        Some(p.to_string_lossy().to_string())
      } else {
        None
      }
    })
    .collect()
}

fn main() {
  let engine = CoreEngine::new(CoreOptions::default()).expect("init CoreEngine");

  let context = tauri::generate_context!();

  let app = tauri::Builder::default()
    .manage(engine)
    .manage(commands::PendingOpenState(std::sync::Mutex::new(Vec::new())))
    .setup(|app| {
      #[cfg(target_os = "macos")]
      {
        // Handle Finder double-click / "Open With" (AppleEvent openFile/openFiles).
        macos_open::install(app.handle());
      }

      let paths = collect_open_paths_from_argv();
      if paths.is_empty() {
        return Ok(());
      }

      // Store for cold-start scenarios so the frontend can fetch them after booting.
      let state: tauri::State<'_, commands::PendingOpenState> = app.state();
      let mut guard = state.0.lock().unwrap_or_else(|e| e.into_inner());
      for p in paths {
        if !guard.contains(&p) {
          guard.push(p);
        }
      }

      Ok(())
    })
    .invoke_handler(tauri::generate_handler![
      commands::open_file,
      commands::scan_folder_tree,
      commands::path_kind,
      commands::next_page,
      commands::get_record_raw,
      commands::search,
      commands::get_task,
      commands::search_task_hits_page,
      commands::export,
      commands::cancel_task,
      commands::take_pending_open_paths,
      commands::json_list_children,
      commands::json_node_summary,
      commands::json_list_children_at_offset,
      commands::json_node_summary_at_offset
    ])
    .build(context)
    .expect("error while building tauri application");

  // Keep the run loop active; additional "open file" events while running are
  // handled differently across platforms and are not wired up here for Tauri v1.
  app.run(|_app_handle, _event| {});
}

