<script lang="ts">
  import { onMount, tick } from 'svelte';
  import { tooltip } from '$lib/actions/tooltip';
  import { clipboardWriteText, dialogOpen, dialogSave, eventListen, isTauri } from '$lib/platform';
  import JsonTree from '$lib/components/JsonTree.svelte';
  import JsonLazyTree from '$lib/components/JsonLazyTree.svelte';
  import JsonTreePicker, { type JsonPath } from '$lib/components/JsonTreePicker.svelte';
  import FileTreeNode from '$lib/components/FileTreeNode.svelte';
  import {
    cancelTask,
    exportToFile,
    getTask,
    getRecordRaw,
    nextPage,
    openFile,
    pathKind,
    scanFolderTree,
    search,
    searchTaskHitsPage,
    takePendingOpenPaths,
    type ExportFormat,
    type ExportRequest,
    type FsNode,
    type Record,
    type RecordPage,
    type SessionInfo,
    type Task
  } from '$lib/ipc';

  let isTauriEnv = false;
  const demoFiles = [
    { label: 'training_data.jsonl（示例）', path: 'demo://jsonl/training_data.jsonl' },
    { label: 'scores.csv（示例）', path: 'demo://csv/scores.csv' },
    { label: 'sample.json（示例）', path: 'demo://json/sample.json' }
  ];

  let session: SessionInfo | null = null;
  let page: RecordPage | null = null;
  let selected: Record | null = null;
  let checked = new Set<number>();
  // Selection for json_subtree mode (pseudo record ids within current subtree).
  // NOTE: Selections from "记录" and "检索结果" are merged into this single set.
  let checkedSubtree = new Set<number>();

  let pageSize = 10;
  // cursor history for prev/next navigation
  let pageCursorHistory: (string | null)[] = [null];
  let pageCursorIndex = 0;

  // Record-panel search (scope depends on recordViewMode):
  // - backend: full-file search via backend scan_all task
  // - json_subtree: local filter within current subtree records
  // Draft text: typing should NOT affect any state until user confirms (Enter/click).
  let recordSearchDraft = '';
  // Only enter "search results mode" after user explicitly starts a search.
  // This prevents the record list from going blank while the user is typing.
  let recordSearchActive = false;
  let recordSearchCommittedText = '';
  let recordSearchTask: Task | null = null;
  let recordSearchHits: RecordPage | null = null;
  let recordSearchHitsCursor: string | null = null;

  type RecordPanelTab = 'records' | 'search';
  let recordPanelTab: RecordPanelTab = 'records';

  // Local (json_subtree) search results over the ENTIRE selected subtree.
  let recordSubviewSearchAll: Record[] = [];
  let recordSubviewSearchPageIndex = 0;
  let recordSubviewSearchRangeText = '';
  let recordSubviewSearchEmptyMsg: string | null = null;

  // Detail-panel search (current detail root only; highlight + next/prev)
  let detailSearchText = '';
  let detailSearchDraft = '';
  let detailSearchHitsCount = 0;
  let detailSearchHitIndex = 0;
  let detailJsonViewEl: HTMLElement | null = null;
  let detailSearchToken = 0;
  let detailSearchCountToken = 0;
  let detailSearchCountBusy = false;
  let detailSearchCountErr: string | null = null;
  let detailSearchCountWorkers: Worker[] = [];

  let exportFormat: ExportFormat = 'jsonl';
  let exportModalOpen = false;

  let errorMsg: string | null = null;
  let busy = false;

  let detailText = '';
  let detailCharLen = 0;
  let detailJsonOk = false;
  let detailJsonValue: unknown = null;
  let detailJsonErr: string | null = null;
  let detailLoadingFull = false;
  let detailTruncated = false;
  let detailCanLoadFull = false;
  let detailTooLargeHint: string | null = null;
  let detailStreamMode = false;
  let detailFetchToken = 0;
  let detailDefaultExpand = true;
  let detailCopying = false;
  let detailCopied = false;
  let detailCopyErr: string | null = null;
  let detailCopiedTimer: number | null = null;

  // Record panel JSON focus (pick a subtree as the "record list source")
  type RecordViewMode = 'backend' | 'json_subtree';
  let recordViewMode: RecordViewMode = 'backend';
  let recordFocusModalOpen = false;
  let recordFocusPath: JsonPath | null = null; // null => root
  let recordFocusDraftPath: JsonPath = [];
  let recordFocusInvalid: string | null = null;
  let recordFocusRootValue: unknown = null; // snapshot of the backend/root JSON we picked from
  let recordFocusValue: unknown = null; // derived from recordFocusRootValue + recordFocusPath
  let recordFocusTotal = 0;
  let recordFocusPageIndex = 0;
  let recordSubviewRecords: Record[] = [];
  let recordSubviewEmptyMsg: string | null = null;
  let recordSubviewRangeText = '';
  let selectedBackend: Record | null = null;
  let lastSessionId: string | null = null;

  // Detail panel JSON focus (pick a subtree as the "detail view root")
  type DetailViewMode = 'root' | 'json_subtree';
  let detailViewMode: DetailViewMode = 'root';
  let detailFocusModalOpen = false;
  let detailFocusPath: JsonPath | null = null; // null => root
  let detailFocusDraftPath: JsonPath = [];
  let detailFocusInvalid: string | null = null;
  let detailFocusValue: unknown = null; // derived from detailJsonValue + detailFocusPath

  type OpenFileProgressPayload = { request_id: string; pct_0_100: number; stage: string };
  let openRequestId: string | null = null;
  let openPct: number | null = null;
  let openStage: string = '';

  // Default: emphasize detail panel by giving it more space.
  let splitPct = 40; // record vs detail width percent
  let splitEl: HTMLElement | null = null;

  // Left Session panel (default collapsed)
  let layoutEl: HTMLElement | null = null;
  let sidebarCollapsed = true;
  // Default: slightly narrower sidebar to keep focus on details.
  let sidebarWidth = 240; // px, used when expanded
  let recentFiles: string[] = [];
  let folderTreeRoot: FsNode | null = null;
  let folderTreeTruncated = false;
  let folderTreeTotalNodes = 0;
  let folderExpanded = new Set<string>(); // paths expanded in tree
  let folderSelectedPath: string | null = null;

  let sessionDropActive = false;

  function clamp(n: number, min: number, max: number) {
    return Math.min(max, Math.max(min, n));
  }

  onMount(() => {
    isTauriEnv = isTauri();

    const v = Number(localStorage.getItem('recordDetailSplitPct'));
    if (!Number.isNaN(v) && Number.isFinite(v)) splitPct = clamp(v, 20, 80);

    const w = Number(localStorage.getItem('sidebarWidthPx'));
    if (!Number.isNaN(w) && Number.isFinite(w)) sidebarWidth = clamp(w, 200, 520);

    const c = localStorage.getItem('sidebarCollapsed');
    if (c === '0') sidebarCollapsed = false;

    try {
      const raw = localStorage.getItem('recentFiles');
      if (raw) recentFiles = JSON.parse(raw);
    } catch {
      recentFiles = [];
    }

    try {
      const raw = localStorage.getItem('folderExpanded');
      if (raw) folderExpanded = new Set(JSON.parse(raw));
    } catch {
      folderExpanded = new Set();
    }

    // If the OS launched us with file(s) to open (e.g. double-click associated files),
    // fetch them once the UI is ready and open/scan accordingly.
    if (isTauriEnv) {
      (async () => {
        try {
          const paths = await takePendingOpenPaths();
          if (Array.isArray(paths) && paths.length > 0) {
            await handleDroppedPaths(paths);
          }
        } catch {
          // ignore
        }
      })();
    }
  });

  onMount(() => {
    let unlistenProgress: null | (() => void) = null;
    let unlistenOpenPaths: null | (() => void) = null;
    let unlistenFileDrop: null | (() => void) = null;
    let unlistenFileDropHover: null | (() => void) = null;
    let unlistenFileDropCancelled: null | (() => void) = null;
    if (!isTauri()) return () => {};
    (async () => {
      unlistenProgress = await eventListen<OpenFileProgressPayload>('open_file_progress', (e) => {
        if (!openRequestId) return;
        if (e.payload.request_id !== openRequestId) return;
        openPct = e.payload.pct_0_100;
        openStage = e.payload.stage;
      });

      // When the app is already running, macOS "Open With"/double-click can request us
      // to open additional files. Handle it by simulating a drop/open.
      unlistenOpenPaths = await eventListen<string[]>('open_paths', (e) => {
        const paths = e.payload;
        if (!Array.isArray(paths) || paths.length === 0) return;
        void handleDroppedPaths(paths);
      });

      // Tauri-native file drop events.
      // On macOS, HTML5 DataTransfer.files often does NOT expose real file paths,
      // but Tauri emits them via `tauri://file-drop`.
      unlistenFileDropHover = await eventListen<string[]>('tauri://file-drop-hover', (e) => {
        const paths = e.payload;
        if (!Array.isArray(paths) || paths.length === 0) return;
        sessionDropActive = true;
      });
      // `tauri://file-drop-cancelled` usually has no payload.
      unlistenFileDropCancelled = await eventListen<unknown>('tauri://file-drop-cancelled', (_e) => {
        sessionDropActive = false;
      });
      unlistenFileDrop = await eventListen<string[]>('tauri://file-drop', (e) => {
        const paths = e.payload;
        sessionDropActive = false;
        if (!Array.isArray(paths) || paths.length === 0) return;
        void handleDroppedPaths(paths);
      });
    })();
    return () => {
      unlistenProgress?.();
      unlistenOpenPaths?.();
      unlistenFileDrop?.();
      unlistenFileDropHover?.();
      unlistenFileDropCancelled?.();
    };
  });

  function setSplitPct(next: number) {
    splitPct = clamp(next, 20, 80);
    localStorage.setItem('recordDetailSplitPct', String(splitPct));
  }

  function onSplitterPointerDown(e: PointerEvent) {
    if (!splitEl) return;
    const rect = splitEl.getBoundingClientRect();
    const startX = e.clientX;
    const startPct = splitPct;
    const width = rect.width || 1;

    const target = e.currentTarget as HTMLElement | null;
    target?.setPointerCapture?.(e.pointerId);

    const onMove = (ev: PointerEvent) => {
      const dx = ev.clientX - startX;
      const next = startPct + (dx / width) * 100;
      setSplitPct(next);
    };
    const onUp = () => {
      document.body.classList.remove('dragging-split');
      window.removeEventListener('pointermove', onMove);
      window.removeEventListener('pointerup', onUp);
    };

    document.body.classList.add('dragging-split');
    window.addEventListener('pointermove', onMove);
    window.addEventListener('pointerup', onUp);
  }

  function onSplitterKeyDown(e: KeyboardEvent) {
    if (e.key === 'ArrowLeft') {
      e.preventDefault();
      setSplitPct(splitPct - 2);
    } else if (e.key === 'ArrowRight') {
      e.preventDefault();
      setSplitPct(splitPct + 2);
    } else if (e.key === 'Home') {
      e.preventDefault();
      setSplitPct(20);
    } else if (e.key === 'End') {
      e.preventDefault();
      setSplitPct(80);
    }
  }

  function setSidebarCollapsed(next: boolean) {
    sidebarCollapsed = next;
    localStorage.setItem('sidebarCollapsed', next ? '1' : '0');
  }

  function setSidebarWidthPx(next: number) {
    sidebarWidth = clamp(next, 200, 520);
    localStorage.setItem('sidebarWidthPx', String(sidebarWidth));
  }

  function newRequestId() {
    // eslint-disable-next-line @typescript-eslint/no-explicit-any
    return (globalThis as any).crypto?.randomUUID?.() ?? `${Date.now()}-${Math.random().toString(16).slice(2)}`;
  }

  async function openFilePath(path: string) {
    if (!path) return;
    errorMsg = null;
    busy = true;
    openRequestId = newRequestId();
    // Only show progress UI if backend emits progress events (large files).
    openPct = null;
    openStage = '';
    try {
      const res = await openFile(path, openRequestId);
      session = res.session;
      page = res.first_page;
      selected = page.records[0] ?? null;
      checked = new Set();
      checkedSubtree = new Set();
      pageCursorHistory = [null];
      pageCursorIndex = 0;
      recordSearchTask = null;
      recordSearchHits = null;
      recordSearchHitsCursor = null;
      recordSearchDraft = '';
      recordSearchActive = false;
      recordSearchCommittedText = '';
      recordSubviewSearchAll = [];
      recordSubviewSearchPageIndex = 0;
      recordSubviewSearchRangeText = '';
      recordSubviewSearchEmptyMsg = null;

      // keep a small recent list for the Session panel
      recentFiles = [path, ...recentFiles.filter((p) => p !== path)].slice(0, 12);
      localStorage.setItem('recentFiles', JSON.stringify(recentFiles));
    } catch (e: any) {
      errorMsg = String(e);
    } finally {
      busy = false;
      openRequestId = null;
      openPct = null;
      openStage = '';
    }
  }

  async function scanFolderPath(path: string) {
    if (!path) return;
    errorMsg = null;
    busy = true;
    try {
      const tree = await scanFolderTree({ path, max_depth: 64, max_nodes: 20_000 });
      folderTreeRoot = tree.root;
      folderTreeTruncated = tree.truncated;
      folderTreeTotalNodes = tree.total_nodes;

      // default: expand root
      const nextExpanded = new Set(folderExpanded);
      nextExpanded.add(tree.root.path);
      folderExpanded = nextExpanded;
      localStorage.setItem('folderExpanded', JSON.stringify(Array.from(folderExpanded)));
    } catch (e: any) {
      errorMsg = String(e);
    } finally {
      busy = false;
    }
  }

  async function handleDroppedPaths(paths: string[]) {
    const uniq = Array.from(new Set(paths.map((p) => p?.trim()).filter(Boolean))) as string[];
    if (uniq.length === 0) return;

    errorMsg = null;

    // Prefer: first folder -> show tree; first file -> open session.
    let firstDir: string | null = null;
    let firstFile: string | null = null;
    for (const p of uniq) {
      try {
        const k = await pathKind(p);
        if (k === 'dir' && !firstDir) firstDir = p;
        if (k === 'file' && !firstFile) firstFile = p;
      } catch {
        // ignore classification errors; user will get a proper error when we try open/scan.
      }
      if (firstDir && firstFile) break;
    }

    if (firstDir) await scanFolderPath(firstDir);
    if (firstFile) await openFilePath(firstFile);

    if (!firstDir && !firstFile) {
      // Fall back: try to open the first path as a file.
      await openFilePath(uniq[0]);
    }
  }

  function onSessionDragOver(e: DragEvent) {
    e.preventDefault();
    sessionDropActive = true;
  }

  function onSessionDragLeave(e: DragEvent) {
    if (e.currentTarget === e.target) sessionDropActive = false;
  }

  async function onSessionDrop(e: DragEvent) {
    e.preventDefault();
    sessionDropActive = false;

    const files = Array.from(e.dataTransfer?.files ?? []);
    // In Tauri, File objects often contain an extra `path` field.
    const paths = files
      // eslint-disable-next-line @typescript-eslint/no-explicit-any
      .map((f) => (f as any)?.path as string | undefined)
      .filter(Boolean) as string[];

    if (paths.length === 0) {
      errorMsg = '拖拽失败：未能获取路径（请改用“打开文件/文件夹”按钮）。';
      return;
    }
    await handleDroppedPaths(paths);
  }

  function onSidebarResizerPointerDown(e: PointerEvent) {
    if (!layoutEl) return;
    if (sidebarCollapsed) setSidebarCollapsed(false);

    const rect = layoutEl.getBoundingClientRect();
    const startX = e.clientX;
    const startW = sidebarWidth;
    const maxW = Math.min(520, Math.max(240, rect.width * 0.6));

    const target = e.currentTarget as HTMLElement | null;
    target?.setPointerCapture?.(e.pointerId);

    const onMove = (ev: PointerEvent) => {
      const dx = ev.clientX - startX;
      setSidebarWidthPx(clamp(startW + dx, 200, maxW));
    };
    const onUp = () => {
      document.body.classList.remove('dragging-split');
      window.removeEventListener('pointermove', onMove);
      window.removeEventListener('pointerup', onUp);
    };

    document.body.classList.add('dragging-split');
    window.addEventListener('pointermove', onMove);
    window.addEventListener('pointerup', onUp);
  }

  async function onPickFile() {
    if (!isTauriEnv) {
      errorMsg = '当前为 Web 测试模式：无法打开系统文件对话框，请使用“打开示例数据”。';
      return;
    }
    errorMsg = null;
    let picked: string | null = null;
    try {
      const res = await dialogOpen({ multiple: false, directory: false });
      if (!res || Array.isArray(res)) return;
      picked = res;
    } catch (e: any) {
      errorMsg = `打开文件对话框失败：${String(e)}`;
      return;
    }
    await openFilePath(picked);
  }


  async function onPickFolder() {
    if (!isTauriEnv) {
      errorMsg = '当前为 Web 测试模式：无法打开系统文件夹对话框，请使用“打开示例文件夹”。';
      return;
    }
    errorMsg = null;
    let picked: string | null = null;
    try {
      const res = await dialogOpen({ multiple: false, directory: true });
      if (!res || Array.isArray(res)) return;
      picked = res;
    } catch (e: any) {
      errorMsg = `打开文件夹对话框失败：${String(e)}`;
      return;
    }
    await scanFolderPath(picked);
  }

  function toggleFolder(path: string) {
    const next = new Set(folderExpanded);
    if (next.has(path)) next.delete(path);
    else next.add(path);
    folderExpanded = next;
    localStorage.setItem('folderExpanded', JSON.stringify(Array.from(folderExpanded)));
  }

  async function onTreeFileClick(node: FsNode) {
    folderSelectedPath = node.path;
    if (!node.supported) {
      errorMsg = `不支持的文件格式：${node.name}`;
      return;
    }
    await onOpenRecent(node.path);
  }

  async function loadPageAtCursor(cursor: string | null) {
    if (!session) return;
    errorMsg = null;
    busy = true;
    try {
      page = await nextPage({
        session_id: session.session_id,
        cursor,
        page_size: pageSize
      });
      selected = page.records[0] ?? null;
      checked = new Set();
      checkedSubtree = new Set();
    } catch (e: any) {
      errorMsg = String(e);
    } finally {
      busy = false;
    }
  }

  async function onPrevPage() {
    if (!session) return;
    if (pageCursorIndex <= 0) return;
    pageCursorIndex -= 1;
    await loadPageAtCursor(pageCursorHistory[pageCursorIndex] ?? null);
  }

  async function onNextPage() {
    if (!session || !page) return;
    // If we previously went back, allow "next" to move forward in history without requiring next_cursor.
    if (pageCursorIndex < pageCursorHistory.length - 1) {
      pageCursorIndex += 1;
      await loadPageAtCursor(pageCursorHistory[pageCursorIndex] ?? null);
      return;
    }
    if (!page.next_cursor) return;
    pageCursorHistory = [...pageCursorHistory, page.next_cursor];
    pageCursorIndex += 1;
    await loadPageAtCursor(page.next_cursor);
  }

  async function onOpenRecent(path: string) {
    await openFilePath(path);
  }

  function toggleChecked(id: number) {
    const next = new Set(checked);
    if (next.has(id)) next.delete(id);
    else next.add(id);
    checked = next;
  }

  function toggleCheckedSubtree(id: number) {
    const next = new Set(checkedSubtree);
    if (next.has(id)) next.delete(id);
    else next.add(id);
    checkedSubtree = next;
  }

  function addMany(set0: Set<number>, ids: number[]) {
    const next = new Set(set0);
    for (const id of ids) next.add(id);
    return next;
  }

  function removeMany(set0: Set<number>, ids: number[]) {
    const next = new Set(set0);
    for (const id of ids) next.delete(id);
    return next;
  }

  function eventChecked(e: Event): boolean {
    // Svelte template expressions don't like TS `as` assertions; keep it here.
    const el = e.currentTarget as unknown as { checked?: boolean } | null;
    return Boolean(el?.checked);
  }

  async function onRecordSearch() {
    if (!session) return;
    const q = recordSearchDraft.trim();
    if (!q) return;
    try {
      // Enter "search results mode" only after explicit confirm (click/Enter).
      // This prevents reactive filtering while typing.
      recordSearchActive = true;
      recordSearchCommittedText = q;
      recordSubviewSearchPageIndex = 0;
      recordPanelTab = 'search';

      if (recordViewMode === 'json_subtree') {
        // Local filter only; no backend task.
        return;
      }

      errorMsg = null;
      busy = true;
      recordSearchTask = null;
      recordSearchHits = null;
      recordSearchHitsCursor = null;

      // Backend full-file search (scan_all) supports:
      // - jsonl/csv: line-based scan
      // - json: root-array item scan
      // - parquet: DuckDB row scan
      const mode =
        session.format === 'jsonl' ||
        session.format === 'csv' ||
        session.format === 'json' ||
        session.format === 'parquet'
          ? 'scan_all'
          : 'current_page';
      const res = await search({
        session_id: session.session_id,
        query: {
          text: q,
          mode,
          case_sensitive: false,
          max_hits: 10_000
        }
      });

      if (res.mode === 'scan_all' && res.task) {
        await pollRecordSearch(res.task.id);
      } else {
        // Some environments may return direct hits even for scan_all-like queries.
        recordSearchHits = { records: res.hits, next_cursor: null, reached_eof: true };
        recordSearchHitsCursor = null;
        recordSearchTask = res.task ? await getTask(res.task.id) : null;
      }
    } catch (e: any) {
      errorMsg = String(e);
    } finally {
      if (recordViewMode !== 'json_subtree') {
        busy = false;
      }
    }
  }

  async function pollRecordSearch(taskId: string) {
    recordSearchHits = null;
    recordSearchHitsCursor = null;

    // quick poll loop
    for (let i = 0; i < 600; i++) {
      const t = await getTask(taskId);
      recordSearchTask = t;
      if (t.finished) break;
      await new Promise((r) => setTimeout(r, 200));
    }

    // load first page of hits (even if unfinished, allow partial preview)
    const hits = await searchTaskHitsPage({ task_id: taskId, cursor: null, page_size: pageSize });
    recordSearchHits = hits;
    recordSearchHitsCursor = hits.next_cursor ?? null;
  }

  async function onMoreRecordSearchHits() {
    if (!recordSearchTask) return;
    if (!recordSearchHitsCursor) return;
    const hits = await searchTaskHitsPage({
      task_id: recordSearchTask.id,
      cursor: recordSearchHitsCursor,
      page_size: pageSize
    });
    recordSearchHits = {
      records: [...(recordSearchHits?.records ?? []), ...hits.records],
      next_cursor: hits.next_cursor,
      reached_eof: hits.reached_eof
    };
    recordSearchHitsCursor = hits.next_cursor ?? null;
  }

  async function onCancelRecordSearch() {
    if (!recordSearchTask) return;
    errorMsg = null;
    try {
      await cancelTask(recordSearchTask.id);
      recordSearchTask = await getTask(recordSearchTask.id);
    } catch (e: any) {
      errorMsg = String(e);
    }
  }

  function clearRecordSearch() {
    recordSearchDraft = '';
    recordSearchActive = false;
    recordSearchCommittedText = '';
    recordSearchTask = null;
    recordSearchHits = null;
    recordSearchHitsCursor = null;
    recordSubviewSearchAll = [];
    recordSubviewSearchPageIndex = 0;
    recordSubviewSearchRangeText = '';
    recordSubviewSearchEmptyMsg = null;
    recordPanelTab = 'records';
  }

  function clearDetailSearch() {
    detailSearchText = '';
    detailSearchDraft = '';
    detailSearchHitsCount = 0;
    detailSearchHitIndex = 0;
    detailSearchToken++;
    detailSearchCountToken++;
    detailSearchCountBusy = false;
    detailSearchCountErr = null;
    for (const w of detailSearchCountWorkers) w.terminate();
    detailSearchCountWorkers = [];
  }

  function confirmDetailSearch() {
    if (busy) return;
    // Only start searching/highlighting after explicit confirm (Enter/click).
    detailSearchText = detailSearchDraft.trim();
    refreshDetailSearchHits(true);
    startDetailSearchCount();
  }

  async function startDetailSearchCount() {
    const token = ++detailSearchCountToken;
    detailSearchCountErr = null;
    for (const w of detailSearchCountWorkers) w.terminate();
    detailSearchCountWorkers = [];

    const q = detailSearchText.trim();
    if (!q) return;
    if (!selected) return;
    if (!detailJsonOk) return;
    // Only enable parallel counting when everything is expanded; otherwise DOM doesn't contain all marks.
    if (!detailDefaultExpand) return;

    const text = detailText ?? '';
    if (!text) return;

    // Heuristic: avoid worker overhead for small strings.
    if (text.length < 50_000) return;

    detailSearchCountBusy = true;
    try {
      const concurrency = Math.max(1, Math.min(10, Number((globalThis as any).navigator?.hardwareConcurrency ?? 4)));
      const workersN = Math.max(1, concurrency);
      const overlap = Math.max(0, q.length - 1);
      const chunkSize = Math.ceil(text.length / workersN);

      const promises: Promise<number>[] = [];
      for (let i = 0; i < workersN; i++) {
        const baseStart = i * chunkSize;
        if (baseStart >= text.length) break;
        const limitStartInFullText = Math.min(text.length, (i + 1) * chunkSize);
        const end = Math.min(text.length, limitStartInFullText + overlap);
        const slice = text.slice(baseStart, end);

        const w = new Worker(new URL('$lib/workers/text_search.worker.ts', import.meta.url), { type: 'module' });
        detailSearchCountWorkers = [...detailSearchCountWorkers, w];
        promises.push(
          new Promise<number>((resolve, reject) => {
            w.onmessage = (ev) => resolve(Number((ev.data as any).count ?? 0));
            w.onerror = (e) => reject(e);
            w.postMessage({ text: slice, query: q, caseSensitive: false, baseStart, limitStartInFullText });
          })
        );
      }

      const counts = await Promise.all(promises);
      if (token !== detailSearchCountToken) return;

      const total = counts.reduce((a, b) => a + b, 0);
      // Use as a fast approximation; navigation still uses DOM marks when jumping.
      detailSearchHitsCount = total;
      if (total === 0) detailSearchHitIndex = 0;
    } catch (e: any) {
      if (token !== detailSearchCountToken) return;
      detailSearchCountErr = String(e?.message ?? e);
    } finally {
      if (token !== detailSearchCountToken) return;
      detailSearchCountBusy = false;
      for (const w of detailSearchCountWorkers) w.terminate();
      detailSearchCountWorkers = [];
    }
  }

  async function refreshDetailSearchHits(scrollToFirst: boolean) {
    const token = ++detailSearchToken;
    await tick();
    if (token !== detailSearchToken) return;
    // If we've already computed hit count via workers (expanded-all), avoid expensive DOM scan here.
    // We only need the first hit to scroll into view.
    const first = (detailJsonViewEl?.querySelector?.('mark.jt-hit') as HTMLElement | null) ?? null;
    if (!first) {
      // Fall back to DOM count when not in expanded-all mode, or when we haven't computed count.
      const marks = Array.from(detailJsonViewEl?.querySelectorAll?.('mark.jt-hit') ?? []) as HTMLElement[];
      detailSearchHitsCount = marks.length;
      detailSearchHitIndex = 0;
      return;
    }

    if (scrollToFirst) {
      detailSearchHitIndex = detailSearchHitsCount > 0 ? 1 : 0;
      first.scrollIntoView?.({ block: 'center' });
      return;
    }

    // Clamp index (best-effort; accurate count is refreshed on jump).
    if (detailSearchHitsCount <= 0) {
      detailSearchHitIndex = 0;
    } else {
      if (detailSearchHitIndex <= 0) detailSearchHitIndex = 1;
      if (detailSearchHitIndex > detailSearchHitsCount) detailSearchHitIndex = detailSearchHitsCount;
    }
  }

  function jumpDetailHit(delta: number) {
    const marks = Array.from(detailJsonViewEl?.querySelectorAll?.('mark.jt-hit') ?? []) as HTMLElement[];
    if (marks.length === 0) return;
    // Ensure count is accurate for navigation.
    detailSearchHitsCount = marks.length;
    let idx0 = detailSearchHitIndex - 1;
    if (idx0 < 0 || idx0 >= marks.length) idx0 = 0;
    idx0 = (idx0 + delta + marks.length) % marks.length;
    detailSearchHitIndex = idx0 + 1;
    marks[idx0]?.scrollIntoView?.({ block: 'center' });
  }

  async function onExport() {
    if (!session) return;
    errorMsg = null;

    let request: ExportRequest;
    if (recordViewMode === 'backend') {
      const ids = Array.from(checked.values());
      if (ids.length === 0) {
        errorMsg = '未选择任何记录（可在“记录/检索结果”中勾选）。';
        return;
      }
      request = { type: 'selection', record_ids: ids };
    } else {
      // json_subtree: export the selected subtree children; if subtree is a leaf, export the subtree itself.
      if (exportFormat === 'csv') {
        errorMsg = '子树导出仅支持 json/jsonl。';
        return;
      }
      if (!selectedBackend?.meta) {
        errorMsg = '当前记录缺少定位信息（meta），无法导出子树。';
        return;
      }
      const path = recordFocusPath ? [...recordFocusPath] : [];
      const v = recordFocusValue;
      const picked = Array.from(checkedSubtree.values()).sort((a, b) => a - b);

      // leaf => export root value
      if (!Array.isArray(v) && !(v && typeof v === 'object')) {
        request = { type: 'json_subtree', meta: selectedBackend.meta, path, include_root: true, children: [] };
      } else {
        if (picked.length === 0) {
          errorMsg = '未选择任何子项（可在“记录/检索结果”中勾选，或使用“全选”）。';
          return;
        }
        let children: (string | number)[] = [];
        if (Array.isArray(v)) {
          children = picked;
        } else {
          const keys = Object.keys(v as object);
          children = picked.map((i) => keys[i]).filter((x) => typeof x === 'string') as string[];
        }
        if (children.length === 0) {
          errorMsg = '所选节点没有可导出的子项。';
          return;
        }
        request = { type: 'json_subtree', meta: selectedBackend.meta, path, include_root: false, children };
      }
    }

    const ext = exportFormat === 'csv' ? 'csv' : exportFormat === 'json' ? 'json' : 'jsonl';
    let out: string | null = null;
    try {
      const res = await dialogSave({
        defaultPath: `export.${ext}`
      });
      if (!res) return;
      out = res;
    } catch (e: any) {
      errorMsg = `保存文件对话框失败：${String(e)}`;
      return;
    }

    busy = true;
    try {
      await exportToFile({
        session_id: session.session_id,
        request,
        format: exportFormat,
        output_path: out
      });
      exportModalOpen = false;
    } catch (e: any) {
      errorMsg = String(e);
    } finally {
      busy = false;
    }
  }

  function isTooLargeErrMessage(msg: string) {
    const m = (msg || '').toLowerCase();
    return m.includes('json value too large') || m.includes('record too large') || m.includes('max 52428800');
  }

  async function onLoadFullDetail() {
    if (!session?.session_id) return;
    if (!selected?.meta) return;
    if (!detailTruncated) return;
    if (!detailCanLoadFull) return;

    const token = ++detailFetchToken;
    detailLoadingFull = true;
    detailJsonErr = null;
    try {
      const full = await getRecordRaw({ session_id: session.session_id, meta: selected.meta });
      if (token !== detailFetchToken) return;

      detailText = full;
      detailCharLen = Array.from(full).length;

      const fmt = session?.format;
      const shouldTryParse = Boolean(selected && fmt && isJsonLikeFormat(fmt));
      if (shouldTryParse) {
        detailJsonOk = false;
        detailJsonValue = null;
        detailJsonErr = null;
        try {
          detailJsonValue = JSON.parse(full);
          detailJsonOk = true;
        } catch (e: any) {
          detailJsonErr = String(e?.message ?? e);
        }
      }

      // 成功加载完整内容后，更新截断状态
      detailTruncated = false;
      detailCanLoadFull = false;
    } catch (e: any) {
      if (token !== detailFetchToken) return;
      const msg = String(e?.message ?? e);
      if (isTooLargeErrMessage(msg)) {
        detailJsonErr = '记录过大：无法在详情中加载完整内容（超过 IPC 上限）。请使用“导出本条记录”查看原文。';
      } else {
        detailJsonErr = `获取完整记录失败：${msg}`;
      }
    } finally {
      if (token !== detailFetchToken) return;
      detailLoadingFull = false;
    }
  }

  async function onExportCurrentRecord() {
    if (!session) return;
    if (!selected?.meta) {
      errorMsg = '当前选择不是原始记录（缺少定位信息 meta），无法直接导出本条。';
      return;
    }

    errorMsg = null;
    const fmt: ExportFormat = session.format === 'csv' ? 'csv' : 'jsonl';
    const ext = fmt === 'csv' ? 'csv' : 'jsonl';

    let out: string | null = null;
    try {
      const res = await dialogSave({
        defaultPath: `record_${selected.id}.${ext}`
      });
      if (!res) return;
      out = res;
    } catch (e: any) {
      errorMsg = `保存文件对话框失败：${String(e)}`;
      return;
    }

    busy = true;
    try {
      await exportToFile({
        session_id: session.session_id,
        request: { type: 'selection', record_ids: [selected.id] },
        format: fmt,
        output_path: out
      });
    } catch (e: any) {
      errorMsg = String(e);
    } finally {
      busy = false;
    }
  }

  function onKeyDownGlobal(e: KeyboardEvent) {
    if (e.key !== 'Escape') return;
    if (exportModalOpen) {
      e.preventDefault();
      exportModalOpen = false;
      return;
    }
    if (recordFocusModalOpen) {
      e.preventDefault();
      recordFocusModalOpen = false;
      return;
    }
  }

  function isJsonLikeFormat(fmt: string | undefined) {
    return fmt === 'json' || fmt === 'jsonl' || fmt === 'csv' || fmt === 'parquet';
  }

  function pathToLabel(p: JsonPath | null) {
    if (!p || p.length === 0) return '（根）';
    let out = '';
    for (const seg of p) {
      if (typeof seg === 'number') out += `[${seg}]`;
      else out += (out ? '.' : '') + seg;
    }
    return out;
  }

  function getAtPath(root: unknown, p: JsonPath): { ok: true; value: unknown } | { ok: false; error: string } {
    let cur: unknown = root;
    for (const seg of p) {
      if (typeof seg === 'number') {
        if (!Array.isArray(cur)) return { ok: false, error: '路径指向数组索引，但当前节点不是数组。' };
        cur = cur[seg];
        continue;
      }
      if (!cur || typeof cur !== 'object' || Array.isArray(cur)) return { ok: false, error: '路径指向对象键，但当前节点不是对象。' };
      cur = (cur as { [k: string]: unknown })[seg];
    }
    return { ok: true, value: cur };
  }

  function truncateText(s: string, max = 140) {
    if (s.length <= max) return s;
    return s.slice(0, max) + '…';
  }

  function stripQuotes(s: string) {
    const t = s.trim();
    if ((t.startsWith('"') && t.endsWith('"')) || (t.startsWith("'") && t.endsWith("'"))) return t.slice(1, -1);
    return t;
  }

  function parseKeyValueQuery(q: string): { key: string; value: string } | null {
    const t = q.trim();
    const idx = t.indexOf(':');
    if (idx <= 0) return null;
    const k = t.slice(0, idx).trim();
    const v = t.slice(idx + 1).trim();
    if (!k || !v) return null;
    return { key: stripQuotes(k), value: stripQuotes(v) };
  }

  // Match logic aligned with detail-panel highlighting (`JsonTree`):
  // - Supports `key:value` by requiring BOTH key and value to appear (value allows quoted/unquoted).
  // - For non key:value, supports quoted/unquoted.
  function textMatches(haystack: string, needle: string, caseSensitive: boolean) {
    const q0 = needle.trim();
    if (!q0) return true;

    const hs = caseSensitive ? haystack : haystack.toLowerCase();
    const has = (n: string) => {
      const nn = n.trim();
      if (!nn) return false;
      const nn1 = caseSensitive ? nn : nn.toLowerCase();
      return hs.includes(nn1);
    };

    const kv = parseKeyValueQuery(q0);
    if (kv) {
      const key = kv.key;
      const value = kv.value;
      const keyOk = has(key) || has(JSON.stringify(key));
      const valOk = has(value) || has(JSON.stringify(value));
      return keyOk && valOk;
    }

    return has(q0) || has(JSON.stringify(q0));
  }

  function safePrettyJson(v: unknown) {
    try {
      return JSON.stringify(
        v,
        (_k, vv) => (typeof vv === 'bigint' ? vv.toString() : vv),
        2
      );
    } catch {
      // Fallback: keep something copyable even if value is not JSON-serializable.
      return String(v);
    }
  }

  async function copyTextToClipboard(text: string) {
    try {
      await clipboardWriteText(text);
      return;
    } catch {
      // ignore, fallback to web APIs
    }

    try {
      await globalThis.navigator?.clipboard?.writeText?.(text);
      return;
    } catch {
      // ignore, fallback to execCommand
    }

    const ta = document.createElement('textarea');
    ta.value = text;
    ta.setAttribute('readonly', 'true');
    ta.style.position = 'fixed';
    ta.style.left = '-9999px';
    ta.style.top = '0';
    document.body.appendChild(ta);
    ta.select();
    try {
      document.execCommand('copy');
    } finally {
      document.body.removeChild(ta);
    }
  }

  async function onCopyDetailJson() {
    if (!detailJsonOk) return;
    detailCopyErr = null;
    detailCopied = false;
    if (detailCopiedTimer) window.clearTimeout(detailCopiedTimer);
    detailCopiedTimer = null;

    const v = detailViewMode === 'json_subtree' ? detailFocusValue : detailJsonValue;
    const text = safePrettyJson(v) + '\n';

    detailCopying = true;
    try {
      await copyTextToClipboard(text);
      detailCopied = true;
      detailCopiedTimer = window.setTimeout(() => {
        detailCopied = false;
        detailCopiedTimer = null;
      }, 1200);
    } catch (e: any) {
      detailCopyErr = String(e?.message ?? e ?? '复制失败');
    } finally {
      detailCopying = false;
    }
  }

  function previewForJsonValue(v: unknown): string {
    if (v === null) return 'null';
    if (Array.isArray(v)) return `[Array(${v.length})]`;
    const t = typeof v;
    if (t === 'string') return truncateText(JSON.stringify(v), 160);
    if (t === 'number' || t === 'boolean') return String(v);
    if (t === 'object') {
      const keys = Object.keys(v as object);
      const head = keys.slice(0, 6).join(', ');
      return `{Object(${keys.length})${head ? `: ${head}${keys.length > 6 ? ', …' : ''}` : ''}}`;
    }
    return String(v);
  }

  function makePseudoRecord(id: number, label: string, value: unknown): Record {
    const raw = (() => {
      try {
        return JSON.stringify(value);
      } catch {
        return String(value);
      }
    })();
    const preview = label ? `${label} ${previewForJsonValue(value)}` : previewForJsonValue(value);
    return { id, preview, raw, meta: null };
  }

  function openRecordFocusModal() {
    if (!detailJsonOk) return;
    recordFocusDraftPath = recordFocusPath ? [...recordFocusPath] : [];
    recordFocusModalOpen = true;
  }

  function confirmRecordFocus() {
    recordFocusPath = recordFocusDraftPath.length === 0 ? null : [...recordFocusDraftPath];
    recordFocusPageIndex = 0;
    // Switch record list to subtree mode once user confirmed.
    if (session?.format === 'json') {
      selectedBackend = selected;
      recordFocusRootValue = detailJsonValue; // IMPORTANT: snapshot the current parsed root, decouple from later selections
      recordViewMode = 'json_subtree';
      checkedSubtree = new Set();
      // Scope changed: clear backend search hits/task to avoid mixing scopes.
      recordSearchActive = false;
      recordSearchCommittedText = '';
      recordSearchTask = null;
      recordSearchHits = null;
      recordSearchHitsCursor = null;
      // Clear selection so detail waits for user click (avoids showing full root and confusing "联动没生效")
      selected = null;
    }
    recordFocusModalOpen = false;
  }

  function exitJsonSubview() {
    recordViewMode = 'backend';
    recordFocusPath = null;
    recordFocusInvalid = null;
    recordFocusRootValue = null;
    recordFocusValue = null;
    recordFocusTotal = 0;
    recordFocusPageIndex = 0;
    // Leaving subtree: clear local filter/search state.
    recordSearchActive = false;
    recordSearchCommittedText = '';
    recordSearchTask = null;
    recordSearchHits = null;
    recordSearchHitsCursor = null;
    // restore backend selection
    selected = selectedBackend ?? page?.records?.[0] ?? null;
    selectedBackend = null;
    checkedSubtree = new Set();
  }

  function openDetailFocusModal() {
    if (!detailJsonOk) return;
    detailFocusDraftPath = detailFocusPath ? [...detailFocusPath] : [];
    detailFocusModalOpen = true;
  }

  function confirmDetailFocus() {
    detailFocusPath = detailFocusDraftPath.length === 0 ? null : [...detailFocusDraftPath];
    detailViewMode = 'json_subtree';
    detailFocusModalOpen = false;
  }

  function exitDetailSubview() {
    detailViewMode = 'root';
    detailFocusPath = null;
    detailFocusInvalid = null;
    detailFocusValue = null;
  }

  $: {
    const token = ++detailFetchToken;
    detailLoadingFull = false;
    detailTruncated = false;
    detailCanLoadFull = false;
    detailTooLargeHint = null;
    detailStreamMode = false;

    const fmt = session?.format;
    const baseText = selected?.raw ?? selected?.preview ?? '';
    detailText = baseText;
    detailCharLen = Array.from(baseText).length;

    // Parse based on *baseText* (important: do not depend on `detailText` to avoid reactive loops)
    detailJsonOk = false;
    detailJsonValue = null;
    detailJsonErr = null;
    const shouldTryParse = Boolean(selected && fmt && isJsonLikeFormat(fmt));
    let baseParsedOk = false;
    if (shouldTryParse) {
      try {
        detailJsonValue = JSON.parse(baseText);
        detailJsonOk = true;
        baseParsedOk = true;
      } catch (e: any) {
        detailJsonErr = String(e?.message ?? e);
      }
    }

    // If the raw text looks truncated (ends with our ellipsis marker),
    // we DO NOT auto-fetch full content anymore (can exceed IPC limits and freeze UI).
    // Instead, expose a manual "加载完整内容" action when it's safe.
    detailTruncated = Boolean(selected?.raw && baseText.endsWith('…'));
    const metaLen = selected?.meta?.byte_len ?? 0;
    // Keep a safety margin under Tauri's ~50MB IPC cap.
    const IPC_SAFE_MAX_BYTES = 45 * 1024 * 1024;
    if (detailTruncated && selected?.meta && session?.session_id) {
      if (metaLen > 0 && metaLen > IPC_SAFE_MAX_BYTES) {
        detailCanLoadFull = false;
        detailTooLargeHint = `记录过大（约 ${Math.ceil(metaLen / (1024 * 1024))}MB），无法在详情中加载完整内容。`;
        // For huge JSON/JSONL records, enable streaming tree mode by default.
        if (session?.format === 'json' || session?.format === 'jsonl') {
          detailStreamMode = true;
        }
      } else {
        detailCanLoadFull = true;
      }
    }
  }

  // Reset json-subtree view when switching session/file, and derive the focused value from the snapshotted root value.
  $: {
    const sid = session?.session_id ?? null;
    if (sid !== lastSessionId) {
      lastSessionId = sid;
      recordFocusPath = null;
      recordFocusInvalid = null;
      recordFocusDraftPath = [];
      recordFocusModalOpen = false;
      recordViewMode = 'backend';
      recordFocusPageIndex = 0;
      recordFocusTotal = 0;
      recordFocusRootValue = null;
      recordSearchActive = false;
      recordSearchCommittedText = '';

      detailViewMode = 'root';
      detailFocusPath = null;
      detailFocusInvalid = null;
      detailFocusDraftPath = [];
      detailFocusModalOpen = false;
    }

    recordFocusInvalid = null;
    // If we're in subtree mode, use the snapshotted root value; otherwise don't derive anything.
    const root = recordViewMode === 'json_subtree' ? recordFocusRootValue : null;
    if (recordViewMode !== 'json_subtree' || root === null || root === undefined) {
      recordFocusValue = null;
      recordFocusTotal = 0;
    } else {
      if (!recordFocusPath || recordFocusPath.length === 0) {
        recordFocusValue = root;
      } else {
        const r = getAtPath(root, recordFocusPath);
        if ('error' in r) {
          // Fallback to root so UI doesn't go blank.
          recordFocusValue = root;
          recordFocusInvalid = r.error;
        } else {
          recordFocusValue = r.value;
        }
      }
    }

    // total size for local paging
    const v = recordFocusValue;
    if (Array.isArray(v)) recordFocusTotal = v.length;
    else if (v && typeof v === 'object') recordFocusTotal = Object.keys(v as object).length;
    else recordFocusTotal = v === null || v === undefined ? 0 : 1;

    // clamp page index
    const maxPage = recordFocusTotal > 0 ? Math.max(0, Math.ceil(recordFocusTotal / pageSize) - 1) : 0;
    if (recordFocusPageIndex > maxPage) recordFocusPageIndex = maxPage;
  }

  // Derive detail focus value from current parsed JSON (works for json/jsonl/csv/parquet if detailJsonOk).
  $: {
    detailFocusInvalid = null;
    if (!detailJsonOk || detailViewMode !== 'json_subtree') {
      detailFocusValue = null;
    } else {
      const root = detailJsonValue;
      if (!detailFocusPath || detailFocusPath.length === 0) {
        detailFocusValue = root;
      } else {
        const r = getAtPath(root, detailFocusPath);
        if ('error' in r) {
          // Fallback to root so UI doesn't go blank.
          detailFocusValue = root;
          detailFocusInvalid = r.error;
        } else {
          detailFocusValue = r.value;
        }
      }
    }
  }

  // Build current-page "record list" for json subtree mode (keep template simple).
  $: {
    if (recordViewMode !== 'json_subtree') {
      recordSubviewRecords = [];
      recordSubviewEmptyMsg = null;
      recordSubviewRangeText = '';
    } else {
      const v = recordFocusValue;
      const start = recordFocusPageIndex * pageSize;
      const end = Math.min(recordFocusTotal, start + pageSize);
      recordSubviewRangeText = recordFocusTotal > 0 ? `${start + 1}–${end} / ${recordFocusTotal}` : `0 / 0`;

      const out: Record[] = [];
      recordSubviewEmptyMsg = null;

      if (Array.isArray(v)) {
        const slice = v.slice(start, end);
        for (let i = 0; i < slice.length; i++) {
          const rid = start + i;
          out.push(makePseudoRecord(rid, `#${rid}`, slice[i]));
        }
      } else if (v && typeof v === 'object') {
        const keys = Object.keys(v as object).slice(start, end);
        for (let i = 0; i < keys.length; i++) {
          const rid = start + i;
          const k = keys[i];
          const item = (v as { [kk: string]: unknown })[k];
          out.push(makePseudoRecord(rid, `${k}:`, item));
        }
      } else if (v !== null && v !== undefined) {
        out.push(makePseudoRecord(0, '', v));
      } else {
        recordSubviewEmptyMsg = '该节点没有可展示的内容。';
      }

      recordSubviewRecords = out;
    }
  }

  function textMatchesLikeDetail(hay: string, q: string): boolean {
    const needle = q.trim();
    if (!needle) return false;
    const h = hay.toLowerCase();
    const n = needle.toLowerCase();
    if (h.includes(n)) return true;
    // Accept both unquoted and JSON-string-quoted needles (detail view shows strings with quotes).
    const quoted = JSON.stringify(needle).toLowerCase();
    return h.includes(quoted);
  }

  // Build local search results for json_subtree mode over the ENTIRE subtree (not just current page).
  $: {
    recordSubviewSearchEmptyMsg = null;
    recordSubviewSearchRangeText = '';
    recordSubviewSearchAll = [];

    if (recordViewMode !== 'json_subtree') {
      // no-op
    } else if (!recordSearchActive || !recordSearchCommittedText.trim()) {
      // not searching => keep default list mode
    } else {
      const q = recordSearchCommittedText.trim();
      const v = recordFocusValue;
      const out: Record[] = [];

      if (Array.isArray(v)) {
        for (let i = 0; i < v.length; i++) {
          const r = makePseudoRecord(i, `#${i}`, v[i]);
          const hay = `${r.preview}\n${r.raw ?? ''}`;
          if (textMatchesLikeDetail(hay, q)) out.push(r);
        }
      } else if (v && typeof v === 'object') {
        const entries = Object.entries(v as { [k: string]: unknown });
        for (let i = 0; i < entries.length; i++) {
          const [k, vv] = entries[i];
          const r = makePseudoRecord(i, `${k}:`, vv);
          const hay = `${r.preview}\n${r.raw ?? ''}`;
          if (textMatchesLikeDetail(hay, q)) out.push(r);
        }
      } else if (v !== null && v !== undefined) {
        const r = makePseudoRecord(0, '', v);
        const hay = `${r.preview}\n${r.raw ?? ''}`;
        if (textMatchesLikeDetail(hay, q)) out.push(r);
      }

      recordSubviewSearchAll = out;
      if (out.length === 0) recordSubviewSearchEmptyMsg = '子树内无命中。';

      const start = recordSubviewSearchPageIndex * pageSize;
      const end = Math.min(out.length, start + pageSize);
      recordSubviewSearchRangeText = out.length > 0 ? `${start + 1}–${end} / ${out.length}` : `0 / 0`;
    }
  }

  // Detail search bookkeeping: after any re-render that could affect hits, recompute and optionally jump to first hit.
  $: {
    // This reactive block intentionally ignores `detailSearchHitsCount`/`detailSearchHitIndex` to avoid loops.
    // eslint-disable-next-line @typescript-eslint/no-unused-expressions
    detailSearchText;
    // eslint-disable-next-line @typescript-eslint/no-unused-expressions
    selected?.id;
    // eslint-disable-next-line @typescript-eslint/no-unused-expressions
    detailViewMode;
    // eslint-disable-next-line @typescript-eslint/no-unused-expressions
    pathToLabel(detailFocusPath);
    // eslint-disable-next-line @typescript-eslint/no-unused-expressions
    detailDefaultExpand;
    // Trigger async refresh (fire-and-forget).
    refreshDetailSearchHits(true);
  }
