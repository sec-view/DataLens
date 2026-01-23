## 测试（规范 + 仓库现状）

仓库根目录 `EXAM.md` 规定了**人工测试记录**的格式，但当前仓库里**尚未创建 `test/` 目录**（不要被旧文档误导）。

因此本文件分两部分：

- **规范**：你应该如何记录人工测试（来自 `EXAM.md`）
- **现状**：当前代码里，自动化测试/验收入口在哪里

---

## 人工测试记录规范（来自 `EXAM.md`）

每次测试必须记录：

- **需要测试哪些内容**
- **测试结果**
- **问题聚焦**

并且每次测试后要标明**测试版本**，在计划的 `test/main_record.md` 中做“目录式汇总”。

> 建议后续真的创建 `test/` 目录并落地该规范；本次仅更新文档，不在这里创建目录结构。

---

## 自动化测试与验收入口（当前代码）

- **Rust core 单测/集成测试**：`core/tests/core_flow.rs`
  - 运行：`cargo test --manifest-path core/Cargo.toml`
- **Rust core 示例（手动 smoke）**：`core/examples/smoke_open.rs`、`core/examples/smoke_full_raw.rs`
  - 运行：`cargo run --example smoke_open --manifest-path core/Cargo.toml -- <path>`
- **前端类型检查**：`apps/desktop/package.json` 的 `npm run check`


