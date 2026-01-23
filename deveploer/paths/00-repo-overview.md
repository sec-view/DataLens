## 仓库总览（按路径理解代码在做什么）

本仓库是一个 **Tauri（桌面壳） + SvelteKit（UI） + Rust（核心引擎）** 的数据集浏览工具，实现目标是：**大文件秒级首屏、流式分页、可搜索/可导出**。

---

## 目录一览（高价值路径）

- **`apps/desktop/`**：前端 UI（SvelteKit）+ Tauri 壳（Rust 命令层）
- **`core/`**：Rust 核心引擎（crate 名 `dh_core`）：格式检测、分页读取、搜索、导出、任务、SQLite 持久化
- **`deveploer/`**：开发文档（本目录），`deveploer/main.md` 是总入口
- **`EXAM.md`**：人工测试记录规范（当前仓库尚未创建 `test/` 目录，见 `deveploer/paths/30-test.md`）
- **`dev.sh`**：一键启动开发环境（含端口检查与前端预构建）

---

## 运行时调用链（从 UI 到核心）

- **UI（Svelte）** 调用 `apps/desktop/src/lib/ipc.ts` 的函数（本质是 `@tauri-apps/api` 的 `invoke`）
- **Tauri 命令层（Rust）** 位于 `apps/desktop/src-tauri/src/commands.rs`，负责：
  - 参数接收与少量兼容处理（camelCase/snake_case）
  - 把耗时工作丢进 `spawn_blocking`
  - 需要时通过 `window.emit(...)` 推送进度事件（例如打开大 JSON）
  - 文件系统辅助：目录扫描生成文件树、路径类型判断、OS 打开路径缓存/事件等
  - 超大 JSON：按需加载 children 的“流式 JSON 树”API
- **核心引擎（Rust）** 位于 `core/src/engine.rs`（`CoreEngine`），负责：
  - `open_file / next_page`：分页读取
  - `search`：当前页同步搜索 + 全量扫描任务（可取消）
  - `export`：导出选中/导出搜索任务结果
  - `get_record_raw`：读取完整记录（用于详情截断后的“加载完整内容”）
  - JSON lazy tree：列举子节点与摘要统计（用于超大 JSON 记录的结构浏览）
  - `storage`：SQLite recent/settings

---

## “产物目录”说明（不要当源码读）

这些目录通常是构建产物或本机依赖缓存，**不建议写功能文档/也不应提交改动**：

- **`apps/desktop/node_modules/`**：Node 依赖
- **`apps/desktop/build/`**：SvelteKit/Vite 构建产物（`tauri.conf.json` 的 `distDir` 指向这里）
- **`apps/desktop/src-tauri/target/`**：Rust 编译产物（体积非常大）

---

## 历史遗留说明

- 仓库曾出现过 `apps/apps/desktop/package-lock.json` 这类重复路径（看起来像误拷贝残留），已在清理缓存/无效文件时移除；后续如再次出现，优先确认工程引用路径是否写错。

---

## 一句话抓住“这个仓库现在的产品形态”

- **左侧 Session 面板**：支持拖拽文件/文件夹、显示文件树、最近打开
- **中间记录列表**：分页浏览 + 后端 scan_all 检索任务（可取消/分页取命中）
- **右侧详情面板**：
  - 正常 JSON：结构树 + 高亮检索 + 跳转
  - 超大 JSON：流式 JSON 树（按需加载 children）+ 保留原始文本预览/导出

