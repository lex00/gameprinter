// Box selection arithmetic: a drag on the 256x240 picture as a rectangle in
// picture pixels, the way the select endpoint takes it.

import type { Box } from "../api";

export const SCREEN_W = 256;
export const SCREEN_H = 240;

/** A point in picture pixels from a pointer position over an element drawn at `bounds`. */
export function toPicture(clientX: number, clientY: number, bounds: { left: number; top: number; width: number; height: number }): { x: number; y: number } {
  const x = ((clientX - bounds.left) / Math.max(1, bounds.width)) * SCREEN_W;
  const y = ((clientY - bounds.top) / Math.max(1, bounds.height)) * SCREEN_H;
  return { x: clamp(Math.floor(x), 0, SCREEN_W - 1), y: clamp(Math.floor(y), 0, SCREEN_H - 1) };
}

/** The box between two corners of a drag, both pixels included, whichever way it went. */
export function dragBox(a: { x: number; y: number }, b: { x: number; y: number }): Box {
  const x0 = clamp(Math.min(a.x, b.x), 0, SCREEN_W - 1);
  const y0 = clamp(Math.min(a.y, b.y), 0, SCREEN_H - 1);
  const x1 = clamp(Math.max(a.x, b.x), 0, SCREEN_W - 1);
  const y1 = clamp(Math.max(a.y, b.y), 0, SCREEN_H - 1);
  return { x: x0, y: y0, w: x1 - x0 + 1, h: y1 - y0 + 1 };
}

/** `x,y,w,h`, as the URL and the endpoint write a box. */
export function boxText(b: Box): string {
  return `${b.x},${b.y},${b.w},${b.h}`;
}

/** A box from `x,y,w,h`, or undefined if it isn't one on the picture. */
export function parseBox(text: string | null | undefined): Box | undefined {
  if (!text) return undefined;
  const n = text.split(",").map(Number);
  if (n.length !== 4 || !n.every(Number.isInteger)) return undefined;
  const [x, y, w, h] = n;
  if (x < 0 || y < 0 || x >= SCREEN_W || y >= SCREEN_H || w < 1 || h < 1) return undefined;
  return { x, y, w: Math.min(w, SCREEN_W - x), h: Math.min(h, SCREEN_H - y) };
}

function clamp(v: number, lo: number, hi: number): number {
  return Math.max(lo, Math.min(hi, v));
}
