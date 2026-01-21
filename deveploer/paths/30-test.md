## `test/`（测试目录：记录规范 + 现状）

该目录用于**保存每次测试的记录**与**测试数据**。仓库根目录 `EXAM.md` 给出了强制规范：每次测试必须记录“测试内容/结果/问题聚焦”，并在 `test/main_record.md` 做目录式汇总。

---

## 必须遵循的测试记录规范（来自 `EXAM.md`）

每次测试必须记录：

- **需要测试哪些内容**
- **测试结果**
- **问题聚焦**

并且：

- 每次测试后都要标明**测试版本**
- 在 `test/main_record.md` 里追加类似目录的记录（方便索引）

---

## 当前已有的测试记录

- `test/main_record.md`
  - 当前仅索引了 `2026-01-13`
- `test/2026-01-13.md`
  - 包含 core/desktop/tauri 的版本、测试项、结果与问题聚焦（并已记录复测结果）

---

## 测试数据（`test/data/`）

- `test/data/training_data_0909.jsonl`
- `test/data/data_classification_alpaca_dataset.json`
- `test/data/train-00000-of-00120.parquet`

这些文件主要用于人工验收与性能/边界情况验证（大文件分页、搜索、导出、parquet 读取等）。

