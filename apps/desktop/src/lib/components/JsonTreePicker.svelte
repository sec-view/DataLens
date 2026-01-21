<script context="module" lang="ts">
  export type JsonPath = (string | number)[];
</script>

<script lang="ts">
  export let value: unknown;
  export let depth = 0;
  export let defaultExpandedDepth = 2;
  export let indentPx = 14;

  // For display only (root uses null).
  export let keyName: string | null = null;
  export let keyType: 'root' | 'key' | 'index' = 'root';

  export let path: JsonPath = [];
  export let selectedPath: JsonPath = [];
  export let onSelect: (p: JsonPath) => void = () => {};

  type JsonKind = 'null' | 'array' | 'object' | 'string' | 'number' | 'boolean' | 'unknown';

  function kindOf(v: unknown): JsonKind {
    if (v === null) return 'null';
    if (Array.isArray(v)) return 'array';
    switch (typeof v) {
      case 'string':
        return 'string';
      case 'number':
        return 'number';
      case 'boolean':
        return 'boolean';
      case 'object':
        return 'object';
      default:
        return 'unknown';
    }
  }

  function safeEntries(v: unknown): [string, unknown][] {
    if (!v || typeof v !== 'object' || Array.isArray(v)) return [];
    return Object.entries(v as Record<string, unknown>);
  }

  function formatString(s: string) {
    return JSON.stringify(s);
  }

  function toStringValue(v: unknown): string {
    return typeof v === 'string' ? v : String(v);
  }

  function pathEq(a: JsonPath, b: JsonPath) {
    if (a === b) return true;
    if (a.length !== b.length) return false;
    for (let i = 0; i < a.length; i++) if (a[i] !== b[i]) return false;
    return true;
  }

  function asPathSegment(): string | number | null {
    if (keyType === 'root') return null;
    if (keyName === null) return null;
    if (keyType === 'index') return Number(keyName);
    return keyName;
  }

  $: seg = asPathSegment();
  $: myPath = seg === null ? path : [...path, seg];
  $: isSelected = pathEq(myPath, selectedPath);

  let k: JsonKind = kindOf(value);
  let entries: [string, unknown][] = [];
  let arr: unknown[] | null = null;

  // Root is always open. Others: open by default to a depth, but user can toggle.
  let open = depth === 0 ? true : depth < defaultExpandedDepth;

  $: k = kindOf(value);
  $: entries = safeEntries(value);
  $: arr = Array.isArray(value) ? (value as unknown[]) : null;
  $: size = k === 'array' ? (arr?.length ?? 0) : k === 'object' ? entries.length : 0;

  function toggleOpen() {
    const sel = globalThis.getSelection?.();
    if (sel && !sel.isCollapsed) return;
    open = !open;
  }

  function onPick(e: MouseEvent) {
    e.stopPropagation();
    onSelect(myPath);
  }

  function onRowKeyDown(e: KeyboardEvent) {
    if (e.key === 'Enter') {
      e.preventDefault();
      onSelect(myPath);
    } else if (e.key === ' ' && (k === 'object' || k === 'array')) {
      e.preventDefault();
      toggleOpen();
    }
  }
</script>

