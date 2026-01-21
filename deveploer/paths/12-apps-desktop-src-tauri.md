## `apps/desktop/src-tauri/`（Tauri 壳：Rust 命令层 + 桥接 CoreEngine）

该路径负责：把前端请求转换为 Rust 调用，并管理与 UI 的交互方式（阻塞任务、进度事件等）。

---

## 入口与关键文件

- **`src-tauri/src/main.rs`**
  - 创建 `dh_core::CoreEngine`（`CoreOptions::default()`）
  - `.manage(engine)` 注入全局 state
  - `.invoke_handler(...)` 注册所有 `#[tauri::command]` 命令
- **`src-tauri/src/commands.rs`**
  - 定义前端可调用命令（open/next/search/task/export/cancel）
  - 负责在必要时把工作放到 `spawn_blocking`，避免阻塞主线程
  - 负责通过 `window.emit(...)` 给前端推送进度事件（`open_file_progress`）
- **`src-tauri/tauri.conf.json`**
  - `build.devPath = http://127.0.0.1:5173`
  - `build.distDir = ../build`（对应前端构建产物目录）
  - `tauri.allowlist.dialog.open/save = true`（前端才能调用系统对话框）
  - `tauri.bundle.icon = ["icons/icon.png"]`

---

## 已实现的 Commands（对外 IPC API）

命令名与 `ipc.ts` 的 `invoke("...")` 对齐：

- **`open_file(path, request_id?) -> OpenFileResponse`**
  - 文件 >= 50MB：启用进度事件（`open_file_progress`）
  - 通过 `spawn_blocking` 执行 `engine.open_file/open_file_with_progress`
- **`next_page(session_id, cursor?, page_size?) -> RecordPage`**
- **`search(session_id, query) -> SearchResult`**
- **`get_task(task_id) -> Task`**
- **`search_task_hits_page(task_id, cursor?, page_size?) -> RecordPage`**
- **`cancel_task(task_id) -> ()`**
- **`export(args: ExportArgs) -> ExportResult`**

---

## 事件（Event）协议

- **`open_file_progress`**：打开大文件时的进度事件
  - payload：`{ request_id, pct_0_100, stage }`
  - 目的：多次打开文件时，用 `request_id` 区分不同请求，避免 UI 进度错乱

---

## 产物目录

- `src-tauri/target/`：Rust 编译输出（非常大），属于构建缓存/产物

