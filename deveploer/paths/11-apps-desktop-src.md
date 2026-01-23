## `apps/desktop/src/`（前端源码：SvelteKit UI）

该路径负责：**界面渲染、用户交互、状态管理**，以及通过 IPC 调用后端能力（打开、分页、搜索、导出、任务轮询/取消）。

---

## 关键子路径

- **`apps/desktop/src/routes/`**
  - `+page.svelte`：主页面（目前几乎所有交互都在这里）
  - `+layout.ts`：SvelteKit 布局（如果后续拆分多页/路由，这里是入口）
- **`apps/desktop/src/lib/`**
  - `ipc.ts`：前端 IPC 层（类型 + 函数封装）
  - `platform.ts / web_backend.ts`：运行环境抽象（Tauri vs Web 测试模式）
  - `components/`：文件树、JSON 结构树、JSON 流式树等组件
  - `workers/text_search.worker.ts`：详情面板大文本检索计数（并行分片）

---

## IPC 层（`lib/ipc.ts`）做什么

- **定义与后端对齐的数据类型**
  - `SessionInfo / Record / RecordPage / SearchQuery / SearchResult / Task / ExportRequest ...`
  - 字段命名风格与 Rust 侧 `serde(rename_all="snake_case")` 对齐（例如 `session_id`）
- **封装对 Tauri commands 的调用**
  - `openFile / nextPage / search / getTask / searchTaskHitsPage / cancelTask / exportToFile`
  - 兼容性处理：同一个参数同时发送 `camelCase` 与 `snake_case`（例如 `sessionId` + `session_id`）
  - 补充能力（与 UI 新交互强相关）：
    - `pathKind / scanFolderTree`：用于拖拽/打开文件夹生成“文件树”
    - `takePendingOpenPaths`：用于 app 启动后补取 OS 传来的“待打开路径”
    - `getRecordRaw`：用于详情截断后的“加载完整内容”
    - JSON 流式树：`jsonListChildrenAtOffset / jsonNodeSummaryAtOffset`

---

## 主页面（`routes/+page.svelte`）核心功能

- **打开文件**
  - `open(...)` 弹文件选择器 → `openFile(path, request_id)` → 渲染 `first_page`
  - 监听 `open_file_progress` 事件：当打开大文件时显示进度条
- **打开文件夹（文件树）**
  - `scanFolderTree({ path })` 获取可展开文件树；文件节点标注 `supported`
  - 点击树上的文件会触发 `openFilePath`
- **拖拽导入**
  - 支持拖拽文件 / 文件夹到左侧 Session 面板
  - 在 Tauri 下优先使用 `tauri://file-drop` 事件获取真实路径（macOS 下 HTML5 DataTransfer 常拿不到路径）
- **OS 启动打开（macOS）**
  - UI 启动后调用 `takePendingOpenPaths()` 拉取“启动时 OS 发来的路径”
  - 运行中通过事件 `open_paths` 接收追加打开请求（模拟一次拖拽处理）
- **分页浏览**
  - `pageCursorHistory` 保存每一页的 cursor，支持“上一页/下一页”
  - `nextPage({ session_id, cursor, page_size })` 加载页面
- **搜索**
  - **记录面板**：默认走后端 `scan_all`（后台任务 + 进度 + 可取消 + 命中分页）
  - **详情面板**：对当前详情 JSON 做高亮与跳转，并用 worker 并行计算命中数（大文本优化）
  - 额外：支持 `key:value` 形式的“弱结构化”匹配（对齐详情高亮规则）
- **导出**
  - 支持三种请求：
    - `selection`：导出勾选记录 id 集合
    - `search_task`：导出 scan_all 任务的所有命中
    - `json_subtree`：导出当前 JSON 记录中的某个子树/子项（用于超大 JSON，不必把整条记录读进来）
  - `save(...)` 选择输出路径 → `exportToFile(...)`
- **Session 面板（左侧）**
  - 默认收起、可拖拽改变宽度
  - 最近打开列表：暂存于 `localStorage`（当前 UI 未读取 core 的 SQLite recent）
  - 文件树展开状态：也暂存于 `localStorage`（`folderExpanded`）
- **超大 JSON 详情**
  - 当记录过大无法完整塞进 IPC/解析时，详情会启用 `JsonLazyTree`（流式按需加载 children）

---

## 与后端契约（需要保持稳定的语义）

前端依赖如下语义（由 Rust 保证）：

- `open_file(path) -> { session, first_page }`
- `next_page(session_id, cursor, page_size) -> RecordPage`
  - `cursor` 是后端返回的 opaque token（前端原样回传）
- `search(session_id, query) -> SearchResult`
  - `scan_all` 返回 `task.id`，并通过 `get_task / search_task_hits_page` 取结果
- `export(args) -> ExportResult`
 - `get_record_raw(session_id, meta) -> String`
 - `scan_folder_tree(path, ...) -> FolderTreeResponse`
 - `take_pending_open_paths() -> string[]`
 - `json_list_children_at_offset / json_node_summary_at_offset`