</script>

<svelte:window on:keydown={onKeyDownGlobal} />

<div class="app">
  <header class="toolbar">
    <button class="primary" on:click={onPickFile} disabled={busy}>打开文件…</button>
    <button class="primary" on:click={onPickFolder} disabled={busy}>打开文件夹…</button>
    {#if !isTauriEnv}
      <button on:click={() => openFilePath(demoFiles[0].path)} disabled={busy} use:tooltip={{ text: 'Web 测试模式：加载内置示例数据' }}>
        打开示例数据
      </button>
      <button on:click={() => scanFolderPath('demo://root')} disabled={busy} use:tooltip={{ text: 'Web 测试模式：加载示例文件夹树' }}>
        打开示例文件夹
      </button>
    {/if}
    {#if openRequestId && openPct !== null}
      <div class="open-progress" aria-live="polite">
        <div class="open-progress-text">{openStage} {openPct}%</div>
        <div class="open-progress-bar" role="progressbar" aria-valuenow={openPct} aria-valuemin="0" aria-valuemax="100">
          <div class="open-progress-bar-inner" style={`width: ${openPct}%`} />
        </div>
      </div>
    {/if}

    <button class="primary" on:click={() => (exportModalOpen = true)} disabled={!session || busy}>导出…</button>

    <div class="spacer" />
    <div class="field page-size">
      <label for="pageSizeInput">每页条数</label>
      <input id="pageSizeInput" type="number" min="1" max="100" bind:value={pageSize} />
    </div>
  </header>

  {#if errorMsg}
    <div class="error-banner" role="alert">
      <div class="error-text">{errorMsg}</div>
      <button
        class="icon-btn"
        type="button"
        on:click={() => (errorMsg = null)}
        aria-label="关闭错误提示"
        use:tooltip={{ text: '关闭错误提示' }}
      >
        ×
      </button>
    </div>
  {/if}

  <main
    class="layout"
    bind:this={layoutEl}
    style={`grid-template-columns: ${sidebarCollapsed ? '56px' : `${sidebarWidth}px`} 10px 1fr;`}
  >
    <aside
      class="sidebar {sidebarCollapsed ? 'collapsed' : ''} {sessionDropActive ? 'drop-active' : ''}"
      on:dragover={onSessionDragOver}
      on:dragleave={onSessionDragLeave}
      on:drop={onSessionDrop}
    >
      <div class="sidebar-head">
        <button
          class="icon-btn"
          type="button"
          on:click={() => setSidebarCollapsed(!sidebarCollapsed)}
          aria-label={sidebarCollapsed ? '展开 Session 面板' : '收起 Session 面板'}
          use:tooltip={{ text: sidebarCollapsed ? '展开 Session 面板' : '收起 Session 面板' }}
        >
          {#if sidebarCollapsed}»{:else}«{/if}
        </button>
        {#if !sidebarCollapsed}
          <div class="sidebar-title">Session</div>
        {/if}
      </div>

      {#if !sidebarCollapsed}
        <div class="panel-lite">
          <h2>当前打开</h2>
          {#if session}
            <div class="kv"><span class="k">文件</span><span class="v">{session.path}</span></div>
            <div class="kv"><span class="k">格式</span><span class="v">{session.format}</span></div>
          {:else}
            <p class="muted">尚未打开文件。</p>
          {/if}

          <div class="dropzone" aria-label="拖拽导入">
            <div class="dropzone-title">拖入 Session</div>
            <div class="muted">支持文件 / 文件夹（拖到这里即可）</div>
          </div>


          <h2>文件树</h2>
          {#if folderTreeRoot}
            {#if folderTreeTruncated}
              <p class="muted">目录过大：已截断显示（节点 {folderTreeTotalNodes}）。</p>
            {:else}
              <p class="muted">节点 {folderTreeTotalNodes}。</p>
            {/if}
            <div class="tree" role="tree" aria-label="文件树">
              {#if folderTreeRoot.children}
                {#each folderTreeRoot.children as n (n.path)}
                  <FileTreeNode
                    node={n}
                    depth={0}
                    expanded={folderExpanded}
                    activePath={session?.path ?? folderSelectedPath}
                    busy={busy}
                    onToggleDir={toggleFolder}
                    onClickFile={onTreeFileClick}
                  />
                {/each}
              {/if}
            </div>
          {:else}
            <p class="muted">选择文件夹后会在这里显示可展开的文件树。</p>
          {/if}

          <h2>最近打开</h2>
          {#if recentFiles.length > 0}
            <div class="recent">
              {#each recentFiles as p (p)}
                <button class="recent-item" type="button" on:click={() => onOpenRecent(p)} disabled={busy} use:tooltip={{ text: p }}>
                  {p}
                </button>
              {/each}
            </div>
          {:else}
            <p class="muted">暂无记录。</p>
          {/if}
        </div>
      {:else}
        <div class="sidebar-collapsed-hint" title={session?.path ?? '尚未打开文件'}>
          <div class="hint-dot" />
        </div>
      {/if}
    </aside>

    <button
      class="sidebar-resizer"
      type="button"
      aria-label="调整 Session 面板宽度"
      on:pointerdown={onSidebarResizerPointerDown}
    />

    <section class="content" bind:this={splitEl}>
      <div
        class="split"
        style={`grid-template-columns: minmax(260px, ${splitPct}%) 8px minmax(360px, ${100 - splitPct}%);`}
      >
        <section class="panel panel-stack">
          <div class="panel-head">
            <div class="panel-title-wrap">
              <div class="panel-title">记录</div>
              <div class="panel-subtitle">
                {#if recordViewMode === 'json_subtree'}
                  本地分页：{recordSubviewRangeText}
                {:else if page}
                  每页 {pageSize} · 游标 {pageCursorIndex + 1}{page?.reached_eof ? ' · 已到末尾' : ''}
                {:else}
                  打开文件后即可浏览记录
                {/if}
              </div>
            </div>
            <div class="pager">
              {#if recordViewMode === 'json_subtree'}
                <div class="task-pill" title="当前记录列表来自 JSON 子树（可返回原始记录）">
                  <span class="dot" />
                  <span class="mono">子树：{pathToLabel(recordFocusPath)}</span>
                  <label class="muted" style="display: inline-flex; align-items: center; gap: 6px">
                    <input
                      type="checkbox"
                      checked={
                        recordSubviewRecords.length > 0 &&
                        recordSubviewRecords.every((r) => checkedSubtree.has(r.id))
                      }
                      on:change={(e) => {
                        const ids = recordSubviewRecords.map((r) => r.id);
                        const on = eventChecked(e);
                        checkedSubtree = on ? addMany(checkedSubtree, ids) : removeMany(checkedSubtree, ids);
                      }}
                      disabled={busy || recordSubviewRecords.length === 0}
                    />
                    全选
                  </label>
                  <button class="link" type="button" on:click={() => (checkedSubtree = new Set())} disabled={busy || checkedSubtree.size === 0}>
                    清空
                  </button>
                  <button class="link" type="button" on:click={exitJsonSubview} disabled={busy}>返回</button>
                </div>
                <button on:click={() => (recordFocusPageIndex = Math.max(0, recordFocusPageIndex - 1))} disabled={busy || recordFocusPageIndex <= 0}>
                  上一页
                </button>
                <button
                  on:click={() => (recordFocusPageIndex = recordFocusPageIndex + 1)}
                  disabled={busy || recordFocusTotal === 0 || (recordFocusPageIndex + 1) * pageSize >= recordFocusTotal}
                >
                  下一页
                </button>
              {:else}
                <label class="muted" style="display: inline-flex; align-items: center; gap: 6px; margin-right: 10px">
                  <input
                    type="checkbox"
                    checked={Boolean(page?.records?.length) && page.records.every((r) => checked.has(r.id))}
                    on:change={(e) => {
                      const ids = (page?.records ?? []).map((r) => r.id);
                      const on = eventChecked(e);
                      checked = on ? addMany(checked, ids) : removeMany(checked, ids);
                    }}
                    disabled={busy || !page?.records?.length}
                  />
                  全选
                </label>
                <button
                  on:click={onPrevPage}
                  disabled={!session || pageCursorIndex <= 0 || busy}
                >
                  上一页
                </button>
                <button
                  on:click={onNextPage}
                  disabled={
                    !session ||
                    busy ||
                    (!page?.next_cursor && pageCursorIndex >= pageCursorHistory.length - 1)
                  }
                >
                  下一页
                </button>
                <button
                  class="icon-btn"
                  type="button"
                  on:click={openRecordFocusModal}
                  disabled={!detailJsonOk || session?.format !== 'json'}
                  aria-label="选择 JSON 节点作为记录列表"
                  use:tooltip={{ text: '选择 JSON 节点作为记录列表' }}
                >
                  ⎘
                </button>
              {/if}
            </div>
          </div>

          <div class="panel-tabs" role="tablist" aria-label="记录视图切换">
            <button
              type="button"
              class="tab {recordPanelTab === 'records' ? 'active' : ''}"
              role="tab"
              aria-selected={recordPanelTab === 'records'}
              on:click={() => (recordPanelTab = 'records')}
            >
              记录
            </button>
            <button
              type="button"
              class="tab {recordPanelTab === 'search' ? 'active' : ''}"
              role="tab"
              aria-selected={recordPanelTab === 'search'}
              on:click={() => (recordPanelTab = 'search')}
            >
              检索结果
              {#if recordSearchActive && recordSearchCommittedText.trim()}
                <span class="badge">
                  {recordViewMode === 'json_subtree'
                    ? recordSubviewSearchAll.length
                    : recordSearchHits
                      ? recordSearchHits.records.length
                      : recordSearchTask
                        ? '…'
                        : 0}
                </span>
              {/if}
            </button>
          </div>

          {#if page}
            <div class="panel-body" role="tabpanel">
              {#if recordPanelTab === 'records'}
                <div class="list">
                  {#if recordViewMode === 'json_subtree'}
                    {#if recordFocusInvalid}
                      <div class="muted">所选节点路径已失效，已回退到根：{recordFocusInvalid}</div>
                    {/if}
                    {#if recordSubviewEmptyMsg}
                      <p class="muted">{recordSubviewEmptyMsg}</p>
                    {:else}
                      {#each recordSubviewRecords as r (r.id)}
                        <div class:selected={selected?.id === r.id} class="row">
                          <input type="checkbox" checked={checkedSubtree.has(r.id)} on:change={() => toggleCheckedSubtree(r.id)} />
                          <button class="row-btn" on:click={() => (selected = r)}>
                            <span class="badge mono">#{r.id}</span>
                            <span class="preview">{r.preview}</span>
                          </button>
                        </div>
                      {/each}
                    {/if}
                  {:else}
                    {#each page.records as r (r.id)}
                      <div class:selected={selected?.id === r.id} class="row">
                        <input type="checkbox" checked={checked.has(r.id)} on:change={() => toggleChecked(r.id)} />
                        <button class="row-btn" on:click={() => (selected = r)}>
                          <span class="badge mono">#{r.id}</span>
                          <span class="preview">{r.preview}</span>
                        </button>
                      </div>
                    {/each}
                  {/if}
                </div>
                <div class="panel-hint muted">下一页游标：{page.next_cursor ? '有' : '无'}</div>
              {:else}
                {#if recordSearchActive && recordSearchCommittedText.trim()}
                  {#if recordViewMode === 'json_subtree'}
                    <div class="pager panel-search-results-head">
                      <div class="muted">子树检索命中：{recordSubviewSearchAll.length}；分页：{recordSubviewSearchRangeText}</div>
                      <label class="muted" style="display: inline-flex; align-items: center; gap: 6px">
                        <input
                          type="checkbox"
                          checked={
                            recordSubviewSearchAll
                              .slice(recordSubviewSearchPageIndex * pageSize, recordSubviewSearchPageIndex * pageSize + pageSize)
                              .every((r) => checkedSubtree.has(r.id))
                          }
                          on:change={(e) => {
                            const pageRecs = recordSubviewSearchAll.slice(
                              recordSubviewSearchPageIndex * pageSize,
                              recordSubviewSearchPageIndex * pageSize + pageSize
                            );
                            const ids = pageRecs.map((r) => r.id);
                            const on = eventChecked(e);
                            checkedSubtree = on ? addMany(checkedSubtree, ids) : removeMany(checkedSubtree, ids);
                          }}
                          disabled={busy || recordSubviewSearchAll.length === 0}
                        />
                        全选
                      </label>
                      <button
                        on:click={() => (recordSubviewSearchPageIndex = Math.max(0, recordSubviewSearchPageIndex - 1))}
                        disabled={busy || recordSubviewSearchPageIndex <= 0}
                      >
                        上一页
                      </button>
                      <button
                        on:click={() => (recordSubviewSearchPageIndex = recordSubviewSearchPageIndex + 1)}
                        disabled={
                          busy ||
                          recordSubviewSearchAll.length === 0 ||
                          (recordSubviewSearchPageIndex + 1) * pageSize >= recordSubviewSearchAll.length
                        }
                      >
                        下一页
                      </button>
                    </div>
                    {#if recordSubviewSearchEmptyMsg}
                      <p class="muted">{recordSubviewSearchEmptyMsg}</p>
                    {:else}
                      <div class="list">
                        {#each recordSubviewSearchAll.slice(recordSubviewSearchPageIndex * pageSize, recordSubviewSearchPageIndex * pageSize + pageSize) as r (r.id)}
                          <div class:selected={selected?.id === r.id} class="row">
                            <input type="checkbox" checked={checkedSubtree.has(r.id)} on:change={() => toggleCheckedSubtree(r.id)} />
                            <button class="row-btn" on:click={() => (selected = r)}>
                              <span class="badge mono">#{r.id}</span>
                              <span class="preview">{r.preview}</span>
                            </button>
                          </div>
                        {/each}
                      </div>
                    {/if}
                  {:else}
                    <div class="pager panel-search-results-head">
                      <div class="muted">
                        {#if recordSearchHits}
                          检索命中：{recordSearchHits.records.length}；已到末尾：{String(recordSearchHits.reached_eof)}；下一页游标：{recordSearchHitsCursor ? '有' : '无'}
                        {:else if recordSearchTask}
                          正在检索… {recordSearchTask.progress_0_100}%
                        {:else}
                          检索已启动：点击下方命中项以查看详情。
                        {/if}
                      </div>
                      {#if recordSearchHits && recordSearchHits.records.length > 0}
                        <label class="muted" style="display: inline-flex; align-items: center; gap: 6px">
                          <input
                            type="checkbox"
                            checked={recordSearchHits.records.every((r) => checked.has(r.id))}
                            on:change={(e) => {
                              const ids = recordSearchHits?.records?.map((r) => r.id) ?? [];
                              const on = eventChecked(e);
                              checked = on ? addMany(checked, ids) : removeMany(checked, ids);
                            }}
                            disabled={busy}
                          />
                          全选
                        </label>
                      {/if}
                    </div>
                    {#if recordSearchHits}
                      <div class="list">
                        {#each recordSearchHits.records as r (r.id)}
                          <div class:selected={selected?.id === r.id} class="row">
                            <input type="checkbox" checked={checked.has(r.id)} on:change={() => toggleChecked(r.id)} />
                            <button class="row-btn" on:click={() => (selected = r)}>
                              <span class="badge mono">#{r.id}</span>
                              <span class="preview">{r.preview}</span>
                            </button>
                          </div>
                        {/each}
                      </div>
                      {#if recordSearchHitsCursor}
                        <button on:click={onMoreRecordSearchHits} disabled={busy || !recordSearchHitsCursor}>加载更多</button>
                      {/if}
                    {/if}
                  {/if}
                {:else}
                  <p class="muted">开始检索后，命中结果会显示在这里（上方“记录”列表不会被检索改变）。</p>
                {/if}
              {/if}
            </div>

            <div class="panel-footer" aria-label="记录检索">
              <div class="panel-search">
              <input
                class="panel-search-text"
                bind:value={recordSearchDraft}
                placeholder={recordViewMode === 'json_subtree' ? '在子树中检索（匹配展示内容）…' : '全文件检索（匹配展示内容）…'}
                disabled={!session || busy}
                on:keydown={(e) => {
                  if (e.key === 'Enter') onRecordSearch();
                }}
              />
              <button on:click={onRecordSearch} disabled={!session || busy}>检索</button>
              <button
                type="button"
                class="link"
                on:click={clearRecordSearch}
                disabled={busy || (!recordSearchDraft && !recordSearchActive && !recordSearchHits && !recordSearchTask)}
              >
                清除
              </button>
              {#if recordViewMode === 'backend' && recordSearchTask}
                <div class="task-pill" title={recordSearchTask.id}>
                  <span>任务 {recordSearchTask.progress_0_100}%</span>
                  {#if !recordSearchTask.finished}
                    <span class="dot" aria-hidden="true" />
                  {/if}
                  <button class="link" on:click={onCancelRecordSearch} disabled={!recordSearchTask.cancellable}>取消</button>
                </div>
                {#if !recordSearchTask.finished}
                  <div
                    class="task-progress-bar"
                    role="progressbar"
                    aria-valuenow={recordSearchTask.progress_0_100}
                    aria-valuemin="0"
                    aria-valuemax="100"
                    title={`检索进度 ${recordSearchTask.progress_0_100}%`}
                  >
                    <div class="task-progress-bar-inner" style={`width: ${recordSearchTask.progress_0_100}%`} />
                  </div>
                {/if}
              {/if}
            </div>
            </div>
          {:else}
            <p class="muted">打开文件后即可浏览记录。</p>
          {/if}
        </section>

        <button
          class="splitter"
          type="button"
          aria-label="调整记录与详情宽度"
          on:pointerdown={onSplitterPointerDown}
          on:keydown={onSplitterKeyDown}
        />

        <section class="panel panel-detail">
          <div class="panel-head panel-head-detail">
            <div class="panel-title-wrap">
              <div class="panel-title">详情</div>
              <div class="panel-subtitle">
                {#if selected?.meta}行 {selected.meta.line_no} · {/if}{detailCharLen} 字符{detailTruncated ? ' · 已截断' : ''}
              </div>
            </div>
            <div class="detail-search" aria-label="详情检索">
              <input
                class="panel-search-text"
                bind:value={detailSearchDraft}
                placeholder="输入关键字后回车/确认开始检索（高亮+跳转）…"
                disabled={!selected || !detailJsonOk || busy}
                on:keydown={(e) => {
                  if (e.key === 'Enter') {
                    e.preventDefault();
                    confirmDetailSearch();
                  }
                }}
              />
              <button
                type="button"
                on:click={confirmDetailSearch}
                disabled={!selected || !detailJsonOk || busy || !detailSearchDraft.trim()}
              >
                确认
              </button>
              <button type="button" on:click={() => jumpDetailHit(-1)} disabled={busy || detailSearchHitsCount === 0}>上一个</button>
              <button type="button" on:click={() => jumpDetailHit(1)} disabled={busy || detailSearchHitsCount === 0}>下一个</button>
              <button type="button" class="link" on:click={clearDetailSearch} disabled={busy || (!detailSearchText && !detailSearchDraft)}>
                清除
              </button>
              <span class="muted mono" aria-live="polite">
                {#if detailSearchText.trim()}
                  {detailSearchHitsCount > 0 ? `${detailSearchHitIndex}/${detailSearchHitsCount}` : '0/0'}
                {/if}
              </span>
            </div>
            <div class="detail-switches" aria-label="详情默认展示设置">
              {#if detailViewMode === 'json_subtree' && detailJsonOk}
                <div class="task-pill" title="当前详情仅展示所选 JSON 子树（可返回查看完整根）">
                  <span class="dot" />
                  <span class="mono">子树：{pathToLabel(detailFocusPath)}</span>
                  <button class="link" type="button" on:click={exitDetailSubview} disabled={busy}>返回</button>
                </div>
              {/if}

              <button
                class="icon-btn"
                type="button"
                on:click={openDetailFocusModal}
                disabled={!detailJsonOk}
                aria-label="选择 JSON 节点作为详情视图根"
                use:tooltip={{ text: '选择 JSON 节点作为详情视图根' }}
              >
                ⎘
              </button>

                <button
                type="button"
                class="switch"
                role="switch"
                aria-checked={detailDefaultExpand}
                on:click={() => (detailDefaultExpand = !detailDefaultExpand)}
                use:tooltip={{ text: '切换 JSON 是否全部展开' }}
              >
                <span class="switch-label">全部展开</span>
                <span class="switch-track" aria-hidden="true">
                  <span class="switch-thumb" class:on={detailDefaultExpand} />
                </span>
              </button>
            </div>
          </div>
          {#if selected}
            {#if selected.meta}
              <div class="kv"><span class="k">行号</span><span class="v">{selected.meta.line_no}</span></div>
            {/if}
            <div class="kv"><span class="k">字符长度</span><span class="v">{detailCharLen}</span></div>
            {#if detailTruncated}
              <div class="kv">
                <span class="k">内容状态</span>
                <span class="v">
                  已截断
                  {#if detailTooLargeHint}
                    <span class="muted">（{detailTooLargeHint}）</span>
                  {/if}
                </span>
              </div>
              <div style="display: flex; gap: 8px; flex-wrap: wrap; margin: 6px 0 2px">
                <button type="button" on:click={onLoadFullDetail} disabled={busy || detailLoadingFull || !detailCanLoadFull}>
                  加载完整内容
                </button>
                <button type="button" on:click={onExportCurrentRecord} disabled={busy || !session || !selected?.meta}>
                  导出本条记录…
                </button>
              </div>
            {/if}

            {#if detailJsonOk}
              <div class="json-view" role="region" aria-label="JSON 结构化详情" bind:this={detailJsonViewEl}>
                <button
                  class="icon-btn json-copy-btn"
                  type="button"
                  on:click={onCopyDetailJson}
                  disabled={detailCopying}
                  aria-label="复制当前详情为格式化 JSON"
                  use:tooltip={{ text: detailCopied ? '已复制' : '复制当前详情为格式化 JSON' }}
                >
                  ⧉
                </button>
                {#if detailFocusInvalid}
                  <div class="muted">所选节点路径在当前记录中无效，已回退到根：{detailFocusInvalid}</div>
                {/if}
                {#if detailCopyErr}
                  <div class="muted">复制失败：{detailCopyErr}</div>
                {/if}
                {#key `${selected?.id ?? 'none'}-${detailDefaultExpand ? 'e1' : 'e0'}-${detailViewMode}-${pathToLabel(detailFocusPath)}` }
                  <JsonTree
                    value={detailViewMode === 'json_subtree' ? detailFocusValue : detailJsonValue}
                    defaultExpandedDepth={detailDefaultExpand ? 1_000_000_000 : 0}
                    indentPx={14}
                    highlightText={detailSearchText}
                    highlightCaseSensitive={false}
                  />
                {/key}
              </div>
            {:else}
              {#if detailStreamMode && (session?.format === 'json' || session?.format === 'jsonl') && selected?.meta}
                <div class="json-view" role="region" aria-label="JSON（流式）结构化详情">
                  <div class="muted" style="margin-bottom: 6px">超大记录：已启用流式结构浏览（按需加载子节点）。</div>
                  <JsonLazyTree sessionId={session.session_id} meta={selected.meta} />
                </div>
                <pre class="raw">{detailText}</pre>
              {:else}
                <pre class="raw">{detailText}</pre>
              {/if}
              {#if detailLoadingFull}
                <div class="muted">正在加载完整内容…</div>
              {/if}
              {#if detailJsonErr}
                <div class="muted">JSON 解析失败（已回退到原始文本）：{detailJsonErr}</div>
              {/if}
            {/if}
          {:else}
            <p class="muted">请选择一条记录。</p>
          {/if}

        </section>
      </div>
    </section>
  </main>
</div>

{#if exportModalOpen}
  <div
    class="modal-backdrop"
    role="presentation"
    on:click={(e) => {
      if (e.target === e.currentTarget) exportModalOpen = false;
    }}
  >
    <div class="modal" role="dialog" aria-modal="true" aria-label="导出配置">
      <div class="modal-head">
        <div class="modal-title">导出</div>
        <button class="icon-btn" type="button" on:click={() => (exportModalOpen = false)} aria-label="关闭" use:tooltip={{ text: '关闭' }}>
          ×
        </button>
      </div>

      <div class="field">
        <label for="exportFormatSelect">格式</label>
        <select id="exportFormatSelect" bind:value={exportFormat} disabled={!session || busy}>
          <option value="jsonl">jsonl</option>
          <option value="json">json</option>
          <option value="csv">csv</option>
        </select>
      </div>

      <div class="modal-actions">
        <button type="button" on:click={() => (exportModalOpen = false)} disabled={busy}>取消</button>
        <button type="button" on:click={onExport} disabled={!session || busy}>开始导出</button>
      </div>

      {#if errorMsg}
        <div class="error">{errorMsg}</div>
      {/if}
    </div>
  </div>
{/if}

{#if recordFocusModalOpen}
  <div
    class="modal-backdrop"
    role="presentation"
    on:click={(e) => {
      if (e.target === e.currentTarget) recordFocusModalOpen = false;
    }}
  >
    <div class="modal modal-wide" role="dialog" aria-modal="true" aria-label="选择 JSON 节点">
      <div class="modal-head">
        <div class="modal-title">选择 JSON 节点（作为记录列表）</div>
        <button
          class="icon-btn"
          type="button"
          on:click={() => (recordFocusModalOpen = false)}
          aria-label="关闭"
          use:tooltip={{ text: '关闭' }}
        >
          ×
        </button>
      </div>

      <div class="muted" style="margin-bottom: 8px">
        选中一个节点后点击“确认”，左侧“记录”将改为展示该节点下的列表（数组按 index，对象按 key）。
      </div>

      <div class="kv" style="padding-top: 0">
        <span class="k">当前选择</span>
        <span class="v"><span class="mono">{pathToLabel(recordFocusDraftPath.length === 0 ? null : recordFocusDraftPath)}</span></span>
      </div>

      <div class="json-picker" role="region" aria-label="JSON 节点树">
        <JsonTreePicker
          value={detailJsonValue}
          selectedPath={recordFocusDraftPath}
          defaultExpandedDepth={1}
          indentPx={14}
          onSelect={(p) => (recordFocusDraftPath = p)}
        />
      </div>

      <div class="modal-actions">
        <button type="button" on:click={() => (recordFocusModalOpen = false)} disabled={busy}>取消</button>
        <button type="button" on:click={() => (recordFocusDraftPath = [])} disabled={busy || !detailJsonOk}>选根（全部）</button>
        <button type="button" on:click={confirmRecordFocus} disabled={busy || !detailJsonOk}>确认</button>
      </div>
    </div>
  </div>
{/if}

{#if detailFocusModalOpen}
  <div
    class="modal-backdrop"
    role="presentation"
    on:click={(e) => {
      if (e.target === e.currentTarget) detailFocusModalOpen = false;
    }}
  >
    <div class="modal modal-wide" role="dialog" aria-modal="true" aria-label="选择 JSON 节点（详情）">
      <div class="modal-head">
        <div class="modal-title">选择 JSON 节点（作为详情视图根）</div>
        <button
          class="icon-btn"
          type="button"
          on:click={() => (detailFocusModalOpen = false)}
          aria-label="关闭"
          use:tooltip={{ text: '关闭' }}
        >
          ×
        </button>
      </div>

      <div class="muted" style="margin-bottom: 8px">
        选中一个节点后点击“确认”，右侧“详情”将只展示该节点下的内容（切换记录会沿用同一路径）。
      </div>

      <div class="kv" style="padding-top: 0">
        <span class="k">当前选择</span>
        <span class="v"><span class="mono">{pathToLabel(detailFocusDraftPath.length === 0 ? null : detailFocusDraftPath)}</span></span>
      </div>

      <div class="json-picker" role="region" aria-label="JSON 节点树（详情）">
        <JsonTreePicker
          value={detailJsonValue}
          selectedPath={detailFocusDraftPath}
          defaultExpandedDepth={1}
          indentPx={14}
          onSelect={(p) => (detailFocusDraftPath = p)}
        />
      </div>

      <div class="modal-actions">
        <button type="button" on:click={() => (detailFocusModalOpen = false)} disabled={busy}>取消</button>
        <button type="button" on:click={() => (detailFocusDraftPath = [])} disabled={busy || !detailJsonOk}>选根（全部）</button>
        <button type="button" on:click={confirmDetailFocus} disabled={busy || !detailJsonOk}>确认</button>
      </div>
    </div>
  </div>
{/if}

<style>
  :global(:root) {
    color-scheme: light dark;

    /* Theme tokens (light) */
    --bg0: #f6f7fb;
    --bg1: #eef2ff;
    --fg: #0f172a;
    --muted: #64748b;
    --border: rgba(15, 23, 42, 0.12);
    --border-strong: rgba(15, 23, 42, 0.18);
    --border-soft: rgba(15, 23, 42, 0.08);
    --surface: rgba(255, 255, 255, 0.78);
    --surface-solid: #ffffff;
    --surface-2: rgba(255, 255, 255, 0.92);

    --accent: #2563eb;
    --accent-2: #60a5fa;
    --accent-soft: rgba(37, 99, 235, 0.12);
    --accent-border: rgba(37, 99, 235, 0.45);

    --danger: #dc2626;
    --danger-soft: rgba(220, 38, 38, 0.12);
    --danger-border: rgba(220, 38, 38, 0.45);

    --shadow-sm: 0 1px 2px rgba(15, 23, 42, 0.06);
    --shadow: 0 8px 30px rgba(15, 23, 42, 0.10), 0 2px 10px rgba(15, 23, 42, 0.06);

    --ring: rgba(96, 165, 250, 0.45);
    --radius: 12px;
    --radius-sm: 10px;

    /* Typography scale (visible hierarchy) */
    --text-xs: 12px;
    --text-sm: 13px;
    --text-base: 14px;
    --text-lg: 16px;
    --text-xl: 18px;
    --leading-tight: 1.25;
    --leading: 1.45;
  }

  @media (prefers-color-scheme: dark) {
    :global(:root) {
      --bg0: #070a12;
      --bg1: #0b1020;
      --fg: #e5e7eb;
      --muted: rgba(229, 231, 235, 0.7);
      --border: rgba(255, 255, 255, 0.12);
      --border-strong: rgba(255, 255, 255, 0.18);
      --border-soft: rgba(255, 255, 255, 0.08);
      --surface: rgba(15, 23, 42, 0.55);
      --surface-solid: #0b1020;
      --surface-2: rgba(15, 23, 42, 0.75);

      --accent: #60a5fa;
      --accent-2: #93c5fd;
      --accent-soft: rgba(96, 165, 250, 0.18);
      --accent-border: rgba(96, 165, 250, 0.55);

      --danger: #fb7185;
      --danger-soft: rgba(251, 113, 133, 0.16);
      --danger-border: rgba(251, 113, 133, 0.55);

      --shadow-sm: 0 1px 2px rgba(0, 0, 0, 0.26);
      --shadow: 0 12px 40px rgba(0, 0, 0, 0.40);
      --ring: rgba(147, 197, 253, 0.40);
    }
  }

  :global(*) {
    box-sizing: border-box;
  }
  :global(html),
  :global(body) {
    height: 100%;
  }
  :global(body) {
    margin: 0;
    color: var(--fg);
    background:
      radial-gradient(900px 380px at 20% 0%, rgba(96, 165, 250, 0.18), transparent 55%),
      radial-gradient(880px 360px at 80% 0%, rgba(167, 139, 250, 0.14), transparent 58%),
      linear-gradient(180deg, var(--bg0), var(--bg1));
  }

  .app {
    font-family:
      ui-sans-serif,
      system-ui,
      -apple-system,
      BlinkMacSystemFont,
      'Segoe UI',
      Roboto,
      Helvetica,
      Arial,
      'Apple Color Emoji',
      'Segoe UI Emoji';
    height: 100vh;
    display: flex;
    flex-direction: column;
    min-height: 0;
  }
  .toolbar {
    display: flex;
    align-items: center;
    gap: 6px;
    padding: 4px 8px;
    border-bottom: 1px solid var(--border);
    flex-wrap: wrap;
    position: sticky;
    top: 0;
    z-index: 50;
    background: var(--surface);
    backdrop-filter: blur(10px);
  }
  .open-progress {
    display: grid;
    gap: 4px;
    min-width: 160px;
  }
  .open-progress-text {
    font-size: 11px;
    color: var(--muted);
    white-space: nowrap;
  }
  .open-progress-bar {
    height: 6px;
    border-radius: 999px;
    background: var(--border-soft);
    overflow: hidden;
  }
  .open-progress-bar-inner {
    height: 100%;
    background: linear-gradient(90deg, var(--accent), var(--accent-2));
    width: 0%;
    transition: width 120ms linear;
  }
  /* removed: toolbar-search (search moved into record/detail panels) */
  .spacer {
    flex: 1;
  }
  .page-size {
    margin-left: auto;
  }
  /* Compact toolbar controls to reduce header height */
  .toolbar button {
    padding: 4px 8px;
  }
  .toolbar input {
    padding: 4px 8px;
    font-size: 12px;
  }
  .layout {
    display: grid;
    gap: 0;
    padding: 12px;
    box-sizing: border-box;
    flex: 1 1 auto;
    min-height: 0;
  }
  .sidebar {
    border: 1px solid var(--border);
    border-radius: var(--radius);
    padding: 10px;
    overflow: auto;
    min-width: 0;
    min-height: 0;
    background: var(--surface);
    box-shadow: var(--shadow-sm);
    backdrop-filter: blur(10px);
  }
  .sidebar.drop-active {
    border-color: var(--accent-border);
    box-shadow: 0 0 0 3px var(--ring), var(--shadow-sm);
  }
  .sidebar.collapsed {
    padding: 8px;
    overflow: hidden;
  }
  .sidebar-head {
    display: flex;
    align-items: center;
    gap: 8px;
    margin-bottom: 6px;
  }
  .sidebar-title {
    font-size: 12px;
    color: var(--muted);
    text-transform: uppercase;
    letter-spacing: 0.06em;
    font-weight: 700;
  }
  .panel-lite {
    display: grid;
    gap: 8px;
  }
  .dropzone {
    border: 1px dashed var(--border);
    border-radius: var(--radius);
    padding: 10px;
    background: var(--surface-2);
  }
  .dropzone-title {
    font-size: 12px;
    color: var(--fg);
    font-weight: 600;
    margin-bottom: 4px;
  }
  .sidebar-collapsed-hint {
    display: grid;
    place-items: center;
    height: 100%;
  }
  .hint-dot {
    width: 10px;
    height: 10px;
    border-radius: 999px;
    background: linear-gradient(180deg, var(--accent-2), var(--accent));
  }
  .sidebar-resizer {
    border: none;
    background: var(--border-soft);
    cursor: col-resize;
    border-radius: 999px;
    margin: 0;
    padding: 0;
    width: 10px;
    height: 100%;
    display: block;
  }
  .sidebar-resizer:hover,
  .sidebar-resizer:focus-visible {
    background: var(--accent-soft);
    outline: none;
  }
  .panel {
    border: 1px solid var(--border);
    border-radius: var(--radius);
    padding: 12px;
    overflow: auto;
    min-height: 0;
    background: var(--surface);
    box-shadow: var(--shadow-sm);
    backdrop-filter: blur(10px);
  }
  .panel-head {
    display: flex;
    align-items: center;
    gap: 10px;
    flex-wrap: wrap;
  }
  .panel-search {
    display: flex;
    align-items: center;
    gap: 8px;
    min-width: 0;
    flex: 1 1 360px;
  }
  .panel-search-results-head {
    justify-content: space-between;
  }
  .panel-search-text {
    min-width: 180px;
    flex: 1;
  }
  .detail-search {
    display: flex;
    align-items: center;
    gap: 8px;
    min-width: 0;
    flex: 1 1 360px;
  }
  .content {
    min-width: 0;
    min-height: 0;
    height: 100%;
  }
  .split {
    display: grid;
    height: 100%;
    min-height: 0;
    align-items: stretch;
  }
  .splitter {
    border: none;
    padding: 0;
    border-radius: 999px;
    background: var(--border-soft);
    cursor: col-resize;
    align-self: stretch;
    margin: 0 2px;
  }
  .splitter:hover,
  .splitter:focus-visible {
    background: var(--accent-soft);
    outline: none;
  }
  :global(body.dragging-split) {
    cursor: col-resize;
    user-select: none;
  }
  .panel-head {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 12px;
  }

  .panel-head-detail {
    margin-bottom: 8px;
  }

  .detail-switches {
    display: flex;
    align-items: center;
    gap: 8px;
  }

  .switch {
    display: inline-flex;
    align-items: center;
    gap: 8px;
    padding: 6px 10px;
    border-radius: 999px;
    border: 1px solid var(--border);
    background: var(--surface-2);
    cursor: pointer;
    user-select: none;
  }

  .switch:hover {
    border-color: var(--accent-border);
    background: var(--surface-2);
  }

  .switch:focus-visible {
    outline: 2px solid var(--accent);
    outline-offset: 2px;
  }

  .switch-label {
    font-size: 12px;
    color: var(--fg);
    white-space: nowrap;
  }

  .switch-track {
    width: 36px;
    height: 18px;
    border-radius: 999px;
    background: var(--border-strong);
    position: relative;
    flex: 0 0 auto;
  }

  .switch-thumb {
    width: 16px;
    height: 16px;
    border-radius: 50%;
    background: #ffffff;
    position: absolute;
    top: 1px;
    left: 1px;
    transition: transform 160ms ease, background 160ms ease;
    box-shadow: 0 1px 2px rgba(0, 0, 0, 0.22);
  }

  .switch-thumb.on {
    transform: translateX(18px);
    background: var(--accent-2);
  }
  h2 {
    font-size: 12px;
    margin: 8px 0;
    color: var(--muted);
    letter-spacing: 0.04em;
    font-weight: 700;
  }
  .field {
    display: grid;
    gap: 4px;
    margin: 8px 0;
  }
  label {
    font-size: var(--text-xs);
    color: var(--muted);
  }
  input,
  select,
  button {
    font: inherit;
  }
  input,
  select {
    border: 1px solid var(--border);
    border-radius: var(--radius-sm);
    padding: 6px 8px;
    background: var(--surface-solid);
    color: var(--fg);
    font-size: var(--text-sm);
  }
  input::placeholder {
    color: var(--muted);
  }
  input:focus-visible,
  select:focus-visible,
  button:focus-visible {
    outline: 2px solid var(--ring);
    outline-offset: 2px;
  }
  button {
    border: 1px solid var(--border);
    background: var(--surface-solid);
    border-radius: var(--radius-sm);
    padding: 6px 10px;
    cursor: pointer;
    box-shadow: var(--shadow-sm);
    font-size: var(--text-sm);
    transition:
      transform 80ms ease,
      box-shadow 120ms ease,
      border-color 120ms ease,
      background 120ms ease;
  }
  button:hover:not(:disabled) {
    border-color: var(--border-strong);
    background: var(--surface-2);
    box-shadow: var(--shadow);
    transform: translateY(-1px);
  }
  button:active:not(:disabled) {
    transform: translateY(0);
    box-shadow: var(--shadow-sm);
  }
  button:disabled {
    cursor: not-allowed;
    opacity: 0.6;
  }

  /* Button variants (opt-in with class) */
  button.primary {
    border-color: var(--accent-border);
    background: linear-gradient(180deg, var(--accent-2), var(--accent));
    color: #ffffff;
  }
  button.primary:hover:not(:disabled) {
    border-color: var(--accent-border);
    box-shadow: var(--shadow);
  }
  button.primary:active:not(:disabled) {
    box-shadow: var(--shadow-sm);
  }
  /* Panel typography & layout */
  .panel-title-wrap {
    display: grid;
    gap: 2px;
    min-width: 240px;
  }
  .panel-title {
    font-size: var(--text-lg);
    font-weight: 700;
    letter-spacing: 0.01em;
    line-height: var(--leading-tight);
    color: var(--fg);
  }
  .panel-subtitle {
    font-size: var(--text-xs);
    color: var(--muted);
    line-height: var(--leading);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }

  .panel.panel-stack {
    display: flex;
    flex-direction: column;
    overflow: hidden; /* keep header/tabs/footer fixed */
  }
  .panel-body {
    flex: 1 1 auto;
    min-height: 0;
    overflow: auto;
    padding: 10px 0 0;
  }
  .panel-footer {
    flex: 0 0 auto;
    margin-top: 10px;
    padding-top: 10px;
    border-top: 1px solid var(--border);
  }
  .panel-hint {
    margin-top: 8px;
  }

  .panel-tabs {
    display: inline-flex;
    gap: 6px;
    margin-top: 10px;
    padding: 4px;
    border-radius: 999px;
    border: 1px solid var(--border);
    background: var(--surface-2);
    align-self: flex-start;
  }
  .tab {
    border: 1px solid transparent;
    background: transparent;
    box-shadow: none;
    padding: 6px 10px;
    border-radius: 999px;
    cursor: pointer;
    color: var(--muted);
    display: inline-flex;
    align-items: center;
    gap: 8px;
    transform: none;
  }
  .tab:hover:not(:disabled) {
    background: var(--surface-solid);
    box-shadow: none;
    border-color: var(--border);
    transform: none;
  }
  .tab.active {
    background: var(--surface-solid);
    border-color: var(--border-strong);
    color: var(--fg);
  }

  .kv {
    display: grid;
    grid-template-columns: 90px 1fr;
    gap: 8px;
    padding: 4px 0;
  }
  .k {
    color: var(--muted);
    font-size: var(--text-xs);
  }
  .v {
    font-size: var(--text-sm);
    word-break: break-all;
  }
  .mono {
    font-family: ui-monospace, SFMono-Regular, Menlo, Monaco, Consolas, 'Liberation Mono', 'Courier New',
      monospace;
  }
  .muted {
    color: var(--muted);
    font-size: var(--text-xs);
  }
  .error {
    margin-top: 10px;
    padding: 8px;
    border-radius: 8px;
    background: var(--danger-soft);
    border: 1px solid var(--danger-border);
    color: var(--danger);
    font-size: 12px;
    word-break: break-word;
  }
  .list {
    display: grid;
    gap: 6px;
    margin: 8px 0;
  }
  .row {
    display: grid;
    grid-template-columns: 20px 1fr;
    gap: 8px;
    align-items: start;
    padding: 10px 10px;
    border: 1px solid var(--border);
    border-radius: var(--radius-sm);
    background: var(--surface-solid);
    transition: background 120ms ease, border-color 120ms ease, transform 80ms ease;
  }
  .row:hover {
    border-color: var(--border-strong);
    background: var(--surface-2);
  }
  .row.selected {
    border-color: var(--accent-border);
    background: var(--accent-soft);
  }
  .row-btn {
    border: none;
    background: transparent;
    padding: 0;
    text-align: left;
    cursor: pointer;
    display: grid;
    gap: 4px;
    width: 100%;
    box-shadow: none;
    transform: none;
  }
  .row-btn:hover:not(:disabled) {
    background: transparent;
    box-shadow: none;
    transform: none;
  }

  /* (unused) row-spacer kept previously for alignment; now all rows have checkbox column. */
  .preview {
    font-size: var(--text-base);
    color: var(--fg);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }

  .badge {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    padding: 2px 8px;
    border-radius: 999px;
    font-size: 11px;
    line-height: 18px;
    border: 1px solid var(--border);
    background: var(--surface-2);
    color: var(--muted);
    flex: 0 0 auto;
  }
  .raw {
    margin-top: 10px;
    padding: 10px;
    border-radius: 10px;
    background: #0b1020;
    color: #e5e7eb;
    font-size: 12px;
    overflow: auto;
    flex: 1 1 auto;
    min-height: 0;
    white-space: pre-wrap;
    word-break: break-word;
    overflow-wrap: anywhere;
    tab-size: 2;
  }

  .json-view {
    margin-top: 10px;
    padding: 10px;
    border-radius: 10px;
    background: #0b1020;
    border: 1px solid rgba(255, 255, 255, 0.06);
    overflow: auto;
    flex: 1 1 auto;
    min-height: 0;
    position: relative;
  }

  .json-copy-btn {
    position: absolute;
    top: 8px;
    right: 8px;
    opacity: 0;
    pointer-events: none;
    transition: opacity 120ms ease;
    border-color: rgba(255, 255, 255, 0.12);
    background: rgba(17, 24, 39, 0.55);
    color: #e5e7eb;
    backdrop-filter: blur(6px);
  }
  .json-view:hover .json-copy-btn,
  .json-view:focus-within .json-copy-btn {
    opacity: 1;
    pointer-events: auto;
  }
  .json-view:hover .json-copy-btn:disabled,
  .json-view:focus-within .json-copy-btn:disabled {
    opacity: 0.5;
    cursor: not-allowed;
  }

  /* Detail panel readability */
  .panel.panel-detail {
    display: flex;
    flex-direction: column;
    overflow: hidden; /* let .json-view/.raw scroll instead of the whole panel */
  }
  .panel-detail .k,
  .panel-detail .v {
    font-size: 13px;
  }
  .panel-detail .raw {
    font-size: 16px;
    line-height: 1.55;
  }
  .panel-detail .json-view {
    --jt-font-size: 16px;
  }

  .recent {
    display: grid;
    gap: 6px;
  }
  .recent-item {
    font-size: 12px;
    color: var(--fg);
    padding: 6px 8px;
    border: 1px solid #f3f4f6;
    border-radius: 8px;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
    text-align: left;
  }

  .pager {
    display: flex;
    gap: 8px;
    align-items: center;
  }

  .icon-btn {
    width: 28px;
    height: 28px;
    display: grid;
    place-items: center;
    border-radius: var(--radius-sm);
    border: 1px solid var(--border);
    background: var(--surface-solid);
    padding: 0;
    cursor: pointer;
    box-shadow: none;
  }
  .icon-btn:hover:not(:disabled) {
    border-color: var(--border-strong);
    background: var(--surface-2);
    transform: none;
    box-shadow: var(--shadow-sm);
  }

  .task-pill {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 6px 10px;
    border: 1px solid var(--border);
    border-radius: 999px;
    font-size: 12px;
    color: var(--muted);
    white-space: nowrap;
  }
  .task-progress-bar {
    height: 6px;
    width: 140px;
    border-radius: 999px;
    background: var(--border-soft);
    overflow: hidden;
  }
  .task-progress-bar-inner {
    height: 100%;
    background: linear-gradient(90deg, var(--accent), var(--accent-2));
  }
  .dot {
    width: 6px;
    height: 6px;
    border-radius: 999px;
    background: var(--accent-2);
    display: inline-block;
  }
  .link {
    border: none;
    background: transparent;
    padding: 0;
    color: var(--accent);
    cursor: pointer;
    text-decoration: underline;
    box-shadow: none;
    transform: none;
  }
  .link:disabled {
    opacity: 0.6;
    cursor: not-allowed;
  }

  .modal-backdrop {
    position: fixed;
    inset: 0;
    background: rgba(0, 0, 0, 0.35);
    backdrop-filter: blur(6px);
    display: grid;
    place-items: center;
    padding: 16px;
    z-index: 1000;
  }
  .modal {
    width: min(520px, 100%);
    background: var(--surface-2);
    border-radius: 14px;
    border: 1px solid var(--border);
    padding: 12px;
    box-shadow: var(--shadow);
    color: var(--fg);
  }
  .modal-head {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 10px;
    margin-bottom: 8px;
  }
  .modal-title {
    font-size: 14px;
    color: var(--fg);
    font-weight: 600;
  }
  .modal-actions {
    display: flex;
    justify-content: flex-end;
    gap: 8px;
    margin-top: 10px;
  }

  .modal-wide {
    width: min(880px, 100%);
  }

  .json-picker {
    margin-top: 8px;
    padding: 10px;
    border-radius: var(--radius);
    border: 1px solid var(--border);
    background: var(--surface-solid);
    max-height: 60vh;
    overflow: auto;
  }

  .error-banner {
    display: flex;
    align-items: flex-start;
    justify-content: space-between;
    gap: 10px;
    margin: 10px 12px 0;
    padding: 10px 10px;
    border-radius: 10px;
    background: var(--danger-soft);
    border: 1px solid var(--danger-border);
    color: var(--danger);
    font-size: 12px;
  }
  .error-text {
    word-break: break-word;
    flex: 1;
  }
  /* File tree */
  .tree {
    display: grid;
    gap: 2px;
    margin: 6px 0 10px;
  }
</style>

