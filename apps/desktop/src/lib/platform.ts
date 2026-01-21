import { webInvoke } from '$lib/web_backend';

export function isTauri(): boolean {
  // Avoid redeclaring `Window.__TAURI_IPC__` here (Tauri's type defs already declare it).
  // eslint-disable-next-line @typescript-eslint/no-explicit-any
  const w = window as any;
  return typeof window !== 'undefined' && typeof w.__TAURI_IPC__ === 'function';
}

export async function invokeCompat<T>(cmd: string, args?: Record<string, unknown>): Promise<T> {
  if (isTauri()) {
    const mod = await import('@tauri-apps/api/tauri');
    return await mod.invoke<T>(cmd, args);
  }
  return await webInvoke<T>(cmd, args);
}

export async function dialogOpen(options: { multiple?: boolean; directory?: boolean }): Promise<string | string[] | null> {
  if (!isTauri()) throw new Error('Web 测试模式不支持系统打开对话框，请使用“打开示例数据/示例文件夹”。');
  const mod = await import('@tauri-apps/api/dialog');
  return (await mod.open(options)) as string | string[] | null;
}

export async function dialogSave(options: { defaultPath?: string }): Promise<string | null> {
  if (!isTauri()) {
    // In web demo, we treat this as "use default filename" and trigger download later.
    return options.defaultPath ?? 'export.jsonl';
  }
  const mod = await import('@tauri-apps/api/dialog');
  return (await mod.save(options)) as string | null;
}

export async function eventListen<T>(event: string, handler: (e: { payload: T }) => void): Promise<() => void> {
  if (!isTauri()) return () => {};
  const mod = await import('@tauri-apps/api/event');
  return await mod.listen<T>(event, handler);
}

export async function clipboardWriteText(text: string): Promise<void> {
  if (isTauri()) {
    const mod = await import('@tauri-apps/api/clipboard');
    await mod.writeText(text);
    return;
  }
  if (typeof navigator !== 'undefined' && navigator.clipboard?.writeText) {
    await navigator.clipboard.writeText(text);
    return;
  }
  throw new Error('复制失败：当前环境不支持剪贴板写入。');
}

