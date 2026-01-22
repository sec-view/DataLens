<p align="center">
  <img src="asset/derived/logo-320.png" alt="DataLens" />
</p>

# DataLens

**English** | [中文](README.zh-CN.md)

A desktop app for browsing huge dataset files with **seconds-to-first-screen** performance and **low memory usage** (Tauri + Rust + SvelteKit).


## Why DataLens

- **Open huge files safely**: no more “double-click a big file and freeze the whole system”.
- **Fast first screen**: stream and render by pages instead of loading everything into memory.
- **Designed for LLM dataset workflows**: inspection, cleaning/debugging, sampling review, and export of selections/search results.
- **Practical formats support**: JSONL / CSV / JSON / Parquet (via DuckDB).

## Install (macOS)

Download the ready-to-install **`.dmg`** from [GitHub Releases](../../releases/latest).

- Open the `.dmg`, then drag **DataLens.app** into **Applications**
- If macOS blocks the first launch: right click the app → **Open** (or allow it in **System Settings → Privacy & Security**)

## Key Features

- **Streaming pagination**: load and render by pages, not whole-file in memory
- **Open progress for large files**: shows loading progress when file is large (default threshold: 50MB)
- **Fast navigation**
  - **Current-page search**: synchronous, instant hits
  - **Full scan search (cancelable)**: background job scanning the whole file; results can be fetched in pages
- **Export**
  - Export **selected records**
  - Export **full search results** (from background task results)
- **Raw record view**: fetch more complete raw content on-demand when preview is truncated
- **Folder scan**: scan a directory tree and mark whether each file is in supported formats

## Supported Formats

- `.jsonl` (JSON Lines)
- `.csv`
- `.json`
- `.parquet` (via DuckDB)

## Quick Start (Development)

### Requirements

- **Node.js**: Node 20 LTS (or newer LTS) recommended
- **Rust**: stable (recommended via `rustup`)
- **Tauri prerequisites**
  - macOS: install Xcode Command Line Tools (`xcode-select --install`)
  - Other platforms: see [Tauri Prerequisites](https://tauri.app/v1/guides/getting-started/prerequisites/)

### Install dependencies

```bash
cd apps/desktop
npm install
```

### Run (recommended)

The repo root provides a more robust dev launcher script `dev.sh` (checks ports, optionally cleans caches, optionally prebuilds frontend, and ensures child processes exit together).

```bash
./dev.sh
```

Common modes:

```bash
./dev.sh tauri   # default: runs tauri dev (includes vite dev)
./dev.sh vite    # vite-only (without tauri shell)
```

Optional env vars:

```bash
# Auto-kill when port is occupied (default: 1)
FORCE_KILL=0 ./dev.sh

# Skip frontend rebuild / cache clean before start (defaults: 1)
REBUILD_FRONTEND=0 CLEAN_FRONTEND=0 ./dev.sh
```

## Project layout (most-viewed paths)

- `apps/desktop/`
  - `src/`: SvelteKit UI (main screens + interactions)
  - `src/lib/ipc.ts`: frontend IPC wrapper (`invoke` + type defs)
  - `src-tauri/`: Tauri shell (Rust command layer)
- `core/`: Rust core engine (crate: `dh_core`)
- `dev.sh`: dev launcher script
- `deveploer/`: developer docs (entry: `deveploer/main.md`)
- `test/`: test records (see `EXAM.md`)

> Note: `apps/desktop/node_modules/`, `apps/desktop/src-tauri/target/`, etc. are build outputs / dependency caches. Avoid writing docs there, and do not treat changes there as source changes.

## IPC / Core capabilities (implemented)

Frontend calls Tauri commands via `apps/desktop/src/lib/ipc.ts` → `apps/desktop/src-tauri/src/commands.rs` → `core/src/engine.rs` (`CoreEngine`).

Implemented core interfaces (by capability):

- **Files / folders**
  - Open file: `open_file(path) -> { session, first_page }`
  - Paged read: `next_page(session_id, cursor, page_size) -> RecordPage`
  - Get raw record: `get_record_raw(session_id, meta) -> String`
  - Scan folder tree: `scan_folder_tree(path, max_depth, max_nodes)`
- **Search**
  - Current page: `mode = current_page`
  - Full scan task: `mode = scan_all` (returns `taskId`, supports `cancel_task`, results are pageable)
- **Export**
  - Export selection: `request = selection`
  - Export search-task results: `request = search_task`

## Roadmap

- **Indexed Search (M4)**: incremental indexing / faster cross-page navigation
- **Stats & column-level filtering (M3)**: schema inference, missing rate, TopK, DuckDB filtering / predicate pushdown
- **UX improvements**: resume, caching strategy, themes / shortcuts, file association open

## Testing & records

Test records live in `test/`. For each test run, follow `EXAM.md` and write to `test/main_record.md` (what you tested, results, focused issues, and version).

## License

The Tauri shell crate declares MIT (see `apps/desktop/src-tauri/Cargo.toml`). If you want to add a root-level `LICENSE` file, we can do it in a later version.