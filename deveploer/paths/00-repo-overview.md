## 仓库总览（按路径理解代码在做什么）

本仓库是一个 **Tauri（桌面壳） + SvelteKit（UI） + Rust（核心引擎）** 的数据集浏览工具，实现目标是：**大文件秒级首屏、流式分页、可搜索/可导出**。

---

## 目录一览（高价值路径）

- **`apps/desktop/`**：前端 UI（SvelteKit）+ Tauri 壳（Rust 命令层）
- **`core/`**：Rust 核心引擎（crate 名 `dh_core`）：格式检测、分页读取、搜索、导出、任务、SQLite 持久化
- **`deveploer/`**：开发文档（本目录），`deveploer/main.md` 是总入口
- **`test/`**：测试记录与测试数据（必须遵循 `EXAM.md` 的记录规范）
- **`dev.sh`**：一键启动开发环境（含端口检查与前端预构建）

---

## 运行时调用链（从 UI 到核心）

- **UI（Svelte）** 调用 `apps/desktop/src/lib/ipc.ts` 的函数（本质是 `@tauri-apps/api` 的 `invoke`）
- **Tauri 命令层（Rust）** 位于 `apps/desktop/src-tauri/src/commands.rs`，负责：
  - 参数接收与少量兼容处理（camelCase/snake_case）
  - 把耗时工作丢进 `spawn_blocking`
  - 需要时通过 `window.emit(...)` 推送进度事件（例如打开大 JSON）
- **核心引擎（Rust）** 位于 `core/src/engine.rs`（`CoreEngine`），负责：
  - `open_file / next_page`：分页读取
  - `search`：当前页同步搜索 + 全量扫描任务（可取消）
  - `export`：导出选中/导出搜索任务结果
  - `storage`：SQLite recent/settings

---

## “产物目录”说明（不要当源码读）

这些目录通常是构建产物或本机依赖缓存，**不建议写功能文档/也不应提交改动**：

- **`apps/desktop/node_modules/`**：Node 依赖
- **`apps/desktop/build/`**：SvelteKit/Vite 构建产物（`tauri.conf.json` 的 `distDir` 指向这里）
- **`apps/desktop/src-tauri/target/`**：Rust 编译产物（体积非常大）

---

## 发现的“重复路径”（需要留意但先不改）

- `apps/apps/desktop/package-lock.json` 仅包含一个 `package-lock.json`，看起来像历史遗留/误拷贝。
  - **建议**：后续确认是否可以删除，避免误导（本次任务先只补文档，不做删除动作）。

