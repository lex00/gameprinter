// Labels by address, for the watch list and the heatmap's tooltip.

import type { Label } from "../api";

/** Every label covering each address: a label of size 2 covers two. */
export function labelIndex(labels: Label[]): Map<number, Label[]> {
  const out = new Map<number, Label[]>();
  for (const l of labels) {
    for (let i = 0; i < Math.max(1, l.size); i++) {
      const list = out.get(l.addr + i) ?? [];
      list.push(l);
      out.set(l.addr + i, list);
    }
  }
  return out;
}

/** A label's bytes in RAM, or undefined outside the 2KB. */
export function labelBytes(ram: Uint8Array | undefined, l: Label): number[] | undefined {
  if (!ram || l.addr + l.size > ram.length) return undefined;
  return Array.from(ram.subarray(l.addr, l.addr + Math.max(1, l.size)));
}

/** A label's value as text: one byte as hex and decimal, more as a list. */
export function formatBytes(bytes: number[] | undefined): string {
  if (!bytes) return "-";
  if (bytes.length === 1) return `$${bytes[0].toString(16).padStart(2, "0")} (${bytes[0]})`;
  return bytes.map((b) => b.toString(16).padStart(2, "0")).join(" ");
}
