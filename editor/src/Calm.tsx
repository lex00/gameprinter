// The calm main screen's pieces (docs/show.md, "The main screen"): the
// compact header (the fidelity gauge, the workers and agents steppers,
// start and pause), the thumbnails of the clips in progress with their
// badges, and a clip's detail drawer. The rules are calmfold.ts's. The
// user asked for it on 2026-10-30: "the data beneath the render area ...
// needs to be rendered more like a real GUI not just text". Every item
// shows at most one line; the rest is in a tooltip or a "details"
// disclosure.

import { type ReactNode, useState } from "react";
import type { GameWords } from "./api";
import { type FixReady, type WorkAttempt, type WorkEntry, type WorkTell, byWords } from "./agentfold";
import { type BgJob, type BgQuestion, type BgRound, changeWords, roundWords } from "./backfold";
import { type Badge, type Status, attemptRow, exactWords } from "./calmfold";
import { jobBars } from "./controlfold";
import type { FidelityState } from "./Fidelity";
import { secs } from "./freefold";

/** A bar from 0 to 1000, in the clip's match colour. */
export function MatchBar({ permille, className = "" }: { permille?: number; className?: string }) {
  const v = permille ?? 0;
  const tone = permille === undefined ? "none" : v >= 1000 ? "match" : v >= 900 ? "close" : "differ";
  return (
    <span className={`mbar ${tone} ${className}`} role="meter" aria-valuemin={0} aria-valuemax={100} aria-valuenow={Math.round(v / 10)} aria-label="how much matches" title={permille === undefined ? "not scored yet" : `${Math.round(v / 10)}% fidelity to the original`}>
      <span style={{ width: `${v / 10}%` }} />
    </span>
  );
}

/** The fidelity score as one gauge, its three parts as small bars, and the clips exact. */
export function FidelityGauge({ f }: { f?: FidelityState }) {
  if (!f) return <div className="fid fid-empty" data-testid="fidelity-gauge" />;
  const r = 17;
  const c = 2 * Math.PI * r;
  const parts: [string, number | undefined][] = [
    ["behaviour", f.behaviour.score],
    ["coverage", f.coverage.score],
    ["picture", f.picture?.score],
  ];
  const tip = ["Fidelity: how close the rebuild plays to the original, measured clip by clip against the ROM (behaviour 60, coverage 25, picture 15).", f.says, f.confidence.says, ...f.holding.map((h) => `Holding it down: ${h.says}`)].join("\n");
  return (
    <div className="fid" data-testid="fidelity-gauge" title={tip}>
      <svg className="fid-ring" width="44" height="44" viewBox="0 0 44 44" role="img" aria-label={`fidelity ${f.score} of 100`}>
        <circle cx="22" cy="22" r={r} className="fid-track" />
        <circle cx="22" cy="22" r={r} className="fid-arc" strokeDasharray={`${(c * Math.max(0, Math.min(100, f.score))) / 100} ${c}`} transform="rotate(-90 22 22)" />
        <text x="22" y="26" textAnchor="middle" data-testid="fidelity-score">
          {f.score}
        </text>
      </svg>
      <div className="fid-parts">
        {parts.map(([k, v]) => (
          <div key={k} className="fid-part" data-testid={`fidelity-${k}`}>
            <span className="fid-label">{k}</span>
            <span className="fid-bar">
              <span style={{ width: `${v ?? 0}%` }} />
            </span>
            <span className="fid-num">{v ?? "-"}</span>
          </div>
        ))}
      </div>
      <div className="fid-exact" title="clips that match the original exactly, of the clips scored">
        <strong data-testid="fidelity-exact">{exactWords(f)}</strong>
        <span>exact</span>
      </div>
    </div>
  );
}

