// Pause, point and tell (docs/point-and-tell.md). A layer over each game
// screen takes a click or a drag while the games are paused; the Tell
// panel says what's there in plain words and sends what the person wants
// as a request; the request list shows each one's answer under it, with a
// proposed change's pictures and accept and reject.

import { useEffect, useRef, useState, type ReactNode } from "react";
import type { Api, Box, CaptureInput } from "./api";
import { CHOICES, H, W, boxFrom, choiceWords, requestInput, screenPoint, statusWords, where, type PointJson, type RequestView } from "./point";

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
  onSent,
  after,
}: {
  api: Api;
  run: string;
  recording?: string;
  frame: number;
  box?: Box;
  /** Called with the request's number once it's sent. */
  onSent?: (id: number) => void;
  /** What happens after sending, said under the button. */
  after?: string;
}) {
  const [point, setPoint] = useState<PointJson>();
  const [error, setError] = useState<string>();
  const [choice, setChoice] = useState<string>();
  const [text, setText] = useState("");
  const [sending, setSending] = useState(false);
  const [sent, setSent] = useState<string>();

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
  const send = async () => {
    if (!point) return;
    setSending(true);
    setError(undefined);
    try {
      const r = await api.captureInput(run, requestInput(point, choice, text));
      const id = Number(String(r.input?.item ?? "").replace("request:", ""));
      setSent(`Sent as request ${id}.`);
      setText("");
      setChoice(undefined);
      if (id) onSent?.(id);
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
        {sent && <span className="hint">{sent}</span>} {after && <span className="hint">{after}</span>}
      </p>
    </section>
  );
}

/** The person's requests, newest first, each with its answers. */
export function RequestList({
  api,
  run,
  requests,
  explaining,
  watch,
  send,
}: {
  api: Api;
  run: string;
  requests: Record<number, RequestView>;
  explaining: number[];
  watch: (recording: string, frame: number) => void;
  send: (i: Omit<CaptureInput, "v" | "actor">) => void;
}) {
  const all = Object.values(requests).sort((a, b) => b.id - a.id);
  if (!all.length) return null;
  return (
    <section className="requests" data-testid="requests">
      <h2>{all.length === 1 ? "What you asked for" : `What you asked for (${all.length})`}</h2>
      <ul>
        {all.map((r) => {
          const last = r.replies[r.replies.length - 1];
          const busy = explaining.includes(r.id);
          return (
            <li key={r.id} data-testid="request">
              <p>
                <b>{r.label}</b> at frame {r.frame}
                {choiceWords(r.choice) ? `: ${choiceWords(r.choice)}` : ""}
                {r.text ? ` "${r.text}"` : ""}{" "}
                <span className={`status ${r.status}`} data-testid="request-status">
                  {last?.agent === "failed" && r.status === "waiting" ? "no agent is set up to answer" : statusWords(r.status, busy && !last, last?.agent === "working")}
                </span>{" "}
                <button className="link" onClick={() => watch(r.recording, r.frame)}>
                  show me
                </button>
              </p>
              {r.replies.map((x) => (
                <div key={x.reply} className="reply" data-testid="request-reply">
                  {(x.lines.length ? x.lines.slice(1) : [x.text]).map((l, i) => (
                    <p key={i}>{l}</p>
                  ))}
                  {x.proposal && <Proposal api={api} run={run} item={x.proposal.item} kind={x.proposal.kind} decided={r.status === "accepted" || r.status === "rejected"} send={send} />}
                </div>
              ))}
            </li>
          );
        })}
      </ul>
    </section>
  );
}

/** A proposed change: the edit's pictures in both games' moments, and the person's decision. */
function Proposal({ api, run, item, kind, decided, send }: { api: Api; run: string; item: string; kind: string; decided: boolean; send: (i: Omit<CaptureInput, "v" | "actor">) => void }) {
  const [copy, seq] = item.split(":");
  return (
    <div className="proposal" data-testid="proposal">
      {kind === "asset_edit" && copy && seq && (
        <>
          <figure>
            <img src={api.captureEditPreview(run, copy, seq, "edit")} alt="the picture, before and after" data-testid="proposal-closeup" />
            <figcaption>the picture, before and after</figcaption>
          </figure>
          <figure>
            <img
              src={api.captureEditPreview(run, copy, seq, "context")}
              alt="the proposed look in the recorded moments, before and after"
              onError={(e) => {
                // An edit seen on no recorded frame has no such picture.
                (e.currentTarget.parentElement as HTMLElement).hidden = true;
              }}
            />
            <figcaption>the proposed look where the game shows it, before and after</figcaption>
          </figure>
        </>
      )}
      {kind === "sound_edit" && <p className="hint">A sound change: listen to it in the run's sound edits.</p>}
      {!decided && (
        <p>
          <button onClick={() => send({ kind: "accept", item })} data-testid="proposal-accept">
            keep it
          </button>{" "}
          <button onClick={() => send({ kind: "reject", item })}>no, undo it</button>
        </p>
      )}
    </div>
  );
}
