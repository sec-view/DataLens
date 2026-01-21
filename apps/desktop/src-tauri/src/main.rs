#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod commands;

use dh_core::{CoreEngine, CoreOptions};

fn main() {
  let engine = CoreEngine::new(CoreOptions::default()).expect("init CoreEngine");

  tauri::Builder::default()
    .manage(engine)
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
      commands::cancel_task
    ])
    .run(tauri::generate_context!())
    .expect("error while running tauri application");
}

