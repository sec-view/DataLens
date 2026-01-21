## `core/`（Rust 核心引擎：`dh_core`）

该路径是整个应用的“性能与能力核心”，负责：

- **格式识别**：JSON/JSONL/CSV/Parquet
- **分页读取**：游标（opaque cursor token）驱动的流式读取
- **搜索**：
  - 当前页同步搜索
  - JSONL/CSV 全量扫描搜索（后台任务，可取消）
- **导出**：导出选中记录 / 导出搜索任务命中
- **持久化（SQLite）**：recent files、settings（为后续 UI 会话/配置做准备）

---

## 对外入口（Public API）

`core/src/lib.rs` 作为 crate 的统一出口，主要 re-export：

- **引擎**
  - `CoreEngine`
  - `CoreOptions`
  - `CoreError`
- **模型**
  - `SessionInfo / Record / RecordPage`
  - `SearchQuery / SearchResult / SearchMode`
  - `Task / TaskInfo / TaskKind`
  - `ExportRequest / ExportResult / ExportFormat`
  - `FileFormat`
- **持久化**
  - `Storage / StorageOptions`

---

## 引擎职责（`CoreEngine` 核心语义）

`CoreEngine`（见 `core/src/engine.rs`）是上层（Tauri/CLI）唯一应该直接使用的对象：

- **会话（Session）**
  - `open_file(...)` 会创建新的 `session_id`，并缓存最近一次 `RecordPage`（用于 current_page 搜索）
- **分页**
  - `next_page(session_id, cursor, page_size)`：cursor 是后端返回的 token，前端原样回传
- **搜索**
  - `current_page`：同步对 “最后一次返回的页面” 做 substring 匹配
  - `scan_all`：启动后台任务扫描 JSONL/CSV，可取消，可分页读取命中
- **导出**
  - 当前实现仅支持 JSONL/CSV（逐行语义明确）
- **持久化**
  - `open_file` 会 `touch_recent(path)` 写入 SQLite recent_files（目前 UI 未读取）

---

## 示例与测试

- `core/examples/smoke_open.rs`：命令行 smoke（打开文件、打印首页信息）
- `core/tests/core_flow.rs`：核心闭环测试（打开/翻页/搜索/导出/JSON/Parquet 等）

