// Pause, point and tell (docs/point-and-tell.md). A layer over each game
// screen takes a click or a drag while the games are paused; the Tell
// panel says what's there in plain words and sends what the person wants
// as a request; the request list shows each one's answer under it, with a
// proposed change's pictures and accept and reject.

import { useEffect, useRef, useState, type ReactNode } from "react";
import type { Api, Box } from "./api";
import { ASK_WORDS, CHOICES, H, W, answersAfter, boxFrom, questionInput, requestInput, screenPoint, where, type PointJson, type Side, type TalkTurn } from "./point";

/** A game screen with the pointing layer over it: active while paused. */
export function ScreenPointer({ active, box, onBox, children, testid }: { active: boolean; box?: Box; onBox: (b: Box) => void; children: ReactNode; testid?: string }) {
  const start = useRef<{ x: number; y: number } | undefined>(undefined);
  const [drag, setDrag] = useState<Box>();
  const shown = drag ?? box;
  const at = (e: { clientX: number; clientY: number; currentTarget: Element }) => screenPoint(e.clientX, e.clientY, e.currentTarget.getBoundingClientRect());
  return (
    <div className={active ? "screen-wrap pointing" : "screen-wrap"}>
      {children}
      {active && (
        <svg
          className="point-layer"
          data-testid={testid}
          viewBox={`0 0 ${W} ${H}`}
          preserveAspectRatio="none"
          onPointerDown={(e) => {
            start.current = at(e);
            (e.currentTarget as Element).setPointerCapture?.(e.pointerId);
          }}
          onPointerMove={(e) => {
            if (start.current) setDrag(boxFrom(start.current, at(e)));
          }}
          onPointerUp={(e) => {
            const a = start.current;
            start.current = undefined;
            setDrag(undefined);
            if (a) onBox(boxFrom(a, at(e)));
          }}
        >
          <title>Click a spot or drag a box around what needs fixing</title>
          {shown && <rect className="point-box" x={shown.x} y={shown.y} width={shown.w} height={shown.h} />}
        </svg>
      )}
    </div>
  );
}

/** What the box covers, and the words and choices to send about it. */
export function TellPanel({
  api,
  run,
  recording,
  frame,
  box,
  side = "original",
}: {
  api: Api;
  run: string;
  recording?: string;
  frame: number;
  box?: Box;
  /** Which game's picture the box is on. */
  side?: Side;
}) {
  const [point, setPoint] = useState<PointJson>();
  const [error, setError] = useState<string>();
  const [choice, setChoice] = useState<string>();
  const [text, setText] = useState("");
  const [sending, setSending] = useState(false);
  const [sent, setSent] = useState<string>();
  // "What is this?": the question's turn in the talk, and the agent's
  // answer to it, read from the talk until it comes.
  const [asked, setAsked] = useState<number>();
  const [answers, setAnswers] = useState<TalkTurn[]>([]);
  useEffect(() => {
    if (asked === undefined) return;
    let live = true;
    const read = () =>
      api
        .mechanicTalk(run)
        .then((s) => live && setAnswers(answersAfter(s.talk, asked)))
        .catch(() => {});
    void read();
    const t = setInterval(read, 2000);
    return () => {
      live = false;
      clearInterval(t);
    };
  }, [api, run, asked]);

  useEffect(() => {
    setPoint(undefined);
    setError(undefined);
    if (!box || !recording) return;
    const ab = new AbortController();
    api
      .capturePoint(run, recording, frame, box, ab.signal)
      .then(setPoint)
      .catch((e: Error) => {
        if (e.name !== "AbortError") setError(e.message);
      });
    return () => ab.abort();
  }, [api, run, recording, frame, box?.x, box?.y, box?.w, box?.h]); // eslint-disable-line react-hooks/exhaustive-deps

  if (!box) {
    return (
      <p className="hint tell-hint" data-testid="tell-hint">
        Paused. Click a spot or drag a box on either game around what needs fixing.
      </p>
    );
  }
  const ask = async () => {
    if (!box || !recording) return;
    setSending(true);
    setError(undefined);
    setAnswers([]);
    try {
      const r = await api.captureInput(run, questionInput(recording, frame, box, side, point));
      setAsked(r.turn);
      setSent(r.mechanic ? `Asked your agent what this is, in the talk on ${r.mechanic}. The answer shows here and there.` : "Asked your agent what this is.");
    } catch (e) {
      setError((e as Error).message);
    } finally {
      setSending(false);
    }
  };
  const send = async () => {
    if (!point) return;
    setSending(true);
    setError(undefined);
    try {
      const r = await api.captureInput(run, requestInput(point, choice, text));
      setSent(r.mechanic ? `Sent to your agent, in the talk on ${r.mechanic}. The answer shows there.` : "Sent to your agent.");
      setText("");
      setChoice(undefined);
    } catch (e) {
      setError((e as Error).message);
    } finally {
      setSending(false);
    }
  };
  return (
    <section className="tell" data-testid="tell">
      <p className="pointed" data-testid="tell-pointed">
        {point ? (
          <>
            You pointed at <b>{point.label}</b>
            {point.things.length > 1 && (
              <span className="hint">
                {" "}
                (also {point.things
                  .slice(1)
                  .map((t) => t.label)
                  .join(", ")}
                )
              </span>
            )}
            {where(point.things[0] ?? { what: "cell", label: "" }) && <span className="hint"> {where(point.things[0])}</span>}.
          </>
        ) : error ? (
          <span className="error">{error}</span>
        ) : (
          "Looking at what's there..."
        )}
      </p>
      <div className="choices" role="group" aria-label="quick choices">
        {CHOICES.map((c) => (
          <button key={c.key} type="button" className={choice === c.key ? "choice on" : "choice"} aria-pressed={choice === c.key} onClick={() => setChoice(choice === c.key ? undefined : c.key)} data-testid="tell-choice">
            {c.words}
          </button>
        ))}
      </div>
      <textarea value={text} onChange={(e) => setText(e.target.value)} placeholder="what should happen here?" rows={2} data-testid="tell-text" />
      <p>
        <button type="button" disabled={!point || sending || (!choice && !text.trim())} onClick={() => void send()} data-testid="tell-send">
          tell the agent
        </button>{" "}
        <button type="button" disabled={!recording || sending} onClick={() => void ask()} data-testid="tell-ask" title="Ask your agent what this is and what drives it; it answers from the game's code">
          {ASK_WORDS}
        </button>{" "}
        {sent && <span className="hint">{sent}</span>}
      </p>
      {asked !== undefined && (
        <div className="tell-answer" data-testid="tell-answer" aria-live="polite">
          {answers.length ? answers.map((a) => <p key={a.n} className="answer">{a.text}</p>) : <p className="hint">Waiting for your agent's answer...</p>}
        </div>
      )}
    </section>
  );
}
