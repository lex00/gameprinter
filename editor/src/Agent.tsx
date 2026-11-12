// Claude's work on the clip card and the progress panel (docs/show.md,
// "Claude working on a clip"): who is working on the clip and on what,
// each attempt with its score change, watch this try (the attempt's
// rebuilt game on the main screen and its curve in the chart), keep, and
// the clip agent's start and stop. The record is polled like the
// background line.

import { useCallback, useEffect, useRef, useState } from "react";
import type { Api } from "./api";
import { type TellBody, type WorkAttempt, type WorkState, WorkClient, agentLine, attemptBusy, attemptLine, attemptNote, byWords, clipAttempts, progressLines, workingOn } from "./agentfold";

/** The run's work record, polled while the page is open. */
export function useWork(api: Api, run: string) {
  const client = useRef(new WorkClient(api));
  const [state, setState] = useState<WorkState>();
  const [error, setError] = useState<string>();
  const refresh = useCallback(async () => {
    try {
      setState(await client.current.state(run));
    } catch {
      // The server may be gone a moment; the next poll tries again.
    }
  }, [run]);
  useEffect(() => {
    void refresh();
    const t = setInterval(() => void refresh(), 2000);
    return () => clearInterval(t);
  }, [refresh]);
  const act = async (f: () => Promise<WorkState>) => {
    try {
      setError(undefined);
      const s = await f();
      setState(s);
    } catch (e) {
      setError((e as Error).message);
    }
  };
  return {
    state,
    error,
    client: client.current,
    approve: (n: number) => act(() => client.current.approve(run, n)),
    start: (clip: string) =>
      act(async () => {
        const r = (await client.current.start(run, clip)) as WorkState & { started?: boolean; why?: string | null };
        if (r.started === false && r.why) setError(r.why);
        return r;
      }),
    stop: () => act(() => client.current.stop(run)),
    /** Tells the agent about a clip; throws when the server refuses, so the box can say why. */
    tell: async (clip: string, body: TellBody) => {
      const r = await client.current.tell(run, clip, body);
      setState((s) => (s ? { ...s, tells: r.tells } : s));
      return r;
    },
  };
}

export interface AgentSectionProps {
  state?: WorkState;
  clip: string;
  /** The attempt whose trial preview is on the screen, if one is. */
  watching?: number;
  watch: (n: number | undefined) => void;
  keep: (n: number) => void;
  /** Start the clip agent here; none when no agent can be started. */
  start?: () => void;
  stop: () => void;
  error?: string;
}

/** The clip card's Agent section: what Claude is doing on this clip and each attempt. */
export function AgentSection({ state, clip, watching, watch, keep, start, stop, error }: AgentSectionProps) {
  const who = workingOn(state, clip);
  const agent = agentLine(state?.clip_agent, clip);
  const atts = clipAttempts(state, clip);
  const otherClip = state?.clip_agent && state.clip_agent.clip !== clip && agentLine(state.clip_agent, state.clip_agent.clip)?.busy;
  if (!who.length && !agent && !atts.length && !start) return null;
  return (
    <div className="clip-agent" data-testid="clip-agent">
      <p className="clip-agent-head">Claude</p>
      {agent && (
        <p className="clip-doing" data-testid="agent-status">
          {agent.busy && <span className="busy-dot" aria-hidden="true" />}
          {agent.says} <small className="hint">the clip agent</small>
        </p>
      )}
      {who
        .filter((w) => w.by !== "clip agent" || !agent)
        .map((w) => (
          <p key={w.by} className="clip-doing" data-testid="agent-working">
            <span className="busy-dot" aria-hidden="true" />
            {w.says} <small className="hint">{byWords(w.by)}</small>
          </p>
        ))}
      {atts.length > 0 && (
        <ol className="agent-attempts" data-testid="agent-attempts">
          {atts.map((a) => (
            <AttemptRow key={a.n} a={a} watching={watching === a.n} watch={watch} keep={keep} />
          ))}
        </ol>
      )}
      <div className="clip-actions">
        {agent?.busy ? (
          <button onClick={stop} data-testid="agent-stop">
            stop
          </button>
        ) : (
          start && (
            <button onClick={start} disabled={!!otherClip} data-testid="agent-start" title={otherClip ? "The clip agent is working on another clip of this run" : "Start Claude on this clip: it finds the cause in the game's code, changes how the mechanic works in its own copy of the repo, and shows each attempt here"}>
              let the agent fix it
            </button>
          )
        )}
      </div>
      {error && <p className="error">{error}</p>}
    </div>
  );
}

function AttemptRow({ a, watching, watch, keep }: { a: WorkAttempt; watching: boolean; watch: (n: number | undefined) => void; keep: (n: number) => void }) {
  const note = attemptNote(a);
  return (
    <li data-testid="agent-attempt">
      {attemptBusy(a) && <span className="busy-dot" aria-hidden="true" />}
      {attemptLine(a)} <small className="hint">{byWords(a.by)}</small>
      {a.says && <span className="hint"> {a.says}</span>}
      {note && (
        <span className={a.approved ? "agent-approved" : "hint"} data-testid="agent-note">
          {" "}
          {note}
        </span>
      )}
      {a.preview && (
        <button onClick={() => watch(watching ? undefined : a.n)} className={watching ? "on" : undefined} data-testid="agent-watch" title="Play this attempt's rebuilt game on the main screen, looping the clip on top of the original, and show its curve">
          {watching ? "back to the game as it is" : "watch this try"}
        </button>
      )}
      {a.keepable && !a.approved && (
        <button className="primary" onClick={() => keep(a.n)} data-testid="agent-keep" title="Keep this change: it is merged into the game by the driver, who restarts the server">
          keep this change
        </button>
      )}
    </li>
  );
}

/** The progress panel's row: who is working on what. */
export function AgentProgress({ state }: { state?: WorkState }) {
  const lines = progressLines(state);
  if (!lines.length) return null;
  return (
    <>
      {lines.map((l) => (
        <div key={l.by} className="progress-row" data-testid="progress-agent">
          <span className="progress-label">Claude</span>
          <span>
            <span className="busy-dot" aria-hidden="true" /> <small className="hint">{l.by}</small>
          </span>
          <span className="one-line" title={l.says}>
            {l.says}
          </span>
        </div>
      ))}
    </>
  );
}
