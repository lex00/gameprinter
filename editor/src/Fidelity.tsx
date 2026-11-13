// The fidelity score on the main screen (docs/fidelity.md, docs/show.md):
// one line under the progress headline, "Fidelity 71 / 100 (behaviour 78,
// coverage 64, picture 70)", with how much evidence is behind it, a small
// trend over models and the three things holding it down.
import { useEffect, useState } from "react";
import type { Api } from "./api";

export interface FidelityPart {
  score: number;
  ppm: number;
}

export interface FidelityEntry {
  model: string;
  score: number;
  behaviour: number;
  coverage: number;
  picture?: number | null;
  clips: number;
  after: string;
}

/** `GET /api/capture/{run}/fidelity`, as much as the line reads. */
export interface FidelityState {
  version: string;
  score: number;
  behaviour: FidelityPart & { clips: number; exact: number };
  coverage: FidelityPart;
  picture?: FidelityPart | null;
  confidence: { level: "low" | "medium" | "high"; says: string };
  holding: { part: string; says: string; milli_points: number }[];
  says: string;
  trend: string;
  history: FidelityEntry[];
}

/** How often the line asks again: the score moves only as clips are scored. */
const POLL_MS = 15000;

export function useFidelity(api: Api, run: string): FidelityState | undefined {
  const [state, setState] = useState<FidelityState>();
  useEffect(() => {
    let live = true;
    const get = async () => {
      try {
        const s = await api.json<FidelityState>(`/api/capture/${encodeURIComponent(run)}/fidelity`);
        if (live) setState(s);
      } catch {
        // The server may be gone a moment; the next poll tries again.
      }
    };
    void get();
    const t = setInterval(() => void get(), POLL_MS);
    return () => {
      live = false;
      clearInterval(t);
    };
  }, [api, run]);
  return state;
}

/** The parts in words: "behaviour 78, coverage 64, picture 70". */
export function partsWords(f: FidelityState): string {
  const pic = f.picture ? `picture ${f.picture.score}` : "picture not measured";
  return `behaviour ${f.behaviour.score}, coverage ${f.coverage.score}, ${pic}`;
}

/** The last scores by model, oldest first, for the trend: at most `n`. */
export function trendScores(f: FidelityState, n = 10): number[] {
  return f.history.slice(-n).map((e) => e.score);
}

/** A small line of the scores, 0 to 100, as SVG points. */
export function sparkPoints(scores: number[], w = 60, h = 16): string {
  if (scores.length < 2) return "";
  const step = w / (scores.length - 1);
  return scores.map((s, i) => `${Math.round(i * step * 10) / 10},${Math.round((h - (h * Math.max(0, Math.min(100, s))) / 100) * 10) / 10}`).join(" ");
}

export function FidelityLine({ f }: { f: FidelityState }) {
  const points = sparkPoints(trendScores(f));
  const low = f.confidence.level === "low";
  return (
    <div className="progress-row fidelity" data-testid="fidelity">
      <span className="progress-label" title={`The fidelity score (${f.version}): how close the model's play is to the original's, from every clip's score (docs/fidelity.md)`}>
        Fidelity
      </span>
      <span>
        <strong data-testid="fidelity-score">{f.score}</strong>
        {` / 100 (${partsWords(f)})`}
        {low && <span className="hint"> low evidence</span>}
        {points && (
          <svg width="60" height="16" viewBox="0 0 60 16" aria-label={`trend: ${trendScores(f).join(", ")}`} style={{ verticalAlign: "middle", marginLeft: "0.5em" }} data-testid="fidelity-trend">
            <polyline points={points} fill="none" stroke="currentColor" strokeWidth="1.5" />
          </svg>
        )}
        {f.trend && <span className="hint"> {f.trend}</span>}
      </span>
      <span className="hint one-line" title={[f.confidence.says, ...f.holding.map((h) => h.says)].join("\n")}>
        {f.confidence.says}
        {f.holding.length > 0 && (
          <>
            {". Holding it down: "}
            {f.holding.map((h, i) => (
              <span key={h.says} data-testid="fidelity-holding">
                {i ? "; " : ""}
                {h.says}
              </span>
            ))}
            {"."}
          </>
        )}
      </span>
    </div>
  );
}

/** The fidelity line for a run, once the server has given it. */
export function Fidelity({ api, run }: { api: Api; run: string }) {
  const f = useFidelity(api, run);
  return f ? <FidelityLine f={f} /> : null;
}
