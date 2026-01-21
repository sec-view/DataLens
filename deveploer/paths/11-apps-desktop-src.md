## `apps/desktop/src/`（前端源码：SvelteKit UI）

该路径负责：**界面渲染、用户交互、状态管理**，以及通过 IPC 调用后端能力（打开、分页、搜索、导出、任务轮询/取消）。

---

## 关键子路径

- **`apps/desktop/src/routes/`**
  - `+page.svelte`：主页面（目前几乎所有交互都在这里）
  - `+layout.ts`：SvelteKit 布局（如果后续拆分多页/路由，这里是入口）
- **`apps/desktop/src/lib/`**
  - `ipc.ts`：前端 IPC 层（类型 + 函数封装）

---

## IPC 层（`lib/ipc.ts`）做什么

- **定义与后端对齐的数据类型**
  - `SessionInfo / Record / RecordPage / SearchQuery / SearchResult / Task / ExportRequest ...`
  - 字段命名风格与 Rust 侧 `serde(rename_all="snake_case")` 对齐（例如 `session_id`）
- **封装对 Tauri commands 的调用**
  - `openFile / nextPage / search / getTask / searchTaskHitsPage / cancelTask / exportToFile`
  - 兼容性处理：同一个参数同时发送 `camelCase` 与 `snake_case`（例如 `sessionId` + `session_id`）

---

## 主页面（`routes/+page.svelte`）核心功能

- **打开文件**
  - `open(...)` 弹文件选择器 → `openFile(path, request_id)` → 渲染 `first_page`
  - 监听 `open_file_progress` 事件：当打开大文件时显示进度条
- **分页浏览**
  - `pageCursorHistory` 保存每一页的 cursor，支持“上一页/下一页”
  - `nextPage({ session_id, cursor, page_size })` 加载页面
- **搜索**
  - `current_page`：直接展示 `SearchResult.hits`
  - `scan_all`：拿到 taskId → `getTask` 轮询 → `searchTaskHitsPage` 分页加载命中
  - 支持取消：`cancelTask(taskId)`
- **导出**
  - 目标：
    - `selection`：导出勾选记录 id 集合
    - `search_task`：导出 scan_all 任务的所有命中
  - `save(...)` 选择输出路径 → `exportToFile(...)`
- **Session 面板（左侧）**
  - 默认收起、可拖拽改变宽度
  - 最近打开列表：暂存于 `localStorage`（并非使用 core 的 SQLite recent）

---

## 与后端契约（需要保持稳定的语义）

前端依赖如下语义（由 Rust 保证）：

- `open_file(path) -> { session, first_page }`
- `next_page(session_id, cursor, page_size) -> RecordPage`
  - `cursor` 是后端返回的 opaque token（前端原样回传）
- `search(session_id, query) -> SearchResult`
  - `scan_all` 返回 `task.id`，并通过 `get_task / search_task_hits_page` 取结果
- `export(args) -> ExportResult`

