// Describe a change (docs/inspector.md, "Describing a change"): pick a
// model path and a new value, run the model with that change through the
// model run's input, and see where the changed run's entities go as ghosts
// over the original's frame. Several variants run at once, each in its
// own color. The change is structured: free text maps to a path and value
// through the MCP server's model_edit, which this view doesn't call yet.

import { useEffect, useMemo, useState } from "react";
import type { FactsJson, VariantDraft, VariantRun } from "../api";
import { setShared, useShared } from "../shared";
import { SCREEN_H, SCREEN_W } from "./boxsel";
import { originalFrameOf } from "./pairing";
import { sprites } from "./ppu";
import { registerView, type ViewProps } from "./registry";
import { editOf, GHOST_COLORS, ghostsAt, offsetFrom, runnable, summary } from "./changes";

/** The ghosts of each run over a picture, at a model frame. Exported for the tests. */
export function Ghosts({ runs, modelFrame, hidden }: { runs: VariantRun[]; modelFrame: number; hidden?: Set<string> }) {
  return (
    <>
      {runs.map((run, i) =>
        hidden?.has(run.name)
          ? null
          : ghostsAt(run, modelFrame).map((g) => (
              <rect
                key={`${run.name}-${g.path}`}
                x={g.x}
                y={g.y}
                width={g.w}
                height={g.h}
                className="ghost"
                style={{ fill: GHOST_COLORS[i % GHOST_COLORS.length], stroke: GHOST_COLORS[i % GHOST_COLORS.length] }}
                data-testid="ghost"
              >
                <title>{`${run.name}: ${g.path}`}</title>
              </rect>
            )),
      )}
    </>
  );
}