/** A small stepper: label, minus, the value, plus. */
export function Stepper({ label, value, min, max, set, testid, title, disabled }: { label: string; value: number; min: number; max: number; set: (n: number) => void; testid: string; title?: string; disabled?: boolean }) {
  return (
    <span className="stepper" title={title}>
      <span className="stepper-label">{label}</span>
      <button aria-label={`fewer ${label}`} disabled={disabled || value <= min} onClick={() => set(value - 1)} data-testid={`${testid}-less`}>
        -
      </button>
      <output data-testid={testid}>{value}</output>
      <button aria-label={`more ${label}`} disabled={disabled || value >= max} onClick={() => set(value + 1)} data-testid={`${testid}-more`}>
        +
      </button>
    </span>
  );
}

const ICON: Record<Badge["kind"], string> = { "fix-ready": "", resolved: "", "needs-you": "?", tell: "", agent: "", fixing: "", scoring: "", better: "^", stuck: "-" };

function Wrench() {
  return (
    <svg className="bdg-svg" width="11" height="11" viewBox="0 0 16 16" aria-hidden="true">
      <path fill="currentColor" d="M14.7 4.3l-2.4 2.4-2-2 2.4-2.4a4 4 0 0 0-5 5.2L1.2 12.2a1.4 1.4 0 0 0 2 2l5.7-6.5a4 4 0 0 0 5.8-3.4z" />
    </svg>
  );
}

function Star() {
  return (
    <svg className="bdg-svg" width="11" height="11" viewBox="0 0 16 16" aria-hidden="true">
      <path fill="currentColor" d="M8 .8l2.2 4.6 5 .7-3.6 3.5.9 5L8 12.2 3.5 14.6l.9-5L.8 6.1l5-.7z" />
    </svg>
  );
}

export function BadgeChip({ b }: { b: Badge }) {
  const spin = b.kind === "agent" || b.kind === "fixing" || b.kind === "scoring";
  return (
    <span className={`bdg ${b.kind}`} data-testid={`badge-${b.kind}`}>
      {spin ? <span className="spin" aria-hidden="true" /> : b.kind === "fix-ready" ? <Wrench /> : b.kind === "resolved" ? <Star /> : b.kind === "tell" ? <span className="bubble" aria-hidden="true" /> : <span className="bdg-icon" aria-hidden="true">{ICON[b.kind]}</span>}
      {b.label}
    </span>
  );
}

export interface Thumb {
  id: string;
  name: string;
  picture: string;
  permille?: number;
  badges: Badge[];
}

/** The clips in progress as thumbnails: the picture, a small match bar and at most two badges. */
export function Thumbs({ thumbs, open, selected, more }: { thumbs: Thumb[]; open: (id: string) => void; selected?: string; more?: number }) {
  const needs = thumbs.filter((t) => t.badges.some((b) => b.kind === "needs-you")).length;
  const busy = thumbs.filter((t) => t.badges.some((b) => b.kind === "agent" || b.kind === "fixing" || b.kind === "scoring")).length;
  return (
    <section className="thumbs" data-testid="thumbs" aria-label="clips in progress">
      <div className="thumbs-head">
        <span className="thumbs-title">In progress</span>
        {needs > 0 && (
          <span className="chip needs-you" data-testid="needs-you-count">
            {needs} need you
          </span>
        )}
        {busy > 0 && <span className="chip working">{busy} working</span>}
        {!!more && <span className="hint">+{more} more</span>}
      </div>
      {thumbs.length ? (
        <div className="thumb-grid">
          {thumbs.map((t) => (
            <button key={t.id} className={`thumb${t.badges.some((b) => b.kind === "needs-you") ? " needs-you" : ""}${selected === t.id ? " on" : ""}`} onClick={() => open(t.id)} title={t.name} data-testid="thumb" data-clip={t.id}>
              <span className="thumb-pic">
                {t.picture ? <img src={t.picture} alt="" width={128} height={120} loading="lazy" /> : <span className="thumb-none" />}
                <span className="thumb-badges">
                  {t.badges.map((b) => (
                    <BadgeChip key={b.kind} b={b} />
                  ))}
                </span>
              </span>
              <MatchBar permille={t.permille} />
              <span className="thumb-name">{t.name}</span>
            </button>
          ))}
        </div>
      ) : (
        <p className="hint" data-testid="thumbs-none">
          Nothing in progress.
        </p>
      )}
    </section>
  );
}

