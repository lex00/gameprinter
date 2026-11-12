// The game's own moments in the game view (docs/inspector.md, "Moments"):
// a strip of ticks above the slider, one a moment, by kind; a hover reads
// it ("Jump 1, 3.6 s: the original goes up 66 px; the model 52 px") and a
// click sets the focus to its own frames. The list of the moments inside
// the focus, and "refine this" on one with the check on every other moment
// of its kind. Each piece takes props and assumes no layout, so a guided
// page can drive them. The words come from momentfold.ts and refinefold.ts.

import { useState } from "react";
import type { GameWords } from "./api";
import { type Moment, type MomentKind, type MomentsPhase, KIND_WORDS, momentLine, momentName, tick } from "./momentfold";
import { type MomentRefinePhase, type RefineState, alsoLine, worseLine } from "./refinefold";

const KINDS: MomentKind[] = ["jump", "landing", "death", "life_lost", "counter", "level"];

export interface MomentsStripProps {
  g?: GameWords;
  moments: Moment[];
  /** The original's frames the strip covers, as the slider below it does. */
  lo: number;
  hi: number;
  /** The moment picked, drawn marked. */
  picked?: Moment;
  phase: MomentsPhase;
  pick: (m: Moment) => void;
}

/** Ticks by kind over the slider's frames, a row a kind that has any. */
export function MomentsStrip({ g, moments, lo, hi, picked, phase, pick }: MomentsStripProps) {
  const [hover, setHover] = useState<Moment>();
  const shown = moments.filter((m) => m.to >= lo && m.from <= hi);
  const kinds = KINDS.filter((k) => moments.some((m) => m.kind === k));
  return (
    <div className={`moments-strip ${phase.state}`} data-testid="moments-strip">
      {phase.state !== "ready" && <p className="hint">{phase.says}</p>}
      {kinds.map((k) => (
        <div key={k} className="moments-row">
          <span className="moments-kind">{KIND_WORDS[k].name}</span>
          <span className="moments-track">
            {shown
              .filter((m) => m.kind === k)
              .map((m) => {
                const t = tick(m, lo, hi);
                const same = picked && picked.kind === m.kind && picked.n === m.n;
                return (
                  <button
                    key={`${m.kind}${m.n}${m.part}`}
                    className={`moment-tick ${m.kind}${m.checked && !m.model ? " missed" : ""}${same ? " picked" : ""}`}
                    style={{ left: `${100 * t.left}%`, width: `max(3px, ${100 * t.width}%)` }}
                    onMouseEnter={() => setHover(m)}
                    onMouseLeave={() => setHover(undefined)}
                    onClick={() => pick(m)}
                    aria-label={momentLine(g, m)}
                    data-testid="moment-tick"
                  />
                );
              })}
          </span>
        </div>
      ))}
      {hover && (
        <p className="moments-tip hint" data-testid="moment-tip">
          {momentLine(g, hover)}
        </p>
      )}
    </div>
  );
}

/** The moments inside the focus, each a line that focuses on it. */
export function MomentsInFocus({ g, moments, pick }: { g?: GameWords; moments: Moment[]; pick: (m: Moment) => void }) {
  if (!moments.length) return null;
  return (
    <div className="focus-moments" data-testid="focus-moments">
      <h3>In these frames</h3>
      <ul>
        {moments.map((m) => (
          <li key={`${m.kind}${m.n}${m.part}`}>
            {momentLine(g, m)} <button onClick={() => pick(m)}>focus on it</button>
          </li>
        ))}
      </ul>
    </div>
  );
}

export interface RefineThisMomentProps {
  moment: Moment;
  phase: MomentRefinePhase;
  state?: RefineState;
  start: (m: Moment) => void;
  stop: () => void;
  /** Both games to a frame of the original. */
  seek: (frame: number) => void;
}

/** "Refine this jump" on a focused moment, what it is doing, and the check on the others of its kind. */
export function RefineThisMoment({ moment, phase, state, start, stop, seek }: RefineThisMomentProps) {
  const also = state?.result?.also;
  const running = phase.state === "refining";
  const kind = KIND_WORDS[moment.kind].name.toLowerCase();
  return (
    <div className="refine-moment" data-testid="refine-moment">
      <p>
        {phase.says} <span className="hint">{phase.next}</span>
      </p>
      {running ? (
        <button onClick={stop}>stop</button>
      ) : (
        <button onClick={() => start(moment)} disabled={!moment.mechanic} data-testid="refine-this" title={moment.mechanic ? `Only ${moment.mechanic}'s numbers, in these frames` : "No mechanic owns this kind of moment"}>
          {`refine this ${moment.kind === "counter" ? "change" : kind}`}
        </button>
      )}
      {also && phase.state === "done" && (
        <div className="refine-also" data-testid="refine-also">
          <p>{alsoLine(also)}</p>
          {also.worse_moments.length > 0 && (
            <ul>
              {also.worse_moments.map((w) => (
                <li key={w.n}>
                  {worseLine(also.kind, w)} <button onClick={() => seek(w.from)}>go there</button>
                </li>
              ))}
            </ul>
          )}
        </div>
      )}
    </div>
  );
}

export { momentName };
