<script lang="ts">
  import { tooltip } from '$lib/actions/tooltip';
  import { dialogSave } from '$lib/platform';
  import {
    exportToFile,
    jsonListChildrenAtOffset,
    jsonNodeSummaryAtOffset,
    type JsonChildItemOffset,
    type JsonNodeKind,
    type RecordMeta
  } from '$lib/ipc';

  export let sessionId: string;
  export let meta: RecordMeta;
  export let path: (string | number)[];
  export let nodeOffset: number;
  export let label: string;
  export let depth: number = 0;
  export let indentPx: number = 14;

  // Root is always expandable; children nodes expand only if backend says object/array,
  // but since we don't have kind info for root without fetching, we treat root as expandable.
  export let kind: JsonNodeKind | null = null;
  export let preview: string | null = null;

  let open = depth === 0;
  let loading = false;
  let err: string | null = null;
  let items: JsonChildItemOffset[] = [];
  let nextCursorOffset: number | null = 0;
  let nextCursorIndex: number | null = 0;
  let reachedEnd = false;

  let summaryLoading = false;
  let summaryErr: string | null = null;
  let summaryCount: number | null = null;
  let summaryComplete: boolean | null = null;

  // Reset state when switching records/session/path.
  let lastStateKey = '';
  $: stateKey = `${sessionId}|${meta?.byte_offset ?? 'na'}|${nodeOffset ?? 'na'}|${JSON.stringify(path ?? [])}`;
  $: if (stateKey !== lastStateKey) {
    lastStateKey = stateKey;
    open = depth === 0;
    loading = false;
    err = null;
    items = [];
    nextCursorOffset = 0;
    nextCursorIndex = 0;
    reachedEnd = false;
    summaryLoading = false;
    summaryErr = null;
    summaryCount = null;
    summaryComplete = null;
    if (open) {
      // fire-and-forget
      void ensureLoaded(true);
      void ensureSummary();
    }
  }

  function canExpand() {
    if (depth === 0) return true;
    return kind === 'object' || kind === 'array';
  }

  function pathToFileSafe() {
    if (!path || path.length === 0) return 'root';
    const parts = path.map((s) => String(s));
    return parts
      .join('_')
      .replaceAll(/[^\p{L}\p{N}\-_]+/gu, '_')
      .slice(0, 120);
  }

  async function onExportNodeJsonl() {
    if (!sessionId || !meta) return;
    const defaultPath = `node_${pathToFileSafe()}.jsonl`;
    const out = await dialogSave({ defaultPath });
    if (!out) return;
    await exportToFile({
      session_id: sessionId,
      request: { type: 'json_subtree', meta, path, include_root: true, children: [] },
      format: 'jsonl',
      output_path: out
    });
  }

  async function ensureLoaded(reset = false) {
    if (!open) return;
    if (!canExpand()) return;
    if (loading) return;
    if (reachedEnd) return;

    loading = true;
    err = null;
    try {
      const cursor_offset = reset ? 0 : nextCursorOffset ?? null;
      const cursor_index = reset ? 0 : nextCursorIndex ?? null;
      const res = await jsonListChildrenAtOffset({
        session_id: sessionId,
        meta,
        node_offset: nodeOffset,
        cursor_offset,
        cursor_index,
        limit: 50
      });
      if (reset) items = [];
      items = [...items, ...(res.items ?? [])];
      nextCursorOffset = res.next_cursor_offset ?? null;
      nextCursorIndex = res.next_cursor_index ?? null;
      reachedEnd = Boolean(res.reached_end) || nextCursorOffset === null;
    } catch (e: any) {
      err = String(e?.message ?? e);
    } finally {
      loading = false;
    }
  }

  async function toggleOpen() {
    const sel = globalThis.getSelection?.();
    if (sel && !sel.isCollapsed) return;
    open = !open;
    if (open) {
      await ensureLoaded();
      void ensureSummary();
    }
  }

  async function ensureSummary() {
    if (!open) return;
    if (!canExpand()) return;
    if (summaryLoading) return;
    if (summaryComplete === true) return;
    if (summaryCount !== null && summaryComplete === false) return; // already got a partial count

    summaryLoading = true;
    summaryErr = null;
    try {
      const res = await jsonNodeSummaryAtOffset({
        session_id: sessionId,
        meta,
        node_offset: nodeOffset,
        max_items: 200_000,
        max_scan_bytes: 64 * 1024 * 1024
      });
      summaryCount = res.child_count ?? null;
      summaryComplete = Boolean(res.complete);
    } catch (e: any) {
      summaryErr = String(e?.message ?? e);
    } finally {
      summaryLoading = false;
    }
  }

  function segLabel(seg: string | number) {
    return typeof seg === 'number' ? `[${seg}]` : seg;
  }

  function kindBadge(k: JsonNodeKind) {
    switch (k) {
      case 'object':
        return '{ }';
      case 'array':
        return '[ ]';
      case 'string':
        return 'str';
      case 'number':
        return 'num';
      case 'boolean':
        return 'bool';
      case 'null':
        return 'null';
      default:
        return '?';
    }
  }

  $: loadedCount = items.length;
  $: countText =
    reachedEnd
      ? `共 ${loadedCount}`
      : summaryCount !== null
        ? summaryComplete
          ? `共 ${summaryCount}`
          : `≥ ${summaryCount}`
        : loadedCount > 0
          ? `已加载 ${loadedCount}+`
          : '';
