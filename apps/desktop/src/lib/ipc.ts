import { invokeCompat } from '$lib/platform';

export type FileFormat = 'jsonl' | 'csv' | 'json' | 'parquet' | 'unknown';

export interface SessionInfo {
  session_id: string;
  path: string;
  format: FileFormat;
  created_at_ms: number;
}

export interface RecordMeta {
  line_no: number;
  byte_offset: number;
  byte_len: number;
}

export interface Record {
  id: number;
  preview: string;
  raw: string | null;
  meta: RecordMeta | null;
}

export interface RecordPage {
  records: Record[];
  next_cursor: string | null;
  reached_eof: boolean;
}

export type SearchMode = 'current_page' | 'scan_all' | 'indexed';

export interface SearchQuery {
  text: string;
  mode: SearchMode;
  case_sensitive: boolean;
  max_hits: number;
}

export type TaskKind = 'search_scan_all' | 'export';

export interface TaskInfo {
  id: string;
  kind: TaskKind;
  cancellable: boolean;
}

export interface SearchResult {
  mode: SearchMode;
  hits: Record[];
  task: TaskInfo | null;
  truncated: boolean;
}

export interface Task {
  id: string;
  kind: TaskKind;
  started_at_ms: number;
  progress_0_100: number;
  cancellable: boolean;
  finished: boolean;
  error: string | null;
}

export type ExportFormat = 'json' | 'jsonl' | 'csv';

export type ExportRequest =
  | { type: 'selection'; record_ids: number[] }
  | { type: 'search_task'; task_id: string }
  | {
      type: 'json_subtree';
      meta: RecordMeta;
      path: (string | number)[];
      include_root: boolean;
      children: (string | number)[];
    };

export interface ExportResult {
  output_path: string;
  records_written: number;
}

export type JsonNodeKind = 'object' | 'array' | 'string' | 'number' | 'boolean' | 'null' | 'unknown';

export interface JsonChildItem {
  seg: string | number;
  kind: JsonNodeKind;
  preview: string;
}

export interface JsonChildrenPage {
  items: JsonChildItem[];
  next_cursor: number | null;
  reached_end: boolean;
}

export interface JsonNodeSummary {
  kind: JsonNodeKind;
  child_count: number | null;
  complete: boolean;
}

export interface JsonChildItemOffset {
  seg: string | number;
  kind: JsonNodeKind;
  preview: string;
  value_offset: number;
}

export interface JsonChildrenPageOffset {
  items: JsonChildItemOffset[];
  next_cursor_offset: number | null;
  next_cursor_index: number | null;
  reached_end: boolean;
}

export interface JsonNodeSummaryOffset {
  kind: JsonNodeKind;
  child_count: number | null;
  complete: boolean;
  node_offset: number;
}

export interface OpenFileResponse {
  session: SessionInfo;
  first_page: RecordPage;
}

export type FsNodeKind = 'dir' | 'file';

export interface FsNode {
  name: string;
  path: string;
  kind: FsNodeKind;
  supported: boolean;
  children?: FsNode[] | null;
}

export interface FolderTreeResponse {
  root: FsNode;
  truncated: boolean;
  total_nodes: number;
}

export type PathKind = 'file' | 'dir' | 'missing' | 'other';

export async function openFile(path: string, request_id?: string): Promise<OpenFileResponse> {
  return await invokeCompat('open_file', {
    path,
    requestId: request_id ?? null,
    request_id: request_id ?? null
  });
}

export async function pathKind(path: string): Promise<PathKind> {
  return await invokeCompat('path_kind', { path });
}

export async function scanFolderTree(args: {
  path: string;
  max_depth?: number | null;
  max_nodes?: number | null;
}): Promise<FolderTreeResponse> {
  return await invokeCompat('scan_folder_tree', {
    path: args.path,
    maxDepth: args.max_depth ?? null,
    max_depth: args.max_depth ?? null,
    maxNodes: args.max_nodes ?? null,
    max_nodes: args.max_nodes ?? null
  });
}

