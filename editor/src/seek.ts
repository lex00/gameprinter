// The browser's side of the seek metric (docs/inspector.md): each seek's
// time from the request to the frame being drawn. The end-to-end test reads
// `window.__gpSeeks` and records the slowest through gp-metrics.

export interface Seek {
  frame: number;
  ms: number;
}

const seeks: Seek[] = [];

declare global {
  interface Window {
    __gpSeeks?: Seek[];
  }
}

export function recordSeek(frame: number, ms: number): void {
  seeks.push({ frame, ms });
  if (typeof window !== "undefined") window.__gpSeeks = seeks;
}

export function seekStats(): { count: number; max: number } {
  return { count: seeks.length, max: seeks.reduce((m, s) => Math.max(m, s.ms), 0) };
}
