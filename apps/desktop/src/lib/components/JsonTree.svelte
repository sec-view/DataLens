<script lang="ts">
  export let value: unknown;
  export let depth = 0;
  export let defaultExpandedDepth = 2;
  export let indentPx = 14;
  export let keyName: string | null = null;
  export let highlightText: string = '';
  export let highlightCaseSensitive: boolean = false;

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
    // Keep readable, avoid huge DOM nodes from escaping; still show full content.
    return JSON.stringify(s);
  }

  function toStringValue(v: unknown): string {
    return typeof v === 'string' ? v : String(v);
  }

  // NOTE: Detail search is pure text substring match. (No special key:value syntax.)

  function splitWithMatches(text: string, query: string, caseSensitive: boolean): { t: string; hit: boolean }[] {
    const q = query.trim();
    if (!q) return [{ t: text, hit: false }];
    const hay = caseSensitive ? text : text.toLowerCase();
    const needle = caseSensitive ? q : q.toLowerCase();
    const out: { t: string; hit: boolean }[] = [];
    let i = 0;
    while (true) {
      const j = hay.indexOf(needle, i);
      if (j < 0) break;
      if (j > i) out.push({ t: text.slice(i, j), hit: false });
      out.push({ t: text.slice(j, j + q.length), hit: true });
      i = j + q.length;
      if (i >= text.length) break;
    }
    if (i < text.length) out.push({ t: text.slice(i), hit: false });
    return out.length === 0 ? [{ t: text, hit: false }] : out;
  }

  function partsForKey(k: string) {
    const q = highlightText.trim();
    if (!q) return [{ t: k, hit: false }];
    return splitWithMatches(k, q, highlightCaseSensitive);
  }

  function partsForLeafText(t: string) {
    const q = highlightText.trim();
    if (!q) return [{ t, hit: false }];
    // For leaf strings, users may type without quotes; accept both.
    const p1 = splitWithMatches(t, q, highlightCaseSensitive);
    if (p1.some((p) => p.hit)) return p1;
    const valQuoted = JSON.stringify(q);
    return splitWithMatches(t, valQuoted, highlightCaseSensitive);
  }

  let k: JsonKind = kindOf(value);
  let entries: [string, unknown][] = [];
  let arr: unknown[] | null = null;

  // Always keep the root branch open so "collapse" means collapsing the *content tree*
  // (children branches), instead of shrinking the whole detail view into a one-line summary.
  let open = depth === 0 ? true : depth < defaultExpandedDepth;

  $: k = kindOf(value);
  $: entries = safeEntries(value);
  $: arr = Array.isArray(value) ? (value as unknown[]) : null;
  $: size = k === 'array' ? (arr?.length ?? 0) : k === 'object' ? entries.length : 0;

  function toggleOpen() {
    // If user is selecting text, don't toggle.
    const sel = globalThis.getSelection?.();
    if (sel && !sel.isCollapsed) return;
    open = !open;
  }

  function onBranchKeyDown(e: KeyboardEvent) {
    if (e.key === 'Enter' || e.key === ' ') {
      e.preventDefault();
      toggleOpen();
    }
  }
</script>