export async function nextPage(args: {
  session_id: string;
  cursor?: string | null;
  page_size?: number;
}): Promise<RecordPage> {
  return await invokeCompat('next_page', {
    // Tauri command args are validated by key name; some environments expect camelCase.
    // Send both for compatibility.
    sessionId: args.session_id,
    session_id: args.session_id,
    cursor: args.cursor ?? null,
    pageSize: args.page_size ?? null,
    page_size: args.page_size ?? null
  });
}

export async function getRecordRaw(args: { session_id: string; meta: RecordMeta }): Promise<string> {
  return await invokeCompat('get_record_raw', {
    sessionId: args.session_id,
    session_id: args.session_id,
    meta: args.meta
  });
}

export async function search(args: { session_id: string; query: SearchQuery }): Promise<SearchResult> {
  return await invokeCompat('search', {
    sessionId: args.session_id,
    session_id: args.session_id,
    query: args.query
  });
}

export async function getTask(task_id: string): Promise<Task> {
  return await invokeCompat('get_task', { taskId: task_id, task_id });
}

export async function searchTaskHitsPage(args: {
  task_id: string;
  cursor?: string | null;
  page_size?: number;
}): Promise<RecordPage> {
  return await invokeCompat('search_task_hits_page', {
    taskId: args.task_id,
    task_id: args.task_id,
    cursor: args.cursor ?? null,
    pageSize: args.page_size ?? null,
    page_size: args.page_size ?? null
  });
}

export async function cancelTask(task_id: string): Promise<void> {
  await invokeCompat('cancel_task', { taskId: task_id, task_id });
}

export async function exportToFile(args: {
  session_id: string;
  request: ExportRequest;
  format: ExportFormat;
  output_path: string;
}): Promise<ExportResult> {
  return await invokeCompat('export', {
    args: {
      sessionId: args.session_id,
      session_id: args.session_id,
      request: args.request,
      format: args.format,
      outputPath: args.output_path,
      output_path: args.output_path
    }
  });
}

export async function jsonListChildren(args: {
  session_id: string;
  meta: RecordMeta;
  path: (string | number)[];
  cursor?: number | null;
  limit?: number | null;
}): Promise<JsonChildrenPage> {
  return await invokeCompat('json_list_children', {
    args: {
      sessionId: args.session_id,
      session_id: args.session_id,
      meta: args.meta,
      path: args.path,
      cursor: args.cursor ?? null,
      limit: args.limit ?? null
    }
  });
}

export async function jsonNodeSummary(args: {
  session_id: string;
  meta: RecordMeta;
  path: (string | number)[];
  max_items?: number | null;
  max_scan_bytes?: number | null;
}): Promise<JsonNodeSummary> {
  return await invokeCompat('json_node_summary', {
    args: {
      sessionId: args.session_id,
      session_id: args.session_id,
      meta: args.meta,
      path: args.path,
      max_items: args.max_items ?? null,
      max_scan_bytes: args.max_scan_bytes ?? null
    }
  });
}

export async function jsonListChildrenAtOffset(args: {
  session_id: string;
  meta: RecordMeta;
  node_offset: number;
  cursor_offset?: number | null;
  cursor_index?: number | null;
  limit?: number | null;
}): Promise<JsonChildrenPageOffset> {
  return await invokeCompat('json_list_children_at_offset', {
    args: {
      sessionId: args.session_id,
      session_id: args.session_id,
      meta: args.meta,
      node_offset: args.node_offset,
      cursor_offset: args.cursor_offset ?? null,
      cursor_index: args.cursor_index ?? null,
      limit: args.limit ?? null
    }
  });
}

export async function jsonNodeSummaryAtOffset(args: {
  session_id: string;
  meta: RecordMeta;
  node_offset: number;
  max_items?: number | null;
  max_scan_bytes?: number | null;
}): Promise<JsonNodeSummaryOffset> {
  return await invokeCompat('json_node_summary_at_offset', {
    args: {
      sessionId: args.session_id,
      session_id: args.session_id,
      meta: args.meta,
      node_offset: args.node_offset,
      max_items: args.max_items ?? null,
      max_scan_bytes: args.max_scan_bytes ?? null
    }
  });
}

export async function takePendingOpenPaths(): Promise<string[]> {
  return await invokeCompat('take_pending_open_paths', {});
}

