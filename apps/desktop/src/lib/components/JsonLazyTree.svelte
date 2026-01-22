<script lang="ts">
  import JsonLazyTreeNode from '$lib/components/JsonLazyTreeNode.svelte';
  import type { RecordMeta } from '$lib/ipc';

  export let sessionId: string;
  export let meta: RecordMeta;

  // Root path is always [].
  const rootPath: (string | number)[] = [];
</script>

<div class="jlt-root" role="tree" aria-label="JSON（流式）">
  <JsonLazyTreeNode sessionId={sessionId} meta={meta} path={rootPath} nodeOffset={meta.byte_offset} label="（根）" depth={0} />
</div>

<style>
  .jlt-root {
    font-family: ui-monospace, SFMono-Regular, Menlo, Monaco, Consolas, 'Liberation Mono', 'Courier New', monospace;
    font-size: var(--jt-font-size, 12px);
    line-height: 1.45;
    color: #e5e7eb;
  }
</style>

