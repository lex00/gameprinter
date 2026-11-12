// Where the two games' pictures differ, drawn over the screen as thin
// outlines (screenfold.ts; docs/show.md): the characters' boxes in one
// color, the background's in another. It sits over the original's whole
// frame, 256 by 240 on the NES, and takes no clicks.

import type { Drawn } from "./screenfold";

export function ScreenBoxes({ boxes, w = 256, h = 240 }: { boxes: Drawn[]; w?: number; h?: number }) {
  return (
    <svg className="screen-boxes" viewBox={`0 0 ${w} ${h}`} preserveAspectRatio="none" aria-hidden="true" data-testid="screen-boxes">
      {boxes.map((b, i) => (
        <rect key={i} className={`screen-box ${b.layer}`} x={b.x + 0.5} y={b.y + 0.5} width={Math.max(0, b.w - 1)} height={Math.max(0, b.h - 1)} data-layer={b.layer} />
      ))}
    </svg>
  );
}
