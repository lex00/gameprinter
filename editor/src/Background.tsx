// The status line under the main screen (docs/show.md, "The background
// work"): one sentence for the background queue, pause and resume, the
// questions badge, and its rounds in words when asked for.

import { useCallback, useEffect, useRef, useState } from "react";
import type { Api, GameWords } from "./api";
import { BackgroundClient, type BgQuestion, type BgRound, type BgState, openQuestions, roundWords, statusSentence } from "./backfold";
import { RoundCurve } from "./CurveChart";
import { questionNeedsYou } from "./calmfold";

/** The queue's state and rounds, polled while the page is open. */
export function useBackground(api: Api, run: string) {
  const client = useRef(new BackgroundClient(api));
  const [state, setState] = useState<BgState>();
  const [history, setHistory] = useState<BgRound[]>([]);
  const rounds = useRef(-1);
  const refresh = useCallback(async () => {
    try {
      const s = await client.current.state(run);
      setState(s);
      const n = (s as BgState & { rounds?: number }).rounds ?? 0;
      if (n !== rounds.current) {
        rounds.current = n;
        setHistory((await client.current.history(run)).rounds);
      }
    } catch {
      // The server may be gone a moment; the next poll tries again.
    }
  }, [run]);
  // Every second while a job runs, so the live cards' bars move; every
  // three seconds otherwise.
  const busy = !!state?.working;
  useEffect(() => {
    void refresh();
    const t = setInterval(() => void refresh(), busy ? 1000 : 3000);
    return () => clearInterval(t);
  }, [refresh, busy]);
  const toggle = async () => {
    const s = state?.enabled ? await client.current.stop(run) : await client.current.start(run);
    setState(s);
  };
  /** "Fix this clip": the queue on, that clip first. */
  const fix = async (clip: string) => {
    setState(await client.current.start(run, clip));
    void refresh();
  };
  const answer = async (q: string, choice: string) => {
    setState(await client.current.answer(run, q, choice));
    void refresh();
  };
  /** How many clips it works on at once. */
  const setWorkers = async (n: number) => {
    await client.current.setWorkers(run, n);
    void refresh();
  };
  /** Replays the run's ledger and checks every result reproduces. */
  const verify = async () => {
    setState(await client.current.verify(run));
  };
  return { state, history, toggle, answer, refresh, fix, setWorkers, verify };
}

export function BackgroundLine({ g, state, history, toggle, openQuestion }: { g?: GameWords; state?: BgState; history: BgRound[]; toggle: () => void; openQuestion: (q: BgQuestion) => void }) {
  const [showHistory, setShowHistory] = useState(false);
  const qs = openQuestions(state).filter(questionNeedsYou);
  return (
    <>
      <span data-testid="bg-says">{statusSentence(state, history, Date.now(), g)}</span>
      <button onClick={toggle} disabled={!state} data-testid="bg-toggle">
        {state?.enabled ? "pause" : history.length ? "resume" : "turn on"}
      </button>
      {qs.length > 0 && (
        <button className="badge" onClick={() => openQuestion(qs[0])} title={qs[0].ask} data-testid="bg-questions">
          {qs.length === 1 ? "1 question" : `${qs.length} questions`}
        </button>
      )}
      {history.length > 0 && (
        <button className="link" onClick={() => setShowHistory(!showHistory)} data-testid="bg-history-toggle">
          {showHistory ? "hide what it did" : "what it did"}
        </button>
      )}
      {showHistory && (
        <ol className="bg-history" data-testid="bg-history">
          {[...history]
            .reverse()
            .slice(0, 20)
            .map((r, i) => (
              <li key={i}>
                {roundWords(g, r)}
                <RoundCurve curve={r.curve} />
              </li>
            ))}
        </ol>
      )}
    </>
  );
}
