// The RAM heatmap's arithmetic, kept apart from drawing so it can be tested.

/** Columns in the heatmap: 2KB of RAM is 32 rows of 64 bytes. */
export const COLUMNS = 64;

export type HeatMode = "value" | "activity";

/** A byte's value as a colour: dark for 0, bright for 255, hue by high bits. */
export function valueColor(v: number): string {
  if (v === 0) return "rgb(20,22,28)";
  const hue = 200 - (v >> 4) * 12;
  const light = 22 + Math.round((v / 255) * 50);
  return `hsl(${hue},70%,${light}%)`;
}

/** How often each byte changed, from frame to frame, over a run of states. */
export function activity(states: Uint8Array[]): Uint16Array {
  const len = states[0]?.length ?? 0;
  const out = new Uint16Array(len);
  for (let i = 1; i < states.length; i++) {
    const a = states[i - 1];
    const b = states[i];
    for (let k = 0; k < len; k++) if (a[k] !== b[k]) out[k]++;
  }
  return out;
}

/** An activity count as a colour, against the window's frame count. */
export function activityColor(count: number, frames: number): string {
  if (count === 0) return "rgb(20,22,28)";
  const t = Math.min(1, count / Math.max(1, frames - 1));
  return `hsl(${40 - t * 40},90%,${30 + t * 35}%)`;
}

export function addressAt(x: number, y: number, cell: number): number | undefined {
  const col = Math.floor(x / cell);
  const row = Math.floor(y / cell);
  if (col < 0 || col >= COLUMNS || row < 0 || row >= 2048 / COLUMNS) return undefined;
  return row * COLUMNS + col;
}
