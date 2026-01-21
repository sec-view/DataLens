## `apps/desktop/`（桌面端：SvelteKit UI + Tauri 壳）

该目录是最终用户看到的桌面应用工程，包含两部分：

- **前端 UI**：`apps/desktop/src/`（SvelteKit）
- **桌面壳**：`apps/desktop/src-tauri/`（Tauri，Rust）

---

## 入口与关键文件

- **前端入口**
  - `apps/desktop/src/routes/+page.svelte`：主界面（打开文件、分页、搜索、导出、Session 面板）
  - `apps/desktop/src/lib/ipc.ts`：对 Tauri 命令的 TS 封装（`invoke(...)` + 类型定义）
- **Tauri 入口**
  - `apps/desktop/src-tauri/src/main.rs`：注册 `CoreEngine` 状态与命令 handler
  - `apps/desktop/src-tauri/src/commands.rs`：暴露给前端的命令集合（open/next/search/task/export/cancel）
  - `apps/desktop/src-tauri/tauri.conf.json`：devPath/distDir、窗口配置、allowlist、bundle/icon

---

## 前端（SvelteKit）在这里做什么

- **UI 与交互**（Svelte）：
  - 文件选择/保存对话框：`@tauri-apps/api/dialog`
  - 事件订阅：监听 `open_file_progress`（打开大文件时显示进度条）
  - 分页浏览：维护 `pageCursorHistory`（支持上一页/下一页的历史游标）
  - 搜索：
    - `current_page`：直接拿到 `hits`
    - `scan_all`：先拿到 taskId，轮询任务，再分页读取 hits
  - 导出：支持导出“选中记录”或“全量搜索结果”
- **注意**：目前 “最近打开文件” 列表在前端用 `localStorage` 维护（并非读取 core 的 SQLite recent）。

---

## Tauri（命令层）在这里做什么

- **桥接 `dh_core::CoreEngine`**
  - `open_file`：文件 < 50MB 时直接打开；>= 50MB 时启用进度事件（`window.emit("open_file_progress", ...)`）
  - `next_page/search/get_task/search_task_hits_page/cancel_task/export`：薄封装，基本是 `engine.xxx(...).map_err(to_string)`
- **并发/阻塞策略**
  - `open_file` 使用 `tauri::async_runtime::spawn_blocking`，避免阻塞 UI 线程

---

## 生成产物（不要当源码修改）

- **`apps/desktop/node_modules/`**：依赖
- **`apps/desktop/build/`**：前端 build 产物（被 Tauri `distDir` 使用）
- **`apps/desktop/src-tauri/target/`**：Rust build 产物

---

## 开发命令（见 `package.json`）

- `npm run dev`：Vite dev（默认 `127.0.0.1:5173`）
- `npm run tauri`：`tauri dev`（会使用 `tauri.conf.json` 中的 devPath，通常需要配合 Vite）
- 推荐通过仓库根的 `./dev.sh` 启动（会检查端口、预构建前端）。