function Variants({ api, a, m, frame, pair, setFrame }: ViewProps) {
  const { changePath } = useShared();
  const [facts, setFacts] = useState<FactsJson>();
  const [path, setPath] = useState("");
  const [value, setValue] = useState("");
  const [variants, setVariants] = useState<VariantDraft[]>([{ name: "variant 1", edits: [] }]);
  const [at, setAt] = useState(0);
  const [runs, setRuns] = useState<VariantRun[]>();
  const [hidden, setHidden] = useState<Set<string>>(new Set());
  const [error, setError] = useState<string>();
  const [busy, setBusy] = useState(false);

  useEffect(() => {
    if (changePath) setPath(changePath);
  }, [changePath]);
  useEffect(() => {
    if (!m) return;
    api
      .facts(m.id)
      .then(setFacts)
      .catch((e: Error) => setError(e.message));
  }, [api, m?.id]);
  // Results belong to the model run and the changes they were run with.
  useEffect(() => setRuns(undefined), [m?.id]);

  const current = useMemo(() => facts?.facts.find((f) => f.path === path), [facts, path]);
  if (!m) return <p className="hint">Pick a model run in the header; changes run on its input.</p>;

  const edit = editOf(path, value);
  const addEdit = () => {
    if (!edit) return;
    setVariants((vs) => vs.map((v, i) => (i === at ? { ...v, edits: [...v.edits.filter((e) => e.path !== edit.path), edit] } : v)));
    setValue("");
  };
  const preview = () => {
    setBusy(true);
    setError(undefined);
    api
      .variants(m.id, runnable(variants))
      .then((r) => setRuns(r.variants))
      .catch((e: Error) => setError(e.message))
      .finally(() => setBusy(false));
  };
  const modelFrame = m.frame;
  const list = sprites(a.state?.get("oam")).filter((s) => !s.hidden);
  const tall = ((a.state?.get("ppu.ctrl")?.[0] ?? 0) & 0x20) !== 0;
  const base = runs?.[0];

  return (
    <div className="variants-view">
      <div className="change-form">
        <label>
          model path{" "}
          <input list="fact-paths" value={path} onChange={(e) => setPath(e.target.value)} placeholder="entities.player.jump.gravity" data-testid="change-path" />
        </label>
        <datalist id="fact-paths">
          {facts?.facts.map((f) => (
            <option key={f.path} value={f.path} />
          ))}
        </datalist>
        <label>
          new value{" "}
          <input value={value} onChange={(e) => setValue(e.target.value)} placeholder={current ? String(current.value) : "0.125"} data-testid="change-value" />
        </label>
        <label>
          into{" "}
          <select value={at} onChange={(e) => setAt(Number(e.target.value))}>
            {variants.map((v, i) => (
              <option key={i} value={i}>
                {v.name}
              </option>
            ))}
          </select>
        </label>
        <button onClick={addEdit} disabled={!edit} data-testid="change-add">
          add change
        </button>
        <button onClick={() => setVariants((vs) => [...vs, { name: `variant ${vs.length + 1}`, edits: [] }])} disabled={variants.length >= 8}>
          new variant
        </button>
        <button onClick={preview} disabled={busy || runnable(variants).length === 0} data-testid="change-preview">
          {busy ? "running" : "preview"}
        </button>
        {current && (
          <span className="hint">
            now {String(current.value)}, contract level {current.level ?? "unknown"}
          </span>
        )}
      </div>
      <ul className="variants">
        {variants.map((v, i) => (
          <li key={i}>
            <span className="swatch" style={{ background: GHOST_COLORS[(i + 1) % GHOST_COLORS.length] }} /> {v.name}:{" "}
            {v.edits.length === 0 ? <span className="hint">no changes yet</span> : v.edits.map((e) => `${e.path} = ${JSON.stringify(e.value)}`).join("; ")}{" "}
            {v.edits.length > 0 && (
              <button className="link" onClick={() => setVariants((vs) => vs.map((x, j) => (j === i ? { ...x, edits: [] } : x)))}>
                clear
              </button>
            )}
          </li>
        ))}
      </ul>
      {error && <p className="error">{error}</p>}
      {runs && (
        <div className="side-by-side">
          <div>
            <div className="pane-name">original, frame {frame}, with the changed runs at model frame {modelFrame}</div>
            <svg viewBox={`0 0 ${SCREEN_W} ${SCREEN_H}`} className="screen" data-testid="ghost-screen">
              <rect width={SCREEN_W} height={SCREEN_H} className="screen-bg" />
              {list.map((s) => (
                <rect key={s.index} x={s.x} y={s.y + 1} width="8" height={tall ? 16 : 8} className="sprite" />
              ))}
              <Ghosts runs={runs} modelFrame={modelFrame} hidden={hidden} />
            </svg>
          </div>
          <table className="runs" data-testid="variant-runs">
            <tbody>
              {runs.map((r, i) => {
                const player = r.entities?.[0]?.path;
                const off = base && player && i > 0 ? offsetFrom(base, r, player, modelFrame) : undefined;
                const frameOf = r.first_difference !== null && r.first_difference !== undefined ? originalFrameOf(pair, r.first_difference) : undefined;
                return (
                  <tr key={r.name} className={r.ok ? "" : "differs"}>
                    <td>
                      <label>
                        <input
                          type="checkbox"
                          checked={!hidden.has(r.name)}
                          onChange={() =>
                            setHidden((h) => {
                              const n = new Set(h);
                              if (!n.delete(r.name)) n.add(r.name);
                              return n;
                            })
                          }
                        />{" "}
                        <span className="swatch" style={{ background: GHOST_COLORS[i % GHOST_COLORS.length] }} /> {r.name}
                      </label>
                    </td>
                    <td>{summary(r)}</td>
                    <td className="hint">{off ? `${player} is ${off.dx},${off.dy} pixels from the model's here` : ""}</td>
                    <td>
                      {frameOf !== undefined && (
                        <button className="link" onClick={() => setFrame(frameOf)}>
                          go to frame {frameOf}
                        </button>
                      )}
                      {r.edits?.map((e) => (
                        <button key={e.path} className="link" onClick={() => setShared({ cardPath: e.path.split(".").slice(0, 2).join(".") })}>
                          card for {e.path.split(".").slice(0, 2).join(".")}
                        </button>
                      ))}
                    </td>
                  </tr>
                );
              })}
            </tbody>
          </table>
        </div>
      )}
      <p className="hint">Free text isn't mapped to a path yet: the MCP server's model_edit takes the same path and value.</p>
    </div>
  );
}

registerView({ id: "variants", title: "Describe a change", order: 35, wide: true, component: Variants });
