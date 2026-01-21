## 开发文档（Developer Guide）

本文是一份**从零开始**开发 `datasets-helper` 的工程文档：定义明确的跨平台技术栈、工程结构、核心架构与协议约定，并给出可以按里程碑推进的实现路径。

---

## 文档入口（按路径索引）

> 你可以把这里当作“总目录”。每份文档都聚焦到一个**代码路径**：它负责什么、入口在哪里、核心模块/流程是什么。

- **仓库总览**：[`deveploer/paths/00-repo-overview.md`](./paths/00-repo-overview.md)
- **桌面端（前端）**：[`deveploer/paths/10-apps-desktop.md`](./paths/10-apps-desktop.md)
  - **前端源码（SvelteKit）**：[`deveploer/paths/11-apps-desktop-src.md`](./paths/11-apps-desktop-src.md)
  - **桌面壳（Tauri）**：[`deveploer/paths/12-apps-desktop-src-tauri.md`](./paths/12-apps-desktop-src-tauri.md)
- **核心引擎（Rust / dh_core）**：[`deveploer/paths/20-core.md`](./paths/20-core.md)
  - **core/src 模块拆解**：[`deveploer/paths/21-core-src.md`](./paths/21-core-src.md)
  - **formats（分页读取实现）**：[`deveploer/paths/22-core-formats.md`](./paths/22-core-formats.md)
- **测试（test/ 目录规范与记录）**：[`deveploer/paths/30-test.md`](./paths/30-test.md)
- **开发脚本（dev.sh）**：[`deveploer/paths/31-dev-sh.md`](./paths/31-dev-sh.md)

---

## 目标（Goals）

- **跨平台单代码库**：同一套代码同时支持 **macOS 与 Windows**（未来可扩展 Linux），不维护两套 UI/两套业务逻辑。
- **秒级首屏 + 低内存**：超大文件也能快速打开并流式浏览，不做全量加载。
- **面向数据工作流**：适配大模型数据集检查、抽样、排错定位、审阅与协作导出。

## 非目标（Non-Goals）

- 不做通用 IDE/编辑器（以“浏览/审阅/定位”为主）。
- 第一阶段不做重度 ETL（转换/清洗可以后置）。

## 支持格式（阶段性目标）

- **M1**：`.jsonl`、`.csv`（先“按行预览”保证首屏速度）
- **M3**：`.parquet`（通过 DuckDB 做按需读取、过滤与统计）
- `.json`：后续补结构树视图与大文件策略

---

## 技术选型（定案）

- **桌面壳**：**Tauri**
- **核心引擎**：**Rust（stable）**
- **UI**：**SvelteKit（TypeScript）**
- **包管理器**：**pnpm（推荐）/ npm（兼容）**
- **数据引擎（M3 引入）**：**DuckDB**
- **持久化（Recent/书签/配置）**：**SQLite（rusqlite）**

选择理由（与目标强绑定）：

- Tauri + Rust 能把性能关键路径（I/O、解析、索引、导出）放到原生侧，UI 只渲染“当前页”，从源头控制内存与卡顿。
- SvelteKit 适合工具类桌面 UI（轻量、组件简单），并天然跨平台复用。
- DuckDB 让 Parquet/CSV 的过滤/统计/列裁剪具备工程上的确定性（谓词下推、按需读取）。

---

## 目录结构（从零初始化后必须保持）

- `apps/desktop/`
  - `src/`：SvelteKit 前端（UI + 状态管理）
  - `src-tauri/`：Tauri 壳 + 命令入口（仅做“桥接”）
- `crates/dh_core/`：Rust 核心（流式读取/分页/搜索/导出/索引）
- `crates/dh_storage/`：Rust 持久化（SQLite：Recent/书签/配置/任务历史）
- `crates/dh_duckdb/`：Rust DuckDB 适配（阶段性引入：M3）
- `test_data/`：示例数据（可脱敏）

---

## 开发环境（macOS 优先，但必须保证可扩展到 Windows）

- **macOS 13+**
- **Xcode Command Line Tools**（Tauri 构建/签名需要）
- **Rust**：`rustup`（stable）
- **Node.js**：LTS（建议 Node 20 LTS 或更新的 LTS）
- **pnpm（推荐）** 或 **npm（兼容）**