<div class="jt-node" data-depth={depth} data-selected={isSelected ? '1' : '0'}>
  {#if k === 'object' || k === 'array'}
    <div class="jt-row" style={`padding-left: ${depth * indentPx}px`}>
      <button
        type="button"
        class="jt-toggle"
        aria-label={open ? '折叠' : '展开'}
        on:click={(e) => {
          e.stopPropagation();
          toggleOpen();
        }}
      >
        {#if open}▾{:else}▸{/if}
      </button>

      <button
        type="button"
        class="jt-pick {isSelected ? 'selected' : ''}"
        aria-label="选择该节点作为视图根"
        aria-pressed={isSelected}
        on:click={onPick}
        on:keydown={onRowKeyDown}
      >
        {#if depth === 0 && keyType === 'root'}
          <span class="jt-root">（根）</span><span class="jt-punc">:</span>
        {:else if keyName !== null}
          <span class="jt-key">{keyName}</span><span class="jt-punc">:</span>
        {/if}

        {#if k === 'object'}
          <span class="jt-brace">{'{'}</span>
          {#if !open}
            <span class="jt-summary">… {size} 键 …</span>
            <span class="jt-brace">{'}'}</span>
          {/if}
        {:else}
          <span class="jt-brace">[</span>
          {#if !open}
            <span class="jt-summary">… {size} 项 …</span>
            <span class="jt-brace">]</span>
          {/if}
        {/if}
      </button>
    </div>

    {#if open}
      {#if k === 'object'}
        {#each entries as [kk, vv] (kk)}
          <svelte:self
            value={vv}
            keyName={kk}
            keyType="key"
            path={myPath}
            selectedPath={selectedPath}
            onSelect={onSelect}
            depth={depth + 1}
            defaultExpandedDepth={defaultExpandedDepth}
            indentPx={indentPx}
          />
        {/each}
        <div class="jt-row" style={`padding-left: ${depth * indentPx}px`}>
          <span class="jt-toggle jt-toggle-placeholder" aria-hidden="true"> </span>
          <span class="jt-brace">{'}'}</span>
        </div>
      {:else}
        {#each arr ?? [] as vv, i (i)}
          <svelte:self
            value={vv}
            keyName={String(i)}
            keyType="index"
            path={myPath}
            selectedPath={selectedPath}
            onSelect={onSelect}
            depth={depth + 1}
            defaultExpandedDepth={defaultExpandedDepth}
            indentPx={indentPx}
          />
        {/each}
        <div class="jt-row" style={`padding-left: ${depth * indentPx}px`}>
          <span class="jt-toggle jt-toggle-placeholder" aria-hidden="true"> </span>
          <span class="jt-brace">]</span>
        </div>
      {/if}
    {/if}
  {:else}
    <div class="jt-row jt-leaf" style={`padding-left: ${depth * indentPx}px`}>
      <span class="jt-toggle jt-toggle-placeholder" aria-hidden="true">•</span>
      <button
        type="button"
        class="jt-pick jt-pick-leaf {isSelected ? 'selected' : ''}"
        aria-label="选择该节点作为视图根"
        aria-pressed={isSelected}
        on:click={onPick}
        on:keydown={onRowKeyDown}
      >
        {#if depth === 0 && keyType === 'root'}
          <span class="jt-root">（根）</span><span class="jt-punc">:</span>
        {:else if keyName !== null}
          <span class="jt-key">{keyName}</span><span class="jt-punc">:</span>
        {/if}

        {#if k === 'string'}
          <span class="jt-string">{formatString(toStringValue(value))}</span>
        {:else if k === 'number'}
          <span class="jt-number">{String(value)}</span>
        {:else if k === 'boolean'}
          <span class="jt-boolean">{String(value)}</span>
        {:else if k === 'null'}
          <span class="jt-null">null</span>
        {:else}
          <span class="jt-unknown">{String(value)}</span>
        {/if}
      </button>
    </div>
  {/if}
</div>

<style>
  .jt-node {
    font-family: ui-monospace, SFMono-Regular, Menlo, Monaco, Consolas, 'Liberation Mono', 'Courier New', monospace;
    font-size: var(--jt-font-size, 12px);
    line-height: 1.45;
    color: #111827;
  }

  .jt-row {
    display: flex;
    align-items: flex-start;
    gap: 6px;
    min-width: 0;
    padding: 2px 0;
    white-space: pre-wrap;
    word-break: break-word;
  }

  .jt-toggle {
    width: 22px;
    height: 22px;
    padding: 0;
    border: none;
    background: transparent;
    color: #6b7280;
    cursor: pointer;
    line-height: 22px;
    font-size: calc(var(--jt-font-size, 12px) + 4px);
    font-weight: 700;
    flex: 0 0 auto;
  }

  .jt-toggle:focus-visible {
    outline: 2px solid #60a5fa;
    border-radius: 4px;
    outline-offset: 1px;
  }

  .jt-toggle-placeholder {
    color: #e5e7eb;
    cursor: default;
  }

  .jt-pick {
    border: none;
    background: transparent;
    padding: 2px 4px;
    border-radius: 8px;
    cursor: pointer;
    display: inline-flex;
    align-items: baseline;
    gap: 6px;
    min-width: 0;
    text-align: left;
  }

  .jt-pick:hover {
    background: rgba(59, 130, 246, 0.08);
  }

  .jt-pick:focus-visible {
    outline: 2px solid #60a5fa;
    outline-offset: 1px;
  }

  .jt-pick.selected {
    background: rgba(59, 130, 246, 0.14);
    outline: 1px solid rgba(59, 130, 246, 0.35);
  }

  .jt-root {
    color: #111827;
    font-weight: 600;
  }

  .jt-key {
    color: #7c3aed;
    flex: 0 0 auto;
  }

  .jt-punc {
    color: #6b7280;
  }

  .jt-brace {
    color: #2563eb;
  }

  .jt-summary {
    color: #6b7280;
  }

  .jt-string {
    color: #16a34a;
  }

  .jt-number {
    color: #b45309;
  }

  .jt-boolean {
    color: #0891b2;
  }

  .jt-null {
    color: #334155;
  }

  .jt-unknown {
    color: #be123c;
  }
</style>

