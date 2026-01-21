import type {
  ExportFormat,
  ExportRequest,
  ExportResult,
  FolderTreeResponse,
  OpenFileResponse,
  PathKind,
  Record,
  RecordMeta,
  RecordPage,
  SearchQuery,
  SearchResult,
  SessionInfo,
  Task
} from '$lib/ipc';

type WebSession = {
  session: SessionInfo;
  records: Record[]; // full dataset records (id stable)
};

type WebTask = {
  task: Task;
  hits: Record[]; // full hit list for paging
};

const sessions = new Map<string, WebSession>();
const tasks = new Map<string, WebTask>();

function nowMs() {
  return Date.now();
}

function uuid() {
  // eslint-disable-next-line @typescript-eslint/no-explicit-any
  return (globalThis as any).crypto?.randomUUID?.() ?? `${Date.now()}-${Math.random().toString(16).slice(2)}`;
}

function truncate(s: string, max = 220) {
  if (s.length <= max) return s;
  return s.slice(0, max) + '…';
}

function inferFormat(path: string): SessionInfo['format'] {
  const p = path.toLowerCase();
  if (p.endsWith('.jsonl') || p.startsWith('demo://jsonl')) return 'jsonl';
  if (p.endsWith('.csv') || p.startsWith('demo://csv')) return 'csv';
  if (p.endsWith('.json') || p.startsWith('demo://json')) return 'json';
  if (p.endsWith('.parquet') || p.startsWith('demo://parquet')) return 'parquet';
  return 'unknown';
}

function demoDatasets(): { path: string; name: string; format: SessionInfo['format']; content: string }[] {
  const jsonl = [
    { id: 1, prompt: 'Hello', response: 'World', meta: { source: 'demo', lang: 'en' } },
    { id: 2, prompt: 'What is 2+2?', response: '4', meta: { source: 'demo', lang: 'en' } },
    { id: 3, prompt: '你好', response: '世界', meta: { source: 'demo', lang: 'zh' } },
    { id: 4, prompt: 'Find keyword: apple', response: 'banana', meta: { tags: ['fruit'] } },
    { id: 5, prompt: 'Long text', response: 'x'.repeat(600), meta: { note: 'to test truncation' } }
  ]
    .map((x) => JSON.stringify(x))
    .join('\n');

  const csv = [
    'id,name,score',
    '1,Alice,98',
    '2,Bob,87',
    '3,Carol,92',
    '4,Dan,87',
    '5,Eve,100'
  ].join('\n');

  const json = JSON.stringify(
    {
      items: [
        { id: 1, title: 'alpha', nested: { ok: true } },
        { id: 2, title: 'beta', nested: { ok: false } },
        { id: 3, title: 'gamma', nested: { ok: true } }
      ]
    },
    null,
    2
  );

  return [
    { path: 'demo://jsonl/training_data.jsonl', name: 'training_data.jsonl', format: 'jsonl', content: jsonl },
    { path: 'demo://csv/scores.csv', name: 'scores.csv', format: 'csv', content: csv },
    { path: 'demo://json/sample.json', name: 'sample.json', format: 'json', content: json }
  ];
}

function parseToRecords(path: string, content: string): Record[] {
  const fmt = inferFormat(path);
  const lines =
    fmt === 'json'
      ? [content] // treat as single record
      : content
          .split(/\r?\n/)
          .map((l) => l.trimEnd())
          .filter((l) => l.length > 0);

  return lines.map((raw, idx) => {
    const meta: RecordMeta = { line_no: idx + 1, byte_offset: 0, byte_len: raw.length };
    return { id: idx, preview: truncate(raw), raw, meta };
  });
}

function pageFrom(records: Record[], cursor: string | null | undefined, pageSize: number | null | undefined): RecordPage {
  const size = Math.max(1, Math.min(200, pageSize ?? 10));
  const start = cursor ? Math.max(0, Number.parseInt(cursor, 10) || 0) : 0;
  const slice = records.slice(start, start + size);
  const next = start + slice.length;
  return {
    records: slice,
    next_cursor: next < records.length ? String(next) : null,
    reached_eof: next >= records.length
  };
}

function downloadText(filename: string, text: string, mime: string) {
  if (typeof document === 'undefined') return;
  const blob = new Blob([text], { type: mime });
  const url = URL.createObjectURL(blob);
  const a = document.createElement('a');
  a.href = url;
  a.download = filename;
  document.body.appendChild(a);
  a.click();
  a.remove();
  URL.revokeObjectURL(url);
}

function ensureSession(sessionId: string): WebSession {
  const s = sessions.get(sessionId);
  if (!s) throw new Error(`Web demo: unknown session_id=${sessionId}`);
  return s;
}