const STATUS_CLASS: Record<Status, string> = { "fix ready": "fixready", "needs you": "needs-you", "being fixed": "working", matches: "match", close: "close", differs: "differ", waiting: "none", "can't start": "none" };

/** The frame timeline: the clip's frames, the first wrong frame marked, and where it plays now. */
export function PartsTimeline({ frames, at, wrong }: { frames: number; at: number; wrong?: number }) {
  const len = Math.max(1, frames);
  const pct = (f: number) => `${Math.min(100, Math.max(0, (f / len) * 100))}%`;
  return (
    <div className="timeline" data-testid="timeline">
      <span className="tl-ok" style={{ width: wrong !== undefined ? pct(wrong) : "100%" }} />
      {wrong !== undefined && <span className="tl-bad" style={{ left: pct(wrong), width: `calc(100% - ${pct(wrong)})` }} />}
      {wrong !== undefined && <span className="tl-mark" style={{ left: pct(wrong) }} data-testid="clip-drift" title={`first wrong frame: ${wrong}`} />}
      <span className="tl-now" style={{ left: pct(at) }} />
    </div>
  );
}

/** One attempt as a row: title, before -> after bar, better and worse counts, and its buttons. */
function AttemptItem({ a, watching, watch, keep }: { a: WorkAttempt; watching: boolean; watch: (n: number | undefined) => void; keep: (n: number) => void }) {
  const r = attemptRow(a);
  return (
    <li className="try" data-testid="agent-attempt" title={a.line ?? a.says ?? a.title}>
      <span className="try-title">
        {r.busy && <span className="spin" aria-hidden="true" />}
        {r.title}
      </span>
      <span className="try-bars">
        <MatchBar permille={r.before} className="before" />
        <span className="arrow" aria-hidden="true">
          {"->"}
        </span>
        <MatchBar permille={r.after} />
      </span>
      <span className="try-counts">
        <span className="good" title="other clips better">
          +{r.better}
        </span>
        <span className="bad" title="other clips worse">
          -{r.worse}
        </span>
      </span>
      <span className="try-acts">
        {a.approved && <span className="chip match">kept</span>}
        {a.preview && (
          <button onClick={() => watch(watching ? undefined : a.n)} className={watching ? "on" : undefined} data-testid="agent-watch" title="Play this attempt's rebuilt game on the screen">
            {watching ? "as it is" : "watch"}
          </button>
        )}
        {a.keepable && !a.approved && (
          <button className="primary" onClick={() => keep(a.n)} data-testid="agent-keep" title="Keep this change: the driver merges it into the game">
            keep
          </button>
        )}
      </span>
    </li>
  );
}

/** A refine round as a row: what it changed, clips fixed in green and worse in red. */
function RoundItem({ g, r }: { g?: GameWords; r: BgRound }) {
  const title = r.changes?.length ? r.changes.map((c) => changeWords(g, c)).join(", ") : r.stuck ? "no numbers make it match" : r.says;
  return (
    <li className="try" title={roundWords(g, r)} data-testid="round-row">
      <span className="try-title">
        Round {r.round}: {title}
      </span>
      <span className="try-counts">
        <span className="good" title="clips fixed">
          +{r.fixed ?? 0}
        </span>
        <span className="bad" title="clips worse">
          -{r.worse?.length ?? 0}
        </span>
      </span>
    </li>
  );
}

