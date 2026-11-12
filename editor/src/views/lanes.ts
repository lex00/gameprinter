// Event lanes: where each event falls, within its frame and across the window.

import type { EventJson } from "../api";

export const KINDS = [
  "nmi",
  "irq",
  "sprite0-hit",
  "bank-switch",
  "controller-read",
  "controller-write",
  "ppu-write",
  "apu-write",
] as const;

/** Dots in a frame: 262 scanlines of 341 on NTSC. */
export const FRAME_DOTS = 262 * 341;

/** Where in its frame an event happened, from 0 to 1. The frame starts at vblank. */
export function framePosition(e: EventJson): number {
  const vblank = 241 * 341 + 1;
  const dot = e.scanline * 341 + e.dot;
  return (((dot - vblank) % FRAME_DOTS) + FRAME_DOTS) % FRAME_DOTS / FRAME_DOTS;
}

/** Events per kind per frame over a window, for the lanes' density. */
export function countsByKind(events: EventJson[], start: number, end: number): Map<string, number[]> {
  const out = new Map<string, number[]>();
  for (const k of KINDS) out.set(k, new Array(Math.max(0, end - start)).fill(0));
  for (const e of events) {
    const row = out.get(e.kind);
    if (row && e.frame >= start && e.frame < end) row[e.frame - start]++;
  }
  return out;
}

export function describe(e: EventJson): string {
  const h = (v: number, w: number) => "$" + v.toString(16).padStart(w, "0");
  switch (e.kind) {
    case "nmi":
    case "irq":
      return `${e.kind} -> handler ${h(e.addr, 4)}`;
    case "sprite0-hit":
      return "sprite 0 hit";
    case "controller-read":
      return `read ${h(e.addr, 4)} -> ${h(e.value, 2)}`;
    default:
      return `${e.kind} ${h(e.addr, 4)} = ${h(e.value, 2)}`;
  }
}