export async function webInvoke<T>(cmd: string, args: any): Promise<T> {
  switch (cmd) {
    case 'open_file': {
      const path: string = args?.path;
      const requestId: string | null = args?.requestId ?? args?.request_id ?? null;
      void requestId; // unused in web demo

      const demo = demoDatasets().find((d) => d.path === path);
      if (!demo) {
        throw new Error(`Web 测试模式仅支持示例数据：请使用“打开示例数据/示例文件夹”。 path=${path}`);
      }
      const session: SessionInfo = {
        session_id: uuid(),
        path,
        format: demo.format,
        created_at_ms: nowMs()
      };
      const records = parseToRecords(path, demo.content);
      sessions.set(session.session_id, { session, records });
      const first_page = pageFrom(records, null, 10);
      return { session, first_page } satisfies OpenFileResponse as T;
    }
    case 'path_kind': {
      const path: string = args?.path;
      if (typeof path === 'string' && path.startsWith('demo://')) return 'file' as PathKind as T;
      return 'missing' as PathKind as T;
    }
    case 'scan_folder_tree': {
      const path: string = args?.path;
      if (path !== 'demo://root') {
        throw new Error('Web 测试模式仅支持示例文件夹：demo://root');
      }
      const children = demoDatasets().map((d) => ({
        name: d.name,
        path: d.path,
        kind: 'file' as const,
        supported: d.format !== 'unknown',
        children: null
      }));
      const tree: FolderTreeResponse = {
        root: { name: 'demo', path: 'demo://root', kind: 'dir', supported: true, children },
        truncated: false,
        total_nodes: 1 + children.length
      };
      return tree as T;
    }
    case 'next_page': {
      const sessionId: string = args?.sessionId ?? args?.session_id;
      const cursor: string | null = args?.cursor ?? null;
      const pageSize: number | null = args?.pageSize ?? args?.page_size ?? null;
      const s = ensureSession(sessionId);
      return pageFrom(s.records, cursor, pageSize) as T;
    }
    case 'get_record_raw': {
      const sessionId: string = args?.sessionId ?? args?.session_id;
      const meta: RecordMeta = args?.meta;
      const s = ensureSession(sessionId);
      const idx = Math.max(0, (meta?.line_no ?? 1) - 1);
      const rec = s.records[idx];
      if (!rec) throw new Error('Web demo: record not found');
      return (rec.raw ?? '') as T;
    }
    case 'search': {
      const sessionId: string = args?.sessionId ?? args?.session_id;
      const query: SearchQuery = args?.query;
      const s = ensureSession(sessionId);

      const q = query?.text ?? '';
      const max = Math.max(1, Math.min(50_000, query?.max_hits ?? 200));
      const caseSensitive = !!query?.case_sensitive;
      const needle = caseSensitive ? q : q.toLowerCase();
      const hitsAll = q
        ? s.records.filter((r) => {
            const hay = caseSensitive ? r.raw ?? r.preview : (r.raw ?? r.preview).toLowerCase();
            return hay.includes(needle);
          })
        : [];

      if (query?.mode === 'scan_all') {
        const id = uuid();
        const task: Task = {
          id,
          kind: 'search_scan_all',
          started_at_ms: nowMs(),
          progress_0_100: 100,
          cancellable: true,
          finished: true,
          error: null
        };
        tasks.set(id, { task, hits: hitsAll.slice(0, max) });
        const out: SearchResult = { mode: 'scan_all', hits: [], task: { id, kind: 'search_scan_all', cancellable: true }, truncated: hitsAll.length > max };
        return out as T;
      }

      const hits = hitsAll.slice(0, max);
      const out: SearchResult = { mode: query?.mode ?? 'current_page', hits, task: null, truncated: hitsAll.length > max };
      return out as T;
    }
    case 'get_task': {
      const taskId: string = args?.taskId ?? args?.task_id;
      const t = tasks.get(taskId)?.task;
      if (!t) throw new Error(`Web demo: unknown task_id=${taskId}`);
      return t as T;
    }
    case 'search_task_hits_page': {
      const taskId: string = args?.taskId ?? args?.task_id;
      const cursor: string | null = args?.cursor ?? null;
      const pageSize: number | null = args?.pageSize ?? args?.page_size ?? null;
      const t = tasks.get(taskId);
      if (!t) throw new Error(`Web demo: unknown task_id=${taskId}`);
      return pageFrom(t.hits, cursor, pageSize) as T;
    }
    case 'cancel_task': {
      const taskId: string = args?.taskId ?? args?.task_id;
      const t = tasks.get(taskId);
      if (t) {
        t.task = { ...t.task, finished: true, error: 'cancelled', progress_0_100: t.task.progress_0_100 };
      }
      return undefined as T;
    }
    case 'export': {
      const inner = args?.args ?? args;
      const sessionId: string = inner?.sessionId ?? inner?.session_id;
      const request: ExportRequest = inner?.request;
      const format: ExportFormat = inner?.format;
      const outputPath: string =
        inner?.outputPath ??
        inner?.output_path ??
        `export.${format === 'csv' ? 'csv' : format === 'json' ? 'json' : 'jsonl'}`;

      const s = ensureSession(sessionId);
      let picked: Record[] = [];
      if (request?.type === 'selection') {
        const set = new Set(request.record_ids);
        picked = s.records.filter((r) => set.has(r.id));
      } else if (request?.type === 'search_task') {
        const t = tasks.get(request.task_id);
        if (!t) throw new Error(`Web demo: unknown search task: ${request.task_id}`);
        picked = t.hits;
      } else if (request?.type === 'json_subtree') {
        // Web demo: pick from the FIRST (and typically only) record.
        const rec = s.records[0];
        if (!rec) throw new Error('Web demo: no records');
        const root = JSON.parse(rec.raw ?? 'null') as unknown;

        const at = (v: any, path: (string | number)[]) => {
          let cur = v;
          for (const seg of path) {
            if (typeof seg === 'number') cur = Array.isArray(cur) ? cur[seg] : undefined;
            else cur = cur && typeof cur === 'object' && !Array.isArray(cur) ? cur[seg] : undefined;
          }
          return cur;
        };

        const subtree = at(root, request.path ?? []);
        let outValues: any[] = [];
        if (request.include_root) {
          outValues = [subtree];
        } else {
          for (const seg of request.children ?? []) {
            if (typeof seg === 'number') outValues.push(Array.isArray(subtree) ? subtree[seg] : undefined);
            else outValues.push(subtree && typeof subtree === 'object' && !Array.isArray(subtree) ? subtree[seg] : undefined);
          }
        }

        if (format === 'jsonl') {
          const jsonl = outValues.map((v) => JSON.stringify(v ?? null)).join('\n');
          downloadText(outputPath, jsonl, 'application/jsonl;charset=utf-8');
          const out: ExportResult = { output_path: outputPath, records_written: outValues.length };
          return out as T;
        }
        if (format === 'json') {
          const json = JSON.stringify(outValues, null, 2);
          downloadText(outputPath, json, 'application/json;charset=utf-8');
          const out: ExportResult = { output_path: outputPath, records_written: outValues.length };
          return out as T;
        }
        throw new Error('Web demo: json_subtree export does not support csv');
      } else {
        throw new Error('Web demo: invalid export request');
      }

      const fmt = s.session.format;
      if (format === 'csv') {
        // naive CSV: single column "raw"
        const csv = ['raw', ...picked.map((r) => JSON.stringify(r.raw ?? ''))].join('\n');
        downloadText(outputPath, csv, 'text/csv;charset=utf-8');
      } else if (format === 'jsonl') {
        if (fmt === 'csv') {
          // Convert CSV rows into jsonl objects using header.
          const lines = s.records.map((r) => r.raw ?? '');
          const header = lines[0] ?? '';
          const headers = header.split(',').map((x) => x.trim());
          const want = new Set(picked.map((r) => r.id));
          const out = lines
            .map((line, idx) => ({ line, idx }))
            .filter(({ idx }) => idx > 0 && want.has(idx))
            .map(({ line }) => {
              const fields = line.split(',');
              const obj: { [k: string]: any } = {};
              for (let i = 0; i < headers.length; i++) obj[headers[i] || `col_${i}`] = fields[i] ?? '';
              return JSON.stringify(obj);
            })
            .join('\n');
          downloadText(outputPath, out, 'application/jsonl;charset=utf-8');
        } else {
          const jsonl = picked.map((r) => r.raw ?? '').join('\n');
          downloadText(outputPath, jsonl, 'application/jsonl;charset=utf-8');
        }
      } else {
        // json array
        if (fmt === 'csv') {
          const lines = s.records.map((r) => r.raw ?? '');
          const header = lines[0] ?? '';
          const headers = header.split(',').map((x) => x.trim());
          const want = new Set(picked.map((r) => r.id));
          const arr = lines
            .map((line, idx) => ({ line, idx }))
            .filter(({ idx }) => idx > 0 && want.has(idx))
            .map(({ line }) => {
              const fields = line.split(',');
              const obj: { [k: string]: any } = {};
              for (let i = 0; i < headers.length; i++) obj[headers[i] || `col_${i}`] = fields[i] ?? '';
              return obj;
            });
          downloadText(outputPath, JSON.stringify(arr, null, 2), 'application/json;charset=utf-8');
        } else {
          const arr = picked.map((r) => {
            try {
              return JSON.parse(r.raw ?? 'null');
            } catch {
              return r.raw ?? null;
            }
          });
          downloadText(outputPath, JSON.stringify(arr, null, 2), 'application/json;charset=utf-8');
        }
      }

      const out: ExportResult = { output_path: outputPath, records_written: picked.length };
      return out as T;
    }
    default:
      throw new Error(`Web demo: unsupported command: ${cmd}`);
  }
}