---

## 初始化与开发命令（从零开始）

> 这里给出“应该怎么做”的标准流程。实际命令细节按 Tauri 官方文档落地即可，但工程结构与约定以本文件为准。

- **初始化 Tauri 项目**：创建 `apps/desktop/`（包含 `src/` 与 `src-tauri/`）
- **初始化 Rust workspace**：创建 `crates/*`，并在根目录统一管理依赖与版本策略
- **开发模式**：
  - 前端热更新 + Tauri dev
  - Rust 侧命令与核心 crate 走 workspace 引用

---

## 产品交互（必须实现的 UI 框架）

### 主界面布局

- **最左侧：会话（Session）面板**：
  - 仅用于展示**当前打开文件**与**历史会话/最近打开记录**
  - 支持**拖拽调整宽度**（可滑动改变大小）
  - **默认宽度较小**，并且**默认收起**（缩进状态），需要时再展开
  - 注：这里不承载搜索与导出配置，避免左侧面板过重
- **中间列表**：分页记录列表（虚拟滚动/分页加载）
- **右侧详情**：选中记录详情（语法高亮、长字段折叠、复制等）

### 顶部工具条

- Open（系统文件选择器）
- Search / Filter（统一入口；在工具条中**横向排列**参数与控件）
- Export（以**按钮**形式呈现；点击后打开**子窗口/弹窗**来完成导出配置与执行）
- 任务进度（后台任务：扫描/导出/索引/统计）

---

## 核心架构（必须遵守）

### 分层原则

- **UI 层**：只负责渲染与交互，不直接读文件、不做重解析。
- **命令层（Tauri commands）**：薄桥接，参数校验 + 调用核心引擎。
- **核心引擎（Rust）**：I/O、解析、分页、搜索、导出、索引、统计。

### 核心数据模型（建议）

- `SessionInfo`：`session_id`, `path`, `format`, `created_at`
- `Record`：`id`, `preview`, `raw?`, `meta?`
- `RecordPage`：`records[]`, `next_cursor`, `reached_eof`
- `CursorToken`：**opaque string**（前端不解释）

---

## Rust ↔ UI 协议（IPC API，必须按此实现）

> 目标：即使底层实现从“按行扫描”升级为“索引/duckdb”，前端也无需改接口语义。

- **打开文件**
  - `open_file(path) -> { session: SessionInfo, first_page: RecordPage }`
- **分页读取**
  - `next_page(session_id, cursor, page_size) -> RecordPage`
  - `cursor` 是后端返回的 **opaque token**，前端原样回传
- **搜索/过滤**
  - `search(session_id, query, mode) -> SearchResult`
  - `mode`：`current_page | scan_all | indexed`（演进路径固定）
- **导出**
  - `export(session_id, selection, format, output_path) -> ExportResult`
- **统计（M3）**
  - `get_stats(session_id, spec) -> StatsResult`

---

## 搜索与过滤（语义与语法约定）

目标：在不同数据源（JSONL 扫描 / DuckDB / 索引）之间保持**一致的用户心智与 API 语义**。

### SearchQuery（统一查询对象）

- `query.text`：用户输入的搜索文本
- `query.mode`：
  - `current_page`：只在当前页 records 中匹配
  - `scan_all`：流式扫描整个文件（必须可取消）
  - `indexed`：基于索引定位（M4 可选）
- `query.case_sensitive`：默认 `false`

> M1 实现约定：对 JSONL/CSV（按行预览）先做“raw line 的 substring 匹配”，不做昂贵的结构化解析；M3 之后对 DuckDB 数据源可扩展为列级过滤与 SQL/表达式适配。

### Filter（阶段性约定）

- **M1**：仅支持 `search`（不实现复杂 filter 语法，避免前期架构漂移）
- **M3（DuckDB）**：支持“列过滤”
  - UI 侧构造 `FilterSpec`（列名、操作符、值）
  - Rust 侧在 DuckDB 里编译为安全的参数化表达式（禁止字符串拼 SQL）

