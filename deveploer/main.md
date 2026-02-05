## 开发文档（Developer Guide）

本文档是 `datasets_helper` 的**当前代码版**开发指南（不是“从零搭架子”的规划稿）。目标是让你在仓库现状下，能快速：

- 跑起来（Tauri + SvelteKit + Rust core）
- 定位功能入口（UI / IPC / core）
- 理解关键协议（cursor、task、json_subtree、lazy-json-tree）
- 打包发布（macOS dmg）

> 目录名 `deveploer/` 是历史拼写，**先以现状为准**，文档中仍沿用该路径。

---

## 文档入口（按路径索引）

每份文档聚焦到一个**代码路径**（它负责什么、入口在哪里、核心流程是什么）：

- **仓库总览**：`deveploer/paths/00-repo-overview.md`
- **桌面端（前端）**：`deveploer/paths/10-apps-desktop.md`
  - **前端源码（SvelteKit）**：`deveploer/paths/11-apps-desktop-src.md`
  - **桌面壳（Tauri）**：`deveploer/paths/12-apps-desktop-src-tauri.md`
- **核心引擎（Rust / dh_core）**：`deveploer/paths/20-core.md`
  - **core/src 模块拆解**：`deveploer/paths/21-core-src.md`
  - **formats（分页读取实现）**：`deveploer/paths/22-core-formats.md`
- **测试规范与现状**：`deveploer/paths/30-test.md`
- **开发脚本（dev.sh）**：`deveploer/paths/31-dev-sh.md`

---

## 当前能力概览（以代码为准）

- **支持格式**：`.jsonl`、`.csv`、`.json`、`.parquet`
- **分页**：基于 cursor token（opaque），前端不解析
- **检索**：
  - `current_page`：同步匹配最后一页（低成本）
  - `scan_all`：后台任务（可取消、可分页取命中）
  - `.json` 的 `scan_all`：**仅支持 root array**（见 core 实现）
- **导出**：
  - selection / search_task
  - `.json` 额外支持 `json_subtree`（导出当前记录内的子树/子项，支持超大记录的流式导出）
- **超大 JSON 详情**：提供 **流式 JSON 树**（按需加载 children，不需要把整条记录解析到内存）
- **文件夹树**：可扫描目录生成文件树，标记哪些格式可打开
- **系统集成**：macOS “双击关联文件 / Open With” 可把 path 交给正在运行的 app 处理

---

## 快速开始（开发）

### 前置依赖

- **Rust stable**（`rustup`）
- **Node.js**（建议 LTS）
- **npm**（仓库脚本默认用 npm）

### 安装依赖

在 `apps/desktop/`：

- `npm install`

如果你本机设置了 `NODE_ENV=production`（或 npm 配置导致跳过 devDependencies），会出现 `vite: command not found` / `svelte-kit: command not found`。此时用：

- `npm install --include=dev`

### 开发启动

推荐从仓库根目录启动：

- `./dev.sh`（默认 `tauri` 模式，会检查/释放 5173，并在启动前 `vite build`）

只启动 Vite：

- `./dev.sh vite`

### 核心测试（Rust）

在仓库根目录：

- `cargo test --manifest-path core/Cargo.toml`

---

## 架构约定（读代码时的心智模型）

- **UI（SvelteKit）**：只做交互与渲染，所有 I/O/解析/大计算都在 Rust
- **IPC（Tauri commands）**：薄桥接（参数整理 + 调用 core + 必要的线程/事件处理）
- **core（Rust / dh_core）**：格式检测、分页、搜索任务、导出、SQLite 持久化、JSON lazy tree

---

## IPC 契约（重要：稳定语义）

下面这些接口语义要保持稳定（实现可演进，语义不随意变）：

- `open_file(path, request_id?) -> { session, first_page }`
- `next_page(session_id, cursor?, page_size?) -> RecordPage`
- `search(session_id, query) -> SearchResult`
  - `scan_all` 返回 `task`，用 `get_task / search_task_hits_page` 取结果
- `cancel_task(task_id)`
- `export({ session_id, request, format, output_path }) -> ExportResult`
  - `request.type`：`selection | search_task | json_subtree`
- `get_record_raw(session_id, meta) -> String`（用于详情截断后的“加载完整内容”）
- 文件树与系统打开：
  - `path_kind(path) -> file|dir|missing|other`
  - `scan_folder_tree(path, max_depth?, max_nodes?) -> FolderTreeResponse`
  - `take_pending_open_paths() -> string[]`（UI 启动后补取 OS 传入的路径）
- JSON lazy tree（超大 JSON 记录）：
  - `json_list_children_at_offset(...)`
  - `json_node_summary_at_offset(...)`

---

## 常见问题（Troubleshooting）

- **5173 被占用导致 dev 起不来**：`./dev.sh` 默认会尝试释放端口；如不希望自动 kill，占用端口时设置 `FORCE_KILL=0`
- **UI 修改没生效**：`./dev.sh` 默认会清理 Svelte/Vite 缓存并 `vite build`；想加快启动可设置 `REBUILD_FRONTEND=0` / `CLEAN_FRONTEND=0`
- **详情“加载完整内容”失败**：受 Tauri IPC 大小上限影响，`core` 侧对单条记录读取有 50MB safety cap；超大 JSON 会切到 `JsonLazyTree` 流式浏览与/或提示用导出查看

---

## 变更记录

- 2026-02-05：将产品名由 DataLens 更换为 FluxPeek，并同步更新 README、Tauri 配置与 macOS UTI 标识。