<div class="jt-node" data-depth={depth}>
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

      <div
        class="jt-branch-hit"
        role="button"
        tabindex="0"
        aria-expanded={open}
        on:click={toggleOpen}
        on:keydown={onBranchKeyDown}
      >
        {#if keyName !== null}
          <span class="jt-key">
            {#each partsForKey(keyName) as p, i (i)}
              {#if p.hit}
                <mark class="jt-hit">{p.t}</mark>
              {:else}
                {p.t}
              {/if}
            {/each}
          </span>
          <span class="jt-punc">:</span>
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
      </div>
    </div>

    {#if open}
      {#if k === 'object'}
        {#each entries as [kk, vv] (kk)}
          <svelte:self
            value={vv}
            keyName={kk}
            depth={depth + 1}
            defaultExpandedDepth={defaultExpandedDepth}
            indentPx={indentPx}
            highlightText={highlightText}
            highlightCaseSensitive={highlightCaseSensitive}
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
            depth={depth + 1}
            defaultExpandedDepth={defaultExpandedDepth}
            indentPx={indentPx}
            highlightText={highlightText}
            highlightCaseSensitive={highlightCaseSensitive}
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
      {#if keyName !== null}
        <span class="jt-key">
          {#each partsForKey(keyName) as p, i (i)}
            {#if p.hit}
              <mark class="jt-hit">{p.t}</mark>
            {:else}
              {p.t}
            {/if}
          {/each}
        </span>
        <span class="jt-punc">:</span>
      {/if}

      {#if k === 'string'}
        <span class="jt-string">
          {#each partsForLeafText(formatString(toStringValue(value))) as p, i (i)}
            {#if p.hit}
              <mark class="jt-hit">{p.t}</mark>
            {:else}
              {p.t}
            {/if}
          {/each}
        </span>
      {:else if k === 'number'}
        <span class="jt-number">
          {#each partsForLeafText(String(value)) as p, i (i)}
            {#if p.hit}
              <mark class="jt-hit">{p.t}</mark>
            {:else}
              {p.t}
            {/if}
          {/each}
        </span>
      {:else if k === 'boolean'}
        <span class="jt-boolean">
          {#each partsForLeafText(String(value)) as p, i (i)}
            {#if p.hit}
              <mark class="jt-hit">{p.t}</mark>
            {:else}
              {p.t}
            {/if}
          {/each}
        </span>
      {:else if k === 'null'}
        <span class="jt-null">
          {#each partsForLeafText('null') as p, i (i)}
            {#if p.hit}
              <mark class="jt-hit">{p.t}</mark>
            {:else}
              {p.t}
            {/if}
          {/each}
        </span>
      {:else}
        <span class="jt-unknown">
          {#each partsForLeafText(String(value)) as p, i (i)}
            {#if p.hit}
              <mark class="jt-hit">{p.t}</mark>
            {:else}
              {p.t}
            {/if}
          {/each}
        </span>
      {/if}
    </div>
  {/if}
</div>

<style>
  .jt-node {
    font-family: ui-monospace, SFMono-Regular, Menlo, Monaco, Consolas, 'Liberation Mono', 'Courier New', monospace;
    font-size: var(--jt-font-size, 12px);
    line-height: 1.45;
    color: #e5e7eb;
  }

  .jt-row {
    display: flex;
    align-items: flex-start;
    gap: 6px;
    min-width: 0;
    padding: 2px 0;
    white-space: pre-wrap;
    word-break: break-word;
    overflow-wrap: anywhere;
  }

  .jt-branch-hit {
    display: inline-flex;
    align-items: baseline;
    gap: 6px;
    min-width: 0;
    cursor: pointer;
    border-radius: 6px;
    padding: 2px 4px;
  }

  .jt-branch-hit:hover {
    background: rgba(147, 197, 253, 0.08);
  }

  mark.jt-hit {
    background: rgba(250, 204, 21, 0.25);
    color: inherit;
    border-radius: 3px;
    padding: 0 1px;
  }

  .jt-branch-hit:focus-visible {
    outline: 2px solid #60a5fa;
    outline-offset: 1px;
  }

  .jt-toggle {
    width: 22px;
    height: 22px;
    padding: 0;
    border: none;
    background: transparent;
    color: var(--jt-toggle-color, #f59e0b);
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
    color: #374151;
    cursor: default;
  }

  .jt-key {
    color: #a78bfa;
    flex: 0 0 auto;
  }

  .jt-punc {
    color: #6b7280;
  }

  .jt-brace {
    color: #93c5fd;
  }

  .jt-summary {
    color: #6b7280;
  }

  .jt-string {
    color: #86efac;
    min-width: 0;
  }

  .jt-number {
    color: #fbbf24;
    min-width: 0;
  }

  .jt-boolean {
    color: #67e8f9;
    min-width: 0;
  }

  .jt-null {
    color: #cbd5e1;
    min-width: 0;
  }

  .jt-unknown {
    color: #fda4af;
    min-width: 0;
  }

</style>

