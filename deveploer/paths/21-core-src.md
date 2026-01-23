## `core/src/`（模块拆解：每个文件负责什么）

这份文档按“文件路径 → 职责 → 关键点”说明 `dh_core` 的内部结构。

---

## `core/src/lib.rs`

- **职责**：crate 的统一出口，集中 `mod` 与 `pub use`。
- **关键点**：
  - 上层（Tauri）应尽量只依赖这里 re-export 的类型，避免直接引用内部模块。

---

## `core/src/models.rs`

- **职责**：IPC/业务模型定义（`serde` 可序列化）。
- **关键点**：
  - `FileFormat`：`jsonl/csv/json/parquet/unknown`
  - `Record`：`id + preview + raw? + meta?`
  - `RecordMeta`：包含 `line_no/byte_offset/byte_len`（对“按行文件”最有意义；Parquet 目前为空）
  - `SearchQuery.mode`：
    - `current_page`（同步）
    - `scan_all`（后台任务）
    - `indexed`（未实现，预留）
  - `ExportRequest`：
    - `selection{record_ids}`：导出选中记录（按行/按 row id）
    - `search_task{task_id}`：导出 scan_all 任务命中集合
    - `json_subtree{meta,path,include_root,children}`：导出“当前 JSON 记录”中的子树（支持超大记录流式导出）
  - JSON lazy tree（用于超大 JSON 记录的结构浏览）：
    - `JsonChildrenPageOffset / JsonNodeSummaryOffset`（UI 当前使用的 offset 版本）

---

## `core/src/engine.rs`

- **职责**：核心 orchestrator（会话管理 + 对外 API）。
- **对外 API**：
  - `open_file / open_file_with_progress`
  - `next_page`
  - `search`
  - `get_task / cancel_task / search_task_hits_page`
  - `export`
  - `get_record_raw`：读取完整记录（用于详情截断后的“加载完整内容”）
  - `json_list_children/json_node_summary`：JSON lazy tree（path 版本）
  - `json_list_children_at_offset/json_node_summary_at_offset`：JSON lazy tree（offset 版本，性能更好）
  - `get_stats`（预留）
- **关键点**：
  - **SessionState** 缓存 `last_page`，用于 current_page 搜索
  - cursor token 使用 `cursor::{encode_cursor, decode_cursor}`（opaque）
  - `.json` 的 open_file 会走 `read_json_page_with_progress`（支持较平滑的进度）

---

## `core/src/cursor.rs`

- **职责**：统一的游标 token 编解码。
- **实现**：`Cursor { offset, line }` → JSON → base64(url-safe no pad)
- **注意**：不同格式对 `offset/line` 的语义不同（详见 formats 文档）。

---

## `core/src/formats/`（格式与分页读取）

- **职责**：按文件格式实现分页读取、格式检测、当前页搜索等。
- **文件**：
  - `formats/mod.rs`：`detect_format` + 统一入口
  - `formats/lines.rs`：JSONL/CSV（按行）分页
  - `formats/json.rs`：`.json` 流式分页（支持 root array、object、以及“多顶层值”）
  - `formats/parquet.rs`：通过 embedded DuckDB 分页读 Parquet
  - `formats/csv.rs`：CSV 的分页读取（支持 preview/raw 截断、meta 定位）

---

## `core/src/tasks.rs`

- **职责**：后台任务系统（目前主要服务于 scan_all 搜索）。
- **关键点**：
  - 并发限制：`max_concurrent_tasks`
  - 取消：`cancel_task` 设置原子标记，扫描循环内检查并提前退出
  - scan_all hits 内存上限：`SearchQuery.max_hits`（超过会 `truncated=true`）
  - hits 分页：`search_task_hits_page(task_id, cursor, page_size)` 使用内部 index cursor

---

## `core/src/export.rs`

- **职责**：导出逻辑（Selection 或 SearchTask）。
- **当前约束**：
  - 支持：JSONL/CSV/JSON/Parquet →（json/jsonl/csv 的一部分组合转换）
  - 对 `.json` 额外支持 `json_subtree`（可对超大记录做流式导出，不需要 full parse）
- **实现要点**：
  - 对输入文件逐行扫描，命中 `record_ids` 就写出（保持原始行内容，统一换行为 `\n`）
  - `json_subtree` 走 formats 的 stream 导出实现（避免一次性加载整条 JSON 记录）

---

## `core/src/storage.rs`

- **职责**：SQLite 持久化（recent_files + settings）。
- **关键点**：
  - 默认路径：`~/.datasets-helper/storage.sqlite`（Windows 用 `%USERPROFILE%`）
  - `touch_recent`：写入/更新最近打开、存在性、置顶标记（pinned）
  - `list_recent(limit)`：读取最近列表（当前 UI 未接入）
  - `set_setting_json/get_setting_json`：可扩展的 JSON 配置存储