/** The job running on the clip now, as a row with its bars. */
function JobItem({ j }: { j: BgJob }) {
  const bars = jobBars(j).filter((b) => b.value !== undefined);
  const title = j.kind === "refine" ? `Round ${j.round ?? "?"}${j.trying ? `: trying ${j.trying.param.split(".").slice(-2).join(" ")} = ${j.trying.value}` : ""}` : "Scoring";
  return (
    <li className="try working" data-testid="job-row" title={j.trying?.words ?? title}>
      <span className="try-title">
        <span className="spin" aria-hidden="true" />
        {title}
      </span>
      <span className="try-minibars">
        {bars.slice(0, 3).map((b) => (
          <span key={b.id} className="minibar" title={`${b.label}: ${b.says}`}>
            <span style={{ width: `${(b.value ?? 0) * 100}%` }} />
          </span>
        ))}
      </span>
    </li>
  );
}

export interface ClipDetailProps {
  g?: GameWords;
  name: string;
  /** One line: "Played by you, 3.1 s" or "Made by the tool, 3.0 s". */
  sub: string;
  status: Status;
  permille?: number;
  frames: number;
  at: number;
  parts?: { frame: number; what: string };
  job?: BgJob;
  /** The clip agent on it, in one line. */
  agent?: { says: string; busy: boolean };
  attempts: WorkAttempt[];
  rounds: BgRound[];
  watching?: number;
  watch: (n: number | undefined) => void;
  keep: (n: number) => void;
  startAgent?: () => void;
  stopAgent?: () => void;
  /** The questions that need the person, the loop's other questions, and the agents' asks. */
  needsYou: BgQuestion[];
  other: BgQuestion[];
  asks: WorkEntry[];
  /** What the person told the agent about this clip, with its replies. */
  tells?: WorkTell[];
  /** An approved fix for the clip that waits to be merged. */
  fixReady?: FixReady;
  /** The spot or region the person clicked on the screen, in words ("the spot (120, 88) at frame 40"). */
  pointed?: string;
  clearPoint?: () => void;
  /** The clip is paused on the screen. */
  paused?: boolean;
  setPaused?: (p: boolean) => void;
  /** Sends a note; the page adds the frame and the point. */
  tell?: (text: string) => Promise<void>;
  answer: (question: string, choice: string) => void;
  /** The background work fixes this clip first; none when it can't. */
  fix?: () => void;
  fixing?: boolean;
  mark: string;
  setMark: (m: "kept" | "skipped" | "starred") => void;
  playOn?: () => void;
  close: () => void;
  /** Everything else, behind the "details" disclosure. */
  more?: ReactNode;
  error?: string;
}

