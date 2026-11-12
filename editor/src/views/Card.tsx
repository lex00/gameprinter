// The resolution card (docs/inspector.md, "Resolution cards"): what the
// model says about the entity or path picked in a selection or the pair
// view. Its facts with the evidence behind each and the contract level
// that governs it, its values at this frame (with the RAM that holds them
// in the original), and what `gp validate` measured against the original.

import { useEffect, useState } from "react";
import type { CardCheck, CardFact, CardJson } from "../api";
import { setShared, useShared } from "../shared";
import { registerView, type ViewProps } from "./registry";

/** A fact's value as text. */
export function factText(v: unknown): string {
  return typeof v === "string" ? v : JSON.stringify(v);
}

/** A check's measured line: expected, actual and tolerance. */
export function checkText(c: CardCheck): string {
  const parts = [c.expected !== null ? `expected ${c.expected}` : "", c.actual !== null ? `measured ${c.actual}` : "", c.tolerance ? `within ${c.tolerance}` : ""];
  return parts.filter(Boolean).join(", ") || (c.detail ?? "");
}

function Fact({ f }: { f: CardFact }) {
  return (
    <tr>
      <td>
        <button className="link" title="change this value" onClick={() => setShared({ changePath: f.path })}>
          {f.path}
        </button>
      </td>
      <td>{factText(f.value)}</td>
      <td>{f.level ?? ""}</td>
      <td className="hint">{f.evidence ? `${f.evidence.source} ${f.evidence.confidence}%` : "no evidence"}</td>
      <td className="hint">{f.evidence?.refs.map((r) => r.at).join(", ") ?? ""}</td>
    </tr>
  );
}

/** The card's contents. Exported for the tests. */
export function CardBody({ card }: { card: CardJson }) {
  return (
    <div className="card" data-testid="card">
      <h3>
        <code>{card.path}</code> <span className="hint">in {card.model.name}, contract level {card.level ?? "unknown"}</span>
      </h3>
      <h4>Values at frame {card.frame}</h4>
      {card.values.length === 0 ? (
        <p className="hint">The recording holds no value under this path.</p>
      ) : (
        <table>
          <tbody>
            {card.values.map((v) => (
              <tr key={v.path}>
                <td>
                  <code>{v.path}</code>
                </td>
                <td>{v.value}</td>
                <td className="hint">{(v.ram ?? []).map((a) => `$${a.toString(16).padStart(4, "0")}`).join(" ")}</td>
                <td className="hint">{(v.labels ?? []).join(", ")}</td>
              </tr>
            ))}
          </tbody>
        </table>
      )}
      <h4>Measured against the original</h4>
      {card.checks.length === 0 ? (
        <p className="hint">`gp validate` has no stored result for this part. Run it with this recording named in --against.</p>
      ) : (
        <table data-testid="card-checks">
          <tbody>
            {card.checks.map((c, i) => (
              <tr key={i} className={c.status === "fail" ? "differs" : ""}>
                <td>
                  <code>{c.part}</code> {c.quantity}
                  {c.case ? ` (${c.case})` : ""}
                </td>
                <td>{c.status}</td>
                <td>
                  {c.level}, against {c.against}
                </td>
                <td className="hint">{checkText(c)}</td>
              </tr>
            ))}
          </tbody>
        </table>
      )}
      <h4>
        Model facts{card.facts_total > card.facts.length ? ` (${card.facts.length} of ${card.facts_total})` : ""}
      </h4>
      <table data-testid="card-facts">
        <tbody>
          {card.facts.map((f) => (
            <Fact key={f.path} f={f} />
          ))}
        </tbody>
      </table>
    </div>
  );
}

function Card({ api, a, frame }: ViewProps) {
  const { cardPath } = useShared();
  const [card, setCard] = useState<CardJson>();
  const [error, setError] = useState<string>();
  useEffect(() => {
    setError(undefined);
    if (!cardPath) {
      setCard(undefined);
      return;
    }
    const abort = new AbortController();
    api
      .card(a.id, cardPath, frame, undefined, abort.signal)
      .then(setCard)
      .catch((e: Error) => {
        if (!abort.signal.aborted) setError(e.message);
      });
    return () => abort.abort();
  }, [api, a.id, cardPath, frame]);
  if (!cardPath) return <p className="hint">Pick an entity in the box selection or a path in the pair view.</p>;
  return (
    <div>
      {error && <p className="error">{error}</p>}
      {card && <CardBody card={card} />}
    </div>
  );
}

registerView({ id: "card", title: "Resolution card", order: 27, wide: true, component: Card });
