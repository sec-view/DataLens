//! macOS "open document" integration.
//!
//! When the user double-clicks a file associated with the app (or uses Finder "Open With"),
//! macOS delivers an AppleEvent to the app delegate (`application:openFile:` / `application:openFiles:`).
//! If we don't implement these delegate methods, Finder shows an error dialog like:
//! "DataLens cannot open files in the “Apache Parquet” format."
//!
//! Tauri v1 does not expose this event via `RunEvent`, so we patch the delegate at runtime.

#![cfg(target_os = "macos")]

use std::{ffi::CStr, os::raw::c_char, os::raw::c_void};

use cocoa::{appkit::NSApp, base::id};
use objc::{
  msg_send,
  runtime::{BOOL, Class, Object, Sel, YES},
  sel, sel_impl,
};
use once_cell::sync::OnceCell;
use tauri::Manager;

use crate::commands::PendingOpenState;

static APP_HANDLE: OnceCell<tauri::AppHandle> = OnceCell::new();
static INSTALLED: OnceCell<()> = OnceCell::new();

// ObjC runtime FFI (objc crate does not expose these helpers directly).
type IMP = *const c_void;
extern "C" {
  fn class_addMethod(cls: *const Class, name: Sel, imp: IMP, types: *const c_char) -> BOOL;
}

fn handle_open_paths(paths: Vec<String>) {
  if paths.is_empty() {
    return;
  }
  let app = match APP_HANDLE.get() {
    Some(h) => h.clone(),
    None => return,
  };

  // Store for cold-start scenarios so the frontend can fetch them after booting.
  let state = app.state::<PendingOpenState>();
  {
    let mut guard = state.0.lock().unwrap_or_else(|e| e.into_inner());
    for p in &paths {
      if !guard.contains(p) {
        guard.push(p.clone());
      }
    }
  }

  // Also emit a live event (covers already-running scenarios).
  let _ = app.emit_all("open_paths", paths);
}

unsafe fn nsstring_to_string(ns: id) -> Option<String> {
  if ns.is_null() {
    return None;
  }
  let c: *const c_char = msg_send![ns, UTF8String];
  if c.is_null() {
    return None;
  }
  Some(CStr::from_ptr(c).to_string_lossy().to_string())
}

/// `- (BOOL)application:(NSApplication *)sender openFile:(NSString *)filename`
unsafe extern "C" fn application_open_file(
  _this: &Object,
  _cmd: Sel,
  _app: id,
  filename: id,
) -> BOOL {
  if let Some(p) = nsstring_to_string(filename) {
    handle_open_paths(vec![p]);
  }
  // Tell Finder we accepted the open request.
  YES
}

/// `- (void)application:(NSApplication *)sender openFiles:(NSArray<NSString *> *)filenames`
unsafe extern "C" fn application_open_files(_this: &Object, _cmd: Sel, _app: id, filenames: id) {
  if filenames.is_null() {
    return;
  }

  let count: usize = msg_send![filenames, count];
  let mut paths = Vec::with_capacity(count);
  for i in 0..count {
    let ns: id = msg_send![filenames, objectAtIndex: i];
    if let Some(p) = nsstring_to_string(ns) {
      paths.push(p);
    }
  }

  handle_open_paths(paths);

  // IMPORTANT: for `openFiles:` macOS expects an explicit reply; otherwise Finder can show an error dialog.
  // 0 = NSApplicationDelegateReplySuccess
  let ns_app = NSApp();
  let _: () = msg_send![ns_app, replyToOpenOrPrint: 0u64];
}

/// Install macOS open-file handlers by patching the NSApplication delegate.
///
/// Call this once during app setup.
pub fn install(app_handle: tauri::AppHandle) {
  let _ = APP_HANDLE.set(app_handle);
  if INSTALLED.set(()).is_err() {
    return; // already installed
  }

  unsafe {
    let ns_app = NSApp();
    let delegate: id = msg_send![ns_app, delegate];
    if delegate.is_null() {
      return;
    }

    // Add methods to the delegate class (best-effort). If methods already exist, class_addMethod returns 0.
    let cls: *const Class = msg_send![delegate, class];
    if cls.is_null() {
      return;
    }

    // BOOL return, self + _cmd + 2 object args => "c@:@@"
    let _ = class_addMethod(
      cls,
      sel!(application:openFile:),
      application_open_file as IMP,
      b"c@:@@\0".as_ptr() as *const c_char,
    );

    // void return, self + _cmd + 2 object args => "v@:@@"
    let _ = class_addMethod(
      cls,
      sel!(application:openFiles:),
      application_open_files as IMP,
      b"v@:@@\0".as_ptr() as *const c_char,
    );
  }
}