/** A clip opened from its thumbnail: a status chip, where it parts, what is being tried, its question, and the rest behind details. */
export function ClipDetail(p: ClipDetailProps) {
  const tried = p.job || p.agent || p.attempts.length || p.rounds.length;
  return (
    <aside className="drawer details" data-testid="clip-card" aria-label={`clip: ${p.name}`}>
      <header className="drawer-head">
        <div className="drawer-title">
          <span className="drawer-name" title={p.name}>
            {p.name}
          </span>
          <span className="hint">{p.sub}</span>
        </div>
        <span className={`chip ${STATUS_CLASS[p.status]}`} data-testid="clip-status">
          {p.status}
        </span>
        <button className="drawer-x" onClick={p.close} aria-label="close" data-testid="clip-close">
          x
        </button>
      </header>
      <div className="drawer-match">
        <MatchBar permille={p.permille} />
        <span className="num">{p.permille === undefined ? "-" : `${Math.round(p.permille / 10)}%`}</span>
      </div>

      {p.fixReady && (
        <p className="fix-ready-line" data-testid="fix-ready" title={p.fixReady.title}>
          <span className="chip fixready">fix ready</span> {p.fixReady.says}
        </p>
      )}

      {!p.fixReady && (p.needsYou.length > 0 || p.asks.length > 0) && (
        <section className="dsec ask-box" data-testid="clip-question">
          <h3>Needs you</h3>
          {p.asks.map((e) => (
            <p key={e.when_ms} className="ask-text" title={e.text}>
              {e.text}
            </p>
          ))}
          {p.needsYou.map((q) => (
            <Question key={q.id} q={q} answer={p.answer} big />
          ))}
        </section>
      )}

      <section className="dsec">
        <h3>Where it parts</h3>
        <PartsTimeline frames={p.frames} at={p.at} wrong={p.parts?.frame} />
        <p className="one-line" data-testid="clip-parts" title={p.parts ? `${p.parts.what}, from frame ${p.parts.frame}` : undefined}>
          {p.parts ? (
            <>
              <strong>{p.parts.what}</strong> <span className="hint">from frame {p.parts.frame}</span>
            </>
          ) : p.status === "matches" ? (
            "They match all the way."
          ) : (
            <span className="hint">Not scored yet.</span>
          )}
        </p>
      </section>

      <section className="dsec">
        <h3>Being tried</h3>
        {tried ? (
          <ul className="tries">
            {p.job && <JobItem j={p.job} />}
            {p.agent && (
              <li className="try working" data-testid="agent-status" title={p.agent.says}>
                <span className="try-title">
                  {p.agent.busy && <span className="spin" aria-hidden="true" />}
                  {p.agent.says}
                </span>
              </li>
            )}
            {p.attempts.slice(0, 5).map((a) => (
              <AttemptItem key={a.n} a={a} watching={p.watching === a.n} watch={p.watch} keep={p.keep} />
            ))}
            {p.rounds.slice(0, 3).map((r) => (
              <RoundItem key={`r${r.round}-${r.when_ms}`} g={p.g} r={r} />
            ))}
          </ul>
        ) : (
          <p className="hint">Nothing yet.</p>
        )}
        <div className="row-acts">
          {p.agent?.busy && p.stopAgent ? (
            <button onClick={p.stopAgent} data-testid="agent-stop">
              stop the agent
            </button>
          ) : (
            p.startAgent && (
              <button onClick={p.startAgent} data-testid="agent-start" title="Start Claude on this clip: it changes how the mechanic works in its own copy of the repo">
                let the agent fix it
              </button>
            )
          )}
          {p.fix && (
            <button className="primary" onClick={p.fix} disabled={p.fixing} data-testid="clip-fix" title="Turn the background work on and fix this clip first">
              {p.fixing ? "fixing..." : "fix this clip"}
            </button>
          )}
        </div>
      </section>

      {p.tell && <TellBox {...p} tell={p.tell} />}

      {p.other.length > 0 && (
        <details className="dsec">
          <summary>The loop's question{p.other.length > 1 ? `s (${p.other.length})` : ""}</summary>
          {p.other.map((q) => (
            <Question key={q.id} q={q} answer={p.answer} />
          ))}
        </details>
      )}

      <div className="row-acts marks">
        {(["kept", "skipped", "starred"] as const).map((m) => (
          <button key={m} className={p.mark === m ? "on" : undefined} onClick={() => p.setMark(m)} data-testid={`clip-${m}`}>
            {m === "kept" ? "keep" : m === "skipped" ? "skip" : "star"}
          </button>
        ))}
        {p.playOn && (
          <button onClick={p.playOn} data-testid="clip-play-on" title="Open the recorder at this clip's save state and play on from it">
            play on from here
          </button>
        )}
      </div>

      {p.more && (
        <details className="dsec">
          <summary>details</summary>
          {p.more}
        </details>
      )}
      {p.error && <p className="error">{p.error}</p>}
    </aside>
  );
}

function Question({ q, answer, big }: { q: BgQuestion; answer: (question: string, choice: string) => void; big?: boolean }) {
  return (
    <div className={big ? "q big" : "q"}>
      <p className={big ? "q-ask" : "q-ask one-line"} title={q.ask}>
        {q.ask}
        {q.at != null && <span className="hint"> at {secs(q.at)}</span>}
      </p>
      <div className="q-choices">
        {q.choices.map((ch) => (
          <button key={ch} className={big ? "primary" : undefined} onClick={() => answer(q.id, ch)}>
            {ch}
          </button>
        ))}
      </div>
      {q.detail && (
        <details>
          <summary>details</summary>
          <p className="hint">{q.detail}</p>
        </details>
      )}
    </div>
  );
}

