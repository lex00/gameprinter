// The control room's pieces (docs/show.md, "The live cards" and "How many
// at once"): the live cards, one for each job the background work runs
// now with its moving bars (in the main screen's "more" panel), and the
// stepper for how many clips it works on at once (in the header). The
// bars are controlfold.ts's; the state comes from the background poll
// (Background.tsx), at least once a second while a job runs.

import type { BgState } from "./backfold";
import { Stepper } from "./Calm";
import { type LiveCard, doneLine, jobBars, liveCards, workerChoices } from "./controlfold";

/** "workers [-] N [+]": 1 to the machine's cores less one, kept by the server for the run. */
export function WorkersControl({ state, set }: { state?: BgState; set: (n: number) => void }) {
  const n = state?.workers ?? 2;
  const choices = workerChoices(state?.max_workers ?? n);
  return (
    <Stepper
      label="workers"
      value={n}
      min={1}
      max={choices[choices.length - 1]}
      set={set}
      disabled={!state}
      testid="ctl-workers"
      title="How many clips the background work scores or fixes at the same time. More is faster and uses more of the machine."
    />
  );
}

function Card({ c, picture, open }: { c: LiveCard; picture?: string; open: () => void }) {
  if (c.state === "running") {
    const j = c.job;
    const doing = j.kind === "score" ? "scoring" : j.kind === "refine" ? `fixing numbers, round ${j.round ?? "?"}` : "making clips";
    return (
      <button className={`live-card ${j.kind}`} onClick={open} disabled={!c.clip} data-testid="live-card" data-state="running" title={j.trying ? `trying ${j.trying.param} = ${j.trying.value}` : "open this clip on the screen"}>
        {picture ? <img src={picture} alt="" width={128} height={120} /> : <span className="live-pic" />}
        <span className="live-name">{c.name}</span>
        <span className="live-doing">
          <span className="busy-dot" aria-hidden="true" />
          {doing}
        </span>
        {jobBars(j).map((b) => (
          <span key={b.id} className="live-bar-row" data-testid={`live-bar-${b.id}`}>
            <span className="live-label">{b.label}</span>
            {b.value !== undefined && (
              <span className="live-bar" role="progressbar" aria-valuemin={0} aria-valuemax={100} aria-valuenow={Math.round(b.value * 100)} aria-label={b.label}>
                <span style={{ width: `${b.value * 100}%` }} />
              </span>
            )}
            <span className="live-says">{b.says}</span>
          </span>
        ))}
      </button>
    );
  }
  const l = doneLine(c.done);
  return (
    <button className={`live-card done ${c.done.exact ? "matches" : ""}`} onClick={open} data-testid="live-card" data-state="done" title="open this clip on the screen">
      {picture ? <img src={picture} alt="" width={128} height={120} /> : <span className="live-pic" />}
      <span className="live-name">{c.name}</span>
      <span className="live-doing">{c.done.kind === "refine" ? "fixed numbers" : "scored"}</span>
      <span className="live-bar-row">
        <span className="live-label">frames together</span>
        <span className="live-bar">
          <span style={{ width: `${c.done.after / 10}%` }} />
        </span>
        <span className={`live-says ${l.change ?? ""}`} data-testid="live-done">
          {l.says}
        </span>
      </span>
    </button>
  );
}

/** The top row of the dashboard: a card for each job running now, then the ones just done. */
export function LiveCards({ state, picture, open }: { state?: BgState; picture: (clip: string) => string | undefined; open: (clip: string) => void }) {
  const cards = liveCards(state?.jobs, state?.done, state?.workers ?? 1);
  return (
    <section className="live-cards" data-testid="live-cards" aria-label="in progress now">
      {state?.holding && <p className="hint">{state.holding}</p>}
      {state?.verified && (
        <p className={state.verified.failures.length ? "error" : "hint"} data-testid="verified" title={state.verified.failures.join("\n")}>
          {state.verified.says}
          {state.verified.failures.length > 0 && `: ${state.verified.failures[0]}`}
        </p>
      )}
      {state?.check && !state.check.passed && (
        <p className="error" data-testid="check-failed">
          {state.check.says}
        </p>
      )}
      {cards.length ? (
        <div className="live-row">
          {cards.map((c) => (
            <Card key={c.key} c={c} picture={c.clip ? picture(c.clip) : undefined} open={() => c.clip && open(c.clip)} />
          ))}
        </div>
      ) : (
        <p className="hint" data-testid="live-none">
          {state?.enabled ? state.says || "Nothing running now." : "The background work is off: turn it on to score and fix clips."}
        </p>
      )}
    </section>
  );
}
