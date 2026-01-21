<script lang="ts">
  import type { FsNode } from '$lib/ipc';

  export let node: FsNode;
  export let depth = 0;
  export let expanded: Set<string>;
  export let activePath: string | null = null;
  export let busy = false;
  export let onToggleDir: (path: string) => void;
  export let onClickFile: (node: FsNode) => void;

  $: isDir = node.kind === 'dir';
  $: isOpen = isDir && expanded.has(node.path);
  $: children = (node.children ?? []) as FsNode[];

  function onDirClick() {
    onToggleDir(node.path);
  }
</script>

<div
  class="row"
  role="treeitem"
  aria-expanded={isDir ? isOpen : undefined}
  aria-selected={activePath === node.path}
  style={`padding-left: ${depth * 12}px`}
>
  {#if isDir}
    <button class="twisty" type="button" on:click={onDirClick} disabled={busy} aria-label={isOpen ? '收起' : '展开'}>
      {#if isOpen}▾{:else}▸{/if}
    </button>
    <button class="name dir" type="button" on:click={onDirClick} disabled={busy} title={node.path}>
      {node.name}
    </button>
  {:else}
    <span class="twisty-spacer" aria-hidden="true" />
    <button
      class="name file {node.supported ? '' : 'unsupported'} {activePath === node.path ? 'active' : ''}"
      type="button"
      on:click={() => onClickFile(node)}
      disabled={busy || !node.supported}
      title={node.path}
    >
      {node.name}
    </button>
  {/if}
</div>

{#if isDir && isOpen && children.length > 0}
  <div role="group">
    {#each children as c (c.path)}
      <svelte:self
        node={c}
        depth={depth + 1}
        expanded={expanded}
        activePath={activePath}
        busy={busy}
        onToggleDir={onToggleDir}
        onClickFile={onClickFile}
      />
    {/each}
  </div>
{/if}

<style>
  .row {
    display: grid;
    grid-template-columns: 16px 1fr;
    align-items: center;
    gap: 6px;
    padding: 4px 6px;
    border-radius: 8px;
  }
  .row:hover {
    background: #f3f4f6;
  }
  .twisty {
    width: 16px;
    height: 16px;
    display: grid;
    place-items: center;
    border: none;
    padding: 0;
    background: transparent;
    cursor: pointer;
    color: #6b7280;
  }
  .twisty:disabled {
    cursor: default;
    opacity: 0.4;
  }
  .twisty-spacer {
    width: 16px;
    height: 16px;
    display: inline-block;
  }
  .name {
    border: none;
    background: transparent;
    padding: 0;
    text-align: left;
    cursor: pointer;
    font-size: 12px;
    color: #111827;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .name:disabled {
    cursor: not-allowed;
  }
  .name.unsupported {
    color: #9ca3af;
  }
  .name.active {
    color: #2563eb;
    font-weight: 600;
  }
  .dir {
    color: #374151;
  }
</style>

