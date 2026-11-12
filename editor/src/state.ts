// A frame's state as the views see it: named fields of bytes.

import type { FrameJson } from "./api";

export type FrameState = Map<string, Uint8Array>;

export function fromHex(hex: string): Uint8Array {
  const out = new Uint8Array(hex.length / 2);
  for (let i = 0; i < out.length; i++) out[i] = parseInt(hex.slice(2 * i, 2 * i + 2), 16);
  return out;
}

export function fromFrameJson(f: FrameJson): FrameState {
  return new Map(Object.entries(f.fields).map(([k, v]) => [k, fromHex(v)]));
}

/** A field of up to 4 bytes as a little-endian number. */
export function int(state: FrameState | undefined, name: string): number | undefined {
  const b = state?.get(name);
  if (!b || b.length === 0 || b.length > 4) return undefined;
  return b.reduceRight((acc, x) => acc * 256 + x, 0);
}

export const hex2 = (v: number) => v.toString(16).padStart(2, "0");
export const hex4 = (v: number) => v.toString(16).padStart(4, "0");