</script>

<div class="jlt-node" role="treeitem" aria-expanded={canExpand() ? open : undefined} aria-selected="false">
  <div class="jlt-row" style={`padding-left: ${depth * indentPx}px`}>
    {#if canExpand()}
      <button type="button" class="jlt-toggle" on:click|stopPropagation={toggleOpen} aria-label={open ? '折叠' : '展开'}>
        {#if open}▾{:else}▸{/if}
      </button>
    {:else}
      <span class="jlt-toggle jlt-toggle-placeholder" aria-hidden="true">•</span>
    {/if}

    <span class="jlt-label">{label}</span>
    {#if kind}
      <span class="jlt-kind">{kindBadge(kind)}</span>
    {/if}
    {#if countText && canExpand()}
      <span class="jlt-count">{countText}</span>
    {/if}
    {#if preview}
      <span class="jlt-preview">{preview}</span>
    {/if}

    {#if canExpand()}
      <button
        type="button"
        class="jlt-action"
        on:click|stopPropagation={() => onExportNodeJsonl()}
        use:tooltip={{ text: '导出该节点（jsonl）' }}
        aria-label="导出该节点（jsonl）"
        disabled={loading}
      >
        ⤓
      </button>
    {/if}

    {#if loading}
      <span class="jlt-muted">加载中…</span>
    {/if}
    {#if err}
      <span class="jlt-err">失败：{err}</span>
    {/if}
    {#if summaryLoading}
      <span class="jlt-muted">统计中…</span>
    {/if}
    {#if summaryErr}
      <span class="jlt-err">统计失败：{summaryErr}</span>
    {/if}
  </div>

  {#if open && canExpand()}
    {#each items as it (typeof it.seg === 'number' ? `i:${it.seg}` : `k:${it.seg}`)}
      <svelte:self
        sessionId={sessionId}
        meta={meta}
        path={[...path, it.seg]}
        nodeOffset={it.value_offset}
        label={segLabel(it.seg)}
        depth={depth + 1}
        indentPx={indentPx}
        kind={it.kind}
        preview={it.preview}
      />
    {/each}

    {#if !reachedEnd}
      <div class="jlt-row" style={`padding-left: ${(depth + 1) * indentPx}px`}>
        <span class="jlt-toggle jlt-toggle-placeholder" aria-hidden="true"> </span>
        <button type="button" class="jlt-more" on:click={() => ensureLoaded(false)} disabled={loading}>
          加载更多…
        </button>
      </div>
    {/if}
  {/if}
</div>

<style>
  .jlt-row {
    display: flex;
    align-items: baseline;
    gap: 8px;
    min-width: 0;
    padding: 2px 0;
    white-space: pre-wrap;
    word-break: break-word;
    overflow-wrap: anywhere;
  }
  .jlt-toggle {
    width: 22px;
    height: 22px;
    padding: 0;
    border: none;
    background: transparent;
    color: #f59e0b;
    cursor: pointer;
    line-height: 22px;
    font-size: calc(var(--jt-font-size, 12px) + 4px);
    font-weight: 700;
    flex: 0 0 auto;
  }
  .jlt-toggle-placeholder {
    color: #374151;
    cursor: default;
  }
  .jlt-label {
    color: #a78bfa;
    flex: 0 0 auto;
  }
  .jlt-kind {
    color: #93c5fd;
    flex: 0 0 auto;
  }
  .jlt-count {
    color: #6b7280;
    font-size: 12px;
    flex: 0 0 auto;
  }
  .jlt-preview {
    color: #e5e7eb;
    min-width: 0;
    opacity: 0.9;
  }
  .jlt-muted {
    color: #6b7280;
    font-size: 12px;
  }
  .jlt-err {
    color: #fda4af;
    font-size: 12px;
  }
  .jlt-more {
    border: 1px solid rgba(255, 255, 255, 0.12);
    background: rgba(17, 24, 39, 0.55);
    color: #e5e7eb;
    border-radius: 8px;
    padding: 4px 8px;
    cursor: pointer;
  }
  .jlt-more:disabled {
    cursor: not-allowed;
    opacity: 0.6;
  }

  .jlt-action {
    border: 1px solid rgba(255, 255, 255, 0.12);
    background: rgba(17, 24, 39, 0.35);
    color: #e5e7eb;
    border-radius: 8px;
    padding: 2px 6px;
    cursor: pointer;
    opacity: 0.0;
    pointer-events: none;
    transition: opacity 120ms ease;
    font-size: 12px;
  }
  .jlt-row:hover .jlt-action,
  .jlt-row:focus-within .jlt-action {
    opacity: 1;
    pointer-events: auto;
  }
  .jlt-action:disabled {
    cursor: not-allowed;
    opacity: 0.4;
    pointer-events: none;
  }
</style>