---

## 文件格式实现策略（必须按顺序）

### JSONL（M1）

- 流式按行读取（chunk + 分割换行）
- `preview` 截断（避免 UI 长字符串撑爆内存）
- 跨页搜索：先扫描式（可取消），后索引式（M4 可选）

### CSV（M1/M3）

- M1：先做“按行预览”（不做严格 CSV 解析，保证首屏）
- M3：接 DuckDB 后切换为“列视图 + 条件过滤 + 统计”

### Parquet（M3）

- 通过 DuckDB：列裁剪 + 谓词下推 + 分页输出

---

## 后台任务系统（进度 / 取消 / 资源上限）

凡是可能“扫描全文件/运行较久”的操作（跨页搜索、导出、统计、索引构建）必须走统一任务系统：

- **任务模型**：`Task { id, kind, started_at, progress, cancellable }`
- **取消**：UI 触发 `cancel_task(task_id)`；Rust 侧必须定期检查取消信号并尽快退出
- **资源上限**：
  - 并发数限制（避免同时多个全量扫描）
  - 内存上限策略（分页缓存/字符串截断/结果集上限）
  - 结果集上限（例如最多返回 N 条命中；支持“继续加载更多命中”）

---

## 性能与稳定性约束（验收标准）

- **首屏**：打开文件后尽快返回 `first_page`（目标：秒级）
- **内存**：禁止全量读入；UI 只保留“当前页 + 少量缓存页”
- **后台任务**：搜索/导出/索引/统计必须可取消，且不阻塞 UI
- **异常处理**：文件编码异常、超长行、损坏 parquet、权限不足都要可提示可恢复

---

## 持久化与配置（必须统一）

- 使用 SQLite（`dh_storage`）存储：
  - Recent/书签（含存在性标记、最后打开时间）
  - 用户偏好（默认浏览模式、每页条数、字段截断长度、主题等）
  - 可选：任务历史（上次搜索条件、上次导出位置）

### SQLite 表结构（建议定稿，避免后续迁移成本）

- `recent_files`
  - `id`（PK）
  - `path`（unique）
  - `display_name`
  - `last_opened_at`（unix ms）
  - `exists`（bool）
  - `pinned`（bool，书签/置顶可复用）
- `settings`
  - `key`（PK）
  - `value_json`（统一用 JSON 存储，便于扩展）

---

## 平台集成（macOS/Windows 都要考虑，但实现尽量薄）

- **文件关联打开**：`.json/.jsonl/.csv/.parquet`
- **文件权限**：
  - macOS：沙盒与 security-scoped bookmarks（如启用沙盒发行）
  - Windows：标准路径权限即可（但要处理文件被占用/锁定）

---

## 测试与性能基准（必须有）

- **单元测试（Rust）**
  - 流式分页：不同换行（LF/CRLF）、超长行、非 UTF-8 的容错
  - 游标一致性：多次 `next_page` 不重复/不丢行
- **集成测试（最小闭环）**
  - 打开文件 → 首页 → 翻页 → 搜索（current_page/scan_all）→ 导出
- **性能基准（Bench）**
  - 首屏耗时（打开到 `first_page`）
  - 连续翻页吞吐
  - 全量扫描搜索耗时（含可取消）

---

## 里程碑（从零推进）

- **M1（可用的核心预览）**
  - Tauri + SvelteKit + Rust workspace
  - JSONL/CSV：open + 分页 + 详情 + Recent
  - 当前页搜索
- **M2（核心价值）**
  - 跨页扫描式搜索（可取消）+ 导出选中/导出结果
  - 书签/配置持久化（SQLite）
- **M3（Parquet + 统计）**
  - DuckDB 接入：Parquet/CSV 的过滤与统计
  - 列视图、TopK/缺失率等基础统计
- **M4（体验与规模）**
  - 可选：增量索引、断点恢复、性能打磨、主题/快捷键

---

## 代码规范（必须执行）

- 性能关键路径只能在 Rust；前端不做重解析。
- 任何可能扫描全文件的操作必须：
  - 显示进度
  - 支持取消
  - 有明确的资源上限（时间/内存/并发）