/** Words for a tell's state under it. */
function tellState(t: WorkTell, agentBusy: boolean): string {
  if (!t.open) return "answered";
  return agentBusy || t.agent ? "the agent has it" : "waiting for an agent";
}

/** What came of a note: the green chip with before -> after, or "fixed once merged" while its fix waits. */
export function TellCreditRow({ t }: { t: WorkTell }) {
  const c = t.credit;
  if (!c || c.status === "open") return null;
  const good = c.status === "fixed" || c.status === "improved";
  return (
    <span className="tell-credit" data-testid="tell-credit" data-status={c.status}>
      <span className={good ? "chip resolved" : "chip fixready"}>{c.words}</span>
      {good && c.change && <span className="hint"> {c.change}</span>}
      {c.status === "fixed" && c.attempt != null && <span className="hint">{`, attempt ${c.attempt}`}</span>}
    </span>
  );
}

/** "tell the agent": one line and a send button, the point clicked on the screen, and each note sent with its reply under it. */
function TellBox(p: ClipDetailProps & { tell: (text: string) => Promise<void> }) {
  const [text, setText] = useState("");
  const [sending, setSending] = useState(false);
  const [err, setErr] = useState<string>();
  const send = async () => {
    if (!text.trim() || sending) return;
    setSending(true);
    setErr(undefined);
    try {
      await p.tell(text);
      setText("");
    } catch (e) {
      setErr((e as Error).message);
    } finally {
      setSending(false);
    }
  };
  const tells = [...(p.tells ?? [])].reverse();
  return (
    <section className="dsec tell-box" data-testid="tell-box">
      <h3>Tell the agent</h3>
      <div className="tell-line">
        <input
          type="text"
          value={text}
          placeholder="what's wrong here?"
          aria-label="tell the agent"
          onChange={(e) => setText(e.target.value)}
          onKeyDown={(e) => e.key === "Enter" && void send()}
          data-testid="tell-input"
        />
        <button className="primary" disabled={!text.trim() || sending} onClick={() => void send()} data-testid="tell-send">
          send
        </button>
      </div>
      <p className="hint one-line" data-testid="tell-point">
        {p.pointed ? (
          <>
            Pointing at {p.pointed}{" "}
            {p.clearPoint && (
              <button className="link" onClick={p.clearPoint} data-testid="tell-unpoint">
                clear
              </button>
            )}
          </>
        ) : (
          <>
            Click the screen to point; it pauses the clip.{" "}
            {p.setPaused && (
              <button className="link" onClick={() => p.setPaused!(!p.paused)} data-testid="tell-pause">
                {p.paused ? "play" : "pause"}
              </button>
            )}
          </>
        )}
      </p>
      {err && <p className="error">{err}</p>}
      {tells.length > 0 && (
        <ul className="tells" data-testid="tells">
          {tells.map((t) => (
            <li key={t.id} className={t.open ? "tell-row open" : "tell-row"} data-testid="tell-row">
              <span className="chip said" data-testid="you-said">
                you said
              </span>
              <span className="tell-text" title={t.text}>
                {t.text}
              </span>
              <span className="hint tell-meta">
                {t.frame != null ? `frame ${t.frame}` : ""} {tellState(t, !!p.agent?.busy)}
              </span>
              <TellCreditRow t={t} />
              {t.replies.map((r) => (
                <p key={r.when_ms} className="tell-reply" data-testid="tell-reply">
                  <span className="hint">{byWords(r.by)}: </span>
                  {r.text}
                </p>
              ))}
            </li>
          ))}
        </ul>
      )}
    </section>
  );
}
