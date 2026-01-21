## `core/src/formats/`（分页读取：每种格式怎么实现）

这一层解决的是：**如何在不全量加载文件的前提下，分页返回 RecordPage**，并生成可继续翻页的 cursor token。

---

## 统一约定（对上层的承诺）

- 上层（Tauri/UI）只把 `cursor` 当作 opaque 字符串：
  - `open_file` 返回 `first_page.next_cursor`
  - `next_page` 接收 `cursor` 并返回下一页
- `RecordPage.reached_eof` 表示是否到末尾
- `Record.preview/raw` 会被截断以控制内存与渲染成本

---

## `formats/mod.rs`：格式检测与入口

- **`detect_format(path)`**：根据扩展名决定 `FileFormat`
- **入口函数**：
  - `read_lines_page`（JSONL/CSV）
  - `read_json_page` / `read_json_page_with_progress`（JSON）
  - `read_parquet_page`（Parquet）
- **`search_current_page(page, query)`**：对 `page.records[*].preview` 做 substring 匹配（大小写可选）

---

## JSONL / CSV（`formats/lines.rs`）

### 读取策略

- 文件按 **字节 offset** seek 到游标位置，然后逐行 `read_until('\n')`
- 对 CRLF 会做裁剪（`\r\n` → `\n`）
- 对非 UTF-8 内容用 `from_utf8_lossy` 容错（替换为 `�`）

### Cursor 语义

- `Cursor.offset`：下一次读取的字节偏移（seek 入口）
- `Cursor.line`：当前行号（也作为 `Record.id`）

### RecordMeta

- `line_no / byte_offset / byte_len` 都可提供（定位“坏样本”很有用）

---

## JSON（`formats/json.rs`）

### 目标与策略

为避免“读取整个 JSON 再反序列化”的大内存问题，该实现对常见的 root array (`[...]`) 采用 **流式扫描**：

- 跟踪：
  - 字符串状态（引号/转义）
  - 嵌套深度（`{[ ]}`）
- 在 depth==0 且遇到分隔符（`,`/`]`/空白）时判定一个 value 结束
- 只捕获一段有限字节用于构建 `preview/raw`（避免巨大对象撑爆内存）

### Cursor 语义

- `Cursor.offset`：下一个 JSON value 的起始 byte offset（可 seek）
- `Cursor.line`：记录 id（0-based），用于稳定标识

### 进度（可选）

`read_json_page_with_progress` 会在扫描/读取过程中回调 `(done_bytes, total_bytes, stage)`，上层可转成百分比与 stage 文案（Tauri 已接入）。

### 容错与兼容

- 支持：
  - root array：分页读取数组元素
  - root object：作为单条记录（`id=0`）
  - 多个顶层 JSON value（类似 JSON stream 保存成 `.json`）
- 忽略头部 BOM、空白、NUL padding（测试覆盖了 trailing NUL）

---

## Parquet（`formats/parquet.rs`）

### 读取策略

通过 embedded DuckDB 在内存中打开连接并执行：

- `SELECT * FROM read_parquet(?) LIMIT ? OFFSET ?`

并把每行的各列拼接为一行 tab-separated 文本作为 `preview/raw`。

### Cursor 语义（与 lines/json 不同）

- `Cursor.line`：**row offset**（0-based），用于 DuckDB 的 OFFSET
- `Cursor.offset`：当前实现忽略（固定为 0）

### RecordMeta

- 当前不提供 `byte_offset/byte_len`（Parquet 的稳定偏移需要更底层索引支持），因此 `meta=None`

### 约束

- 每次分页会新建一个 in-memory DuckDB connection（实现简单，但不是最优；后续可引入连接复用/会话级缓存）。

