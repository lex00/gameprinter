// Event lanes: one lane per event kind. The strip shows the current
// frame's events by where in the frame they happened (vblank at the left);
// the window below shows each kind's count per frame over the 64-frame
// window, and clicking a frame there goes to it.

import type { EventJson } from "../api";
import { eventWindow, type Pane } from "../pane";
import { countsByKind, describe, framePosition, KINDS } from "./lanes";
import { registerView, type ViewProps } from "./registry";

const LANE = 14;
const WIDTH = 640;

function Lanes({ pane, setFrame }: { pane: Pane; setFrame: (f: number) => void }) {
  const events: EventJson[] = pane.events?.events ?? [];
  const here = events.filter((e) => e.frame === pane.frame);
  const [start, end] = eventWindow(pane.frame, pane.manifest.frames);
  const counts = countsByKind(events, start, end);
  const span = Math.max(1, end - start);
  const max = Math.max(1, ...Array.from(counts.values()).flat());
  return (
    <div className="lanes">
      <div className="pane-name">
        {pane.name}: {here.length} events in frame {pane.frame}
        {pane.events ? `, from the ${pane.events.from}` : ""}
      </div>
      <svg viewBox={`0 0 ${WIDTH + 110} ${KINDS.length * LANE}`} className="lane-svg" width={WIDTH + 110} height={KINDS.length * LANE} data-testid={`lanes-${pane.id.slice(0, 8)}`}>
        {KINDS.map((k, row) => (
          <g key={k} transform={`translate(0,${row * LANE})`}>
            <text x="0" y={LANE - 4} className="lane-label">{k}</text>
            <line x1="110" x2={WIDTH + 110} y1={LANE / 2} y2={LANE / 2} className="lane-line" />
            {here
              .filter((e) => e.kind === k)
              .map((e, i) => (
                <rect key={i} x={110 + framePosition(e) * WIDTH} y="2" width="2" height={LANE - 4} className={`ev ev-${k}`}>
                  <title>{`${e.scanline}:${e.dot} ${describe(e)}`}</title>
                </rect>
              ))}
          </g>
        ))}
      </svg>
      <svg viewBox={`0 0 ${WIDTH + 110} ${KINDS.length * LANE}`} className="lane-svg" width={WIDTH + 110} height={KINDS.length * LANE}>
        {KINDS.map((k, row) => (
          <g key={k} transform={`translate(0,${row * LANE})`}>
            <text x="0" y={LANE - 4} className="lane-label">{k}</text>
            {counts.get(k)!.map((n, i) => (
              <rect
                key={i}
                x={110 + (i / span) * WIDTH}
                y="1"
                width={WIDTH / span - 1}
                height={LANE - 2}
                className={start + i === pane.frame ? "cell current" : "cell"}
                style={{ opacity: n ? 0.25 + (0.75 * n) / max : 0.06 }}
                onClick={() => setFrame(start + i)}
              >
                <title>{`frame ${start + i}: ${n} ${k}`}</title>
              </rect>
            ))}
          </g>
        ))}
      </svg>
      <div className="hint">
        frames {start} to {end - 1}; the top strip runs from vblank to vblank
      </div>
    </div>
  );
}

function EventLanes({ a, b, setFrame }: ViewProps) {
  return (
    <div>
      <Lanes pane={a} setFrame={setFrame} />
      {b && <Lanes pane={b} setFrame={setFrame} />}
    </div>
  );
}

registerView({ id: "event-lanes", title: "Event lanes", order: 50, wide: true, component: EventLanes });
