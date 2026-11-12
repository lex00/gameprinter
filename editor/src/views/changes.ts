// Arithmetic for the change view: reading a typed value, building the
// request, and where a variant's entities are drawn over the original.

import type { EditDraft, VariantDraft, VariantRun } from "../api";

/**
 * A typed value as a model value: whole numbers and `true` or `false` as
 * themselves, anything else as a string (fractions are exact decimal
 * strings in a model, such as "0.125"). Empty text is undefined.
 */
export function parseValue(text: string): unknown {
  const t = text.trim();
  if (t === "") return undefined;
  if (/^-?\d+$/.test(t)) return Number(t);
  if (t === "true") return true;
  if (t === "false") return false;
  return t.replace(/^"(.*)"$/, "$1");
}

/** An edit from the two boxes, or undefined while either is empty. */
export function editOf(path: string, value: string): EditDraft | undefined {
  const v = parseValue(value);
  return path.trim() === "" || v === undefined ? undefined : { path: path.trim(), value: v };
}

/** The variants worth running: those with at least one edit. */
export function runnable(variants: VariantDraft[]): VariantDraft[] {
  return variants.filter((v) => v.edits.length > 0);
}

export interface Ghost {
  path: string;
  kind: string;
  x: number;
  y: number;
  w: number;
  h: number;
}

/**
 * The live entities of a run at a model frame as boxes on the picture: each
 * position plus its hitbox, less the camera.
 */
export function ghostsAt(run: VariantRun, modelFrame: number): Ghost[] {
  if (!run.ok || !run.entities || modelFrame < 0 || modelFrame >= (run.frames ?? 0)) return [];
  const [cx, cy] = run.camera?.[modelFrame] ?? [0, 0];
  const out: Ghost[] = [];
  for (const e of run.entities) {
    if (!e.live[modelFrame]) continue;
    const [hx, hy, w, h] = e.hitbox;
    out.push({ path: e.path, kind: e.kind, x: e.x[modelFrame] + hx - cx, y: e.y[modelFrame] + hy - cy, w, h });
  }
  return out;
}

/** The ghost's offset from the unchanged model's entity of the same path, in pixels. */
export function offsetFrom(base: VariantRun, run: VariantRun, path: string, modelFrame: number): { dx: number; dy: number } | undefined {
  const a = ghostsAt(base, modelFrame).find((g) => g.path === path);
  const b = ghostsAt(run, modelFrame).find((g) => g.path === path);
  return a && b ? { dx: b.x - a.x, dy: b.y - a.y } : undefined;
}

/** A line saying how a run parts from the unchanged model. */
export function summary(run: VariantRun): string {
  if (!run.ok) return run.error ?? "refused";
  if (run.first_difference === null || run.first_difference === undefined) return "the same as the model on every frame";
  return `parts from the model at model frame ${run.first_difference}`;
}

export const GHOST_COLORS = ["#d9480f", "#1971c2", "#2f9e44", "#9c36b5", "#c2255c", "#e8590c", "#0b7285", "#5f3dc4"];
