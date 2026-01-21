/// <reference lib="webworker" />

type Req = {
  text: string;
  query: string;
  caseSensitive: boolean;
  // Only count matches whose start index is < limitStartInFullText (dedupe chunk overlaps).
  baseStart: number;
  limitStartInFullText: number;
};

type Res = {
  count: number;
};

function countOccurrences(text: string, query: string, caseSensitive: boolean, baseStart: number, limitStart: number): number {
  const q = query.trim();
  if (!q) return 0;
  const hay = caseSensitive ? text : text.toLowerCase();
  const needle = caseSensitive ? q : q.toLowerCase();

  let count = 0;
  let i = 0;
  while (true) {
    const j = hay.indexOf(needle, i);
    if (j < 0) break;
    const globalJ = baseStart + j;
    if (globalJ < limitStart) count++;
    i = j + needle.length;
    if (i >= hay.length) break;
  }
  return count;
}

self.onmessage = (ev: MessageEvent<Req>) => {
  const { text, query, caseSensitive, baseStart, limitStartInFullText } = ev.data;
  const count = countOccurrences(text, query, caseSensitive, baseStart, limitStartInFullText);
  const res: Res = { count };
  // eslint-disable-next-line no-restricted-globals
  (self as unknown as DedicatedWorkerGlobalScope).postMessage(res);
};

export {};

