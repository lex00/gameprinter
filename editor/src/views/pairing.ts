// Arithmetic for the side-by-side view: which entities differ, where the
// two runs' frames meet, and which frame ranges differ.

import type { EntityBox, PairJson, PathRow } from "../api";

/** The entity instance a path belongs to: `entities.slug[1].x` -> `entities.slug[1]`. */
export function instanceOf(path: string): string | undefined {
  const m = /^(entities\.[A-Za-z0-9_]+\[\d+\])\./.exec(path);
  return m?.[1];
}

/** The entity instances with a mapped path that differs. */
export function differingInstances(rows: PathRow[]): Set<string> {
  const out = new Set<string>();
  for (const r of rows) {
    const i = instanceOf(r.path);
    if (r.differs && i) out.add(i);
  }
  return out;
}

/** The model frame shown with an original frame. */
export function modelFrameAt(pair: Pick<PairJson, "model_frame"> | undefined, frame: number): number {
  if (!pair || pair.model_frame.length === 0) return frame;
  return pair.model_frame[Math.max(0, Math.min(frame, pair.model_frame.length - 1))];
}

/** The first original frame that shows a model frame, or undefined if none does. */
export function originalFrameOf(pair: Pick<PairJson, "model_frame"> | undefined, modelFrame: number): number | undefined {
  const i = pair?.model_frame.indexOf(modelFrame) ?? -1;
  return i >= 0 ? i : undefined;
}

/** How many original frames lie in the ranges. */
export function differingFrames(ranges: [number, number][]): number {
  return ranges.reduce((n, [a, b]) => n + (b - a + 1), 0);
}

/** The first range starting after `frame`, wrapping to the first. */
export function nextDifference(ranges: [number, number][], frame: number): number | undefined {
  if (ranges.length === 0) return undefined;
  return (ranges.find(([a]) => a > frame) ?? ranges[0])[0];
}

/** Entity boxes with their draw style: `differs` when a mapped path of theirs does. */
export function styled(boxes: EntityBox[], differs: Set<string>): { box: EntityBox; differs: boolean }[] {
  return boxes.map((box) => ({ box, differs: differs.has(box.path) }));
}
