<p align="center">
  <img src="asset/derived/logo-320.png" alt="DataLens" />
</p>

# DataLens（数据透镜）

[English](README.md) | **中文**

一个用于 **秒级首屏、低内存** 浏览超大数据文件的桌面工具（Tauri + Rust + SvelteKit）。  
面向大模型数据集检查/清洗排错/抽样审阅等工作流，让你不用担心“双击大文件把系统卡死”。

## 项目优势与作用

- **安全打开大文件**：避免“双击大文件把系统卡死”。
- **秒级首屏**：流式读取、分页渲染，不把整个文件一次性读进内存。
- **面向数据集工作流**：检查、清洗排错、抽样审阅，并支持导出选中/搜索结果。
- **覆盖常用格式**：JSONL / CSV / JSON / Parquet（Parquet 通过 DuckDB 读取）。

### 解析引擎升级（更大文件也能打开）

- **更稳的 JSONL 读取**：按行流式扫描，但只在首屏/详情中传输“有限前缀”，避免超长单行导致 IPC/内存暴涨。
- **超长记录更友好**：详情默认可展示最多 **40,000** 字符；更大的记录建议使用“流式结构浏览（JsonLazyTree）”按需展开。

## 安装（macOS）

可直接从 [GitHub Releases](../../releases/latest) 下载可安装的 **`.dmg`** 包。

- 打开 `.dmg`，将 **DataLens.app** 拖入 **Applications（应用程序）**
- 若首次打开被系统拦截：右键应用 → **打开**（或到 **系统设置 → 隐私与安全性** 中放行）

## 特性

- **流式分页**：不把整个文件一次性读进内存，按页加载与渲染
- **大文件打开进度**：文件较大时展示加载进度（阈值默认 50MB）
- **快速定位**
  - **当前页搜索**：同步返回命中
  - **全量扫描搜索（可取消）**：后台任务扫描全文件，结果支持分页拉取
- **导出**
  - 导出“选中记录”
  - 导出“全量搜索结果”（基于任务结果）
- **原始记录查看**：当预览被截断时，可按需拉取更完整的 raw 内容
- **文件夹扫描**：扫描目录树并标记“是否为支持格式”

## 支持的格式

- `.jsonl`（JSON Lines）
- `.csv`
- `.json`
- `.parquet`（通过 DuckDB 读取）

## 快速开始（开发运行）

### 环境要求

- **Node.js**：建议 Node 20 LTS（或更新的 LTS）
- **Rust**：stable（建议通过 `rustup` 安装）
- **Tauri 构建依赖**
  - macOS：安装 Xcode Command Line Tools（`xcode-select --install`）
  - 其他平台参考 Tauri 官方前置依赖文档：[Tauri Prerequisites](https://tauri.app/v1/guides/getting-started/prerequisites/)

### 安装依赖

```bash
cd apps/desktop
npm install
```

### 启动（推荐）

仓库根目录提供了更稳定的开发启动脚本 `dev.sh`（会检查端口、可选清缓存、可选预构建前端，并确保退出时子进程一起退出）。

```bash
./dev.sh
```

常用模式：

```bash
./dev.sh tauri   # 默认：启动 tauri dev（包含 vite dev）
./dev.sh vite    # 只启动 vite dev（不启动 tauri 壳）
```

可选环境变量：

```bash
# 端口被占用时是否自动 kill（默认 1）
FORCE_KILL=0 ./dev.sh

# 启动前跳过前端 rebuild / cache clean（默认都是 1）
REBUILD_FRONTEND=0 CLEAN_FRONTEND=0 ./dev.sh
```

## 目录结构（你会最常看的路径）

- `apps/desktop/`
  - `src/`：SvelteKit UI（主界面、交互逻辑）
  - `src/lib/ipc.ts`：前端 IPC 封装（`invoke` + 类型定义）
  - `src-tauri/`：Tauri 壳（Rust 命令层）
- `core/`：Rust 核心引擎（crate：`dh_core`）
- `dev.sh`：开发启动脚本
- `deveploer/`：开发文档（入口：`deveploer/main.md`）
- `test/`：测试记录（规范见 `EXAM.md`）

> 注意：`apps/desktop/node_modules/`、`apps/desktop/src-tauri/target/` 等为构建产物/依赖缓存，不建议写功能文档、也不应把变更当作源码修改。

## IPC / 核心能力（实现情况）

前端通过 `apps/desktop/src/lib/ipc.ts` 调用 Tauri 命令（`apps/desktop/src-tauri/src/commands.rs`），再进入 `core/src/engine.rs`（`CoreEngine`）。

已实现的核心接口（按能力分类）：

- **文件/目录**
  - 打开文件：`open_file(path) -> { session, first_page }`
  - 分页读取：`next_page(session_id, cursor, page_size) -> RecordPage`
  - 获取原始记录：`get_record_raw(session_id, meta) -> String`
  - 扫描文件夹树：`scan_folder_tree(path, max_depth, max_nodes)`
- **搜索**
  - 当前页：`mode = current_page`
  - 全量扫描任务：`mode = scan_all`（返回 taskId，可 `cancel_task`，并可分页拉取命中）
- **导出**
  - 选中记录导出：`request = selection`
  - 搜索任务结果导出：`request = search_task`

## 路线图（下一步想做的）

- **Indexed Search（M4）**：增量索引/更快的跨页定位
- **统计与列级过滤（M3）**：schema 推断、缺失率、TopK，DuckDB 过滤/谓词下推
- **体验增强**：断点恢复、缓存策略、主题/快捷键、文件关联打开

## 测试与记录

测试目录在 `test/`，每次测试请按 `EXAM.md` 的规范记录到 `test/main_record.md`（测试内容、结果、问题聚焦、版本）。

## License

目前 Tauri 壳 crate 已声明为 MIT（见 `apps/desktop/src-tauri/Cargo.toml`）。如需补齐仓库根目录的 `LICENSE` 文件，可在后续版本完善。
