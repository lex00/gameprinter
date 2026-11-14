// The curve chart's pure side (CurveChart.tsx): a measure's values as
// points at the clip's frames, the chart's bounds and its lines. Pure, so
// the tests run it on made-up curves.

/** A line's points: the clip's frame and the value. */
export type Points = [number, number][];

/** A series of values as points at the clip's frames, skipping the missing. */
export function points(values: (number | null)[] | undefined, offset: number, frames?: number[]): Points {
  const out: Points = [];
  (values ?? []).forEach((v, k) => {
    if (v != null) out.push([frames?.[k] ?? k + offset, v]);
  });
  return out;
}

/** The bounds every line fits in, with a little room. */
export function bounds(lines: Points[], from: number, to: number): { x0: number; x1: number; y0: number; y1: number } {
  const ys = lines.flatMap((l) => l.filter(([x]) => x >= from && x <= to).map(([, y]) => y));
  let y0 = ys.length ? Math.min(...ys) : 0;
  let y1 = ys.length ? Math.max(...ys) : 1;
  if (y1 - y0 < 1e-9) {
    y0 -= 0.5;
    y1 += 0.5;
  }
  const pad = (y1 - y0) * 0.08;
  return { x0: from, x1: Math.max(to, from + 1), y0: y0 - pad, y1: y1 + pad };
}

/** An SVG polyline's points for a line in a `w` by `h` box. */
export function polyline(l: Points, b: { x0: number; x1: number; y0: number; y1: number }, w: number, h: number): string {
  return l
    .filter(([x]) => x >= b.x0 && x <= b.x1)
    .map(([x, y]) => `${(((x - b.x0) / (b.x1 - b.x0)) * w).toFixed(1)},${(h - ((y - b.y0) / (b.y1 - b.y0)) * h).toFixed(1)}`)
    .join(" ");
}

/** A frame's x in a `w` wide box. */
export function xAt(f: number, b: { x0: number; x1: number }, w: number): number {
  return ((f - b.x0) / (b.x1 - b.x0)) * w;
}
