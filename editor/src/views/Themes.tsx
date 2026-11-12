// Themes: pick a template's theme layers, edit a theme's live effects by
// role, preview the run drawn with them, and see the readability check
// (docs/inspector.md, "Themes"). The preview runs the open recording's
// input, or a baseline's, through the model in gp serve; the state digest
// it shows is the same with any theme. Saving writes only the player's own
// themes and stack, under $GP_HOME/themes/.

import { useEffect, useMemo, useState } from "react";
import type { Effect, Layer, PreviewJson, Readability, ThemeDoc, ThemesJson } from "../api";
import { registerView, type ViewProps } from "./registry";
import {
  addLayer,
  blankTheme,
  copyTheme,
  effectOf,
  isThemeId,
  moveLayer,
  ordered,
  removeLayer,
  ruleFor,
  setEffect,
  setRule,
  toggleLayer,
} from "./themeedit";

const verdictColor = { ok: "#4caf50", warn: "#e0a020", refuse: "#e04040" } as const;

export function ReadabilityTable({ r }: { r: Readability }) {
  return (
    <table className="small" data-testid="readability">
      <caption>
        Readability: <b style={{ color: verdictColor[r.verdict] }}>{r.verdict}</b>
      </caption>
      <tbody>
        {r.groups.map((g) => (
          <tr key={g.group}>
            <td>{g.group}</td>
            <td>{(g.distinct * 100).toFixed(0)}% distinct</td>
            <td>contrast {g.contrast.toFixed(2)}</td>
            <td style={{ color: verdictColor[g.verdict] }}>{g.verdict}</td>
            <td>{g.why ?? ""}</td>
          </tr>
        ))}
        {r.pairs.map((p) => (
          <tr key={`${p.a}-${p.b}`}>
            <td colSpan={2}>
              {p.a} vs {p.b}
            </td>
            <td>delta E {p.delta_e.toFixed(1)}</td>
            <td style={{ color: verdictColor[p.verdict] }}>{p.verdict}</td>
            <td />
          </tr>
        ))}
        {r.absent.length > 0 && (
          <tr>
            <td colSpan={5}>none in the model: {r.absent.join(", ")}</td>
          </tr>
        )}
      </tbody>
    </table>
  );
}

function Num({ label, value, onChange, step = 0.05 }: { label: string; value?: number; onChange: (v: string) => void; step?: number }) {
  return (
    <label>
      {label}{" "}
      <input type="number" step={step} value={value ?? ""} onChange={(e) => onChange(e.target.value)} style={{ width: "5em" }} />
    </label>
  );
}

/** Live effects of one theme: a palette rule per target, and the screen effects. */
function EffectsEditor({ theme, targets, onChange }: { theme: ThemeDoc; targets: string[]; onChange: (t: ThemeDoc) => void }) {
  const [target, setTarget] = useState("player");
  const rule = ruleFor(theme, target);
  const strength = (kind: "scanlines" | "vignette") => {
    const e = effectOf(theme, kind);
    return e && "strength" in e ? e.strength : undefined;
  };
  const setStrength = (kind: "scanlines" | "vignette", v: string) =>
    onChange(setEffect(theme, kind, undefined, v.trim() === "" ? undefined : ({ kind, strength: Number(v) } as Effect)));
  const glow = effectOf(theme, "glow", target);
  const particles = effectOf(theme, "particles");
  return (
    <div data-testid="effects">
      <div>
        Target{" "}
        <select value={target} onChange={(e) => setTarget(e.target.value)} data-testid="target">
          {targets.map((t) => (
            <option key={t}>{t}</option>
          ))}
        </select>{" "}
        <Num label="hue" step={5} value={rule.hue} onChange={(v) => onChange(setRule(theme, target, "hue", v))} />{" "}
        <Num label="saturation" value={rule.saturation} onChange={(v) => onChange(setRule(theme, target, "saturation", v))} />{" "}
        <Num label="lightness" value={rule.lightness} onChange={(v) => onChange(setRule(theme, target, "lightness", v))} />{" "}
        <label>
          tint <input type="color" value={rule.tint ?? "#000000"} onChange={(e) => onChange(setRule(theme, target, "tint", e.target.value))} />
        </label>{" "}
        <Num label="amount" value={rule.tint_amount} onChange={(v) => onChange(setRule(theme, target, "tint_amount", v))} />{" "}
        <label>
          <input
            type="checkbox"
            checked={!!glow}
            onChange={(e) =>
              onChange(setEffect(theme, "glow", target, e.target.checked ? { kind: "glow", target, color: "#ffe6a0", radius: 1 } : undefined))
            }
          />{" "}
          glow
        </label>
      </div>
      <div>
        Screen <Num label="scanlines" value={strength("scanlines")} onChange={(v) => setStrength("scanlines", v)} />{" "}
        <Num label="vignette" value={strength("vignette")} onChange={(v) => setStrength("vignette", v)} />{" "}
        <label>
          particles{" "}
          <select
            value={particles && "style" in particles ? particles.style : ""}
            onChange={(e) =>
              onChange(
                setEffect(
                  theme,
                  "particles",
                  undefined,
                  e.target.value
                    ? { kind: "particles", target: "screen", style: e.target.value as "fireflies", color: "#ffe27a", count: 24, seed: 1 }
                    : undefined,
                ),
              )
            }
          >
            <option value="">none</option>
            <option>fireflies</option>
            <option>snow</option>
            <option>rain</option>
            <option>sparks</option>
          </select>
        </label>
      </div>
    </div>
  );
}

function Themes({ api, a, frame }: ViewProps) {
  const [model, setModel] = useState(a.labels.game ?? a.show.recording.source_id ?? "");
  const [info, setInfo] = useState<ThemesJson>();
  const [layers, setLayers] = useState<Layer[]>([]);
  const [drafts, setDrafts] = useState<Record<string, ThemeDoc>>({});
  const [editing, setEditing] = useState<string>();
  const [source, setSource] = useState("recording");
  const [preview, setPreview] = useState<PreviewJson>();
  const [plain, setPlain] = useState<string>();
  const [busy, setBusy] = useState(false);
  const [message, setMessage] = useState<string>();
  const [error, setError] = useState<string>();

  useEffect(() => {
    setInfo(undefined);
    setError(undefined);
    if (!model) return;
    let live = true;
    api
      .themes(model)
      .then((t) => {
        if (!live) return;
        setInfo(t);
        setLayers(ordered(t.stack));
        setDrafts({});
      })
      .catch((e: Error) => live && setError(e.message));
    return () => {
      live = false;
    };
  }, [api, model]);

  const themeOf = (id: string): ThemeDoc | undefined => drafts[id] ?? info?.themes.find((t) => t.id === id)?.theme ?? undefined;
  const scopeOf = (id: string) => info?.themes.find((t) => t.id === id)?.scope ?? "personal";
  const targets = useMemo(() => (info ? ["*", ...info.groups, ...info.roles] : []), [info]);

  const request = (ls: Layer[]) => ({
    layers: ls,
    drafts,
    frames: [Math.max(1, frame)],
    ...(source === "recording" ? { recording: a.id } : { baseline: source }),
  });

  const run = () => {
    setBusy(true);
    setError(undefined);
    // The same run with no theme, to show the digest doesn't move.
    Promise.all([api.themePreview(model, request(layers)), api.themePreview(model, { ...request([]), drafts: {} })])
      .then(([p, q]) => {
        setPreview(p);
        setPlain(q.digest);
      })
      .catch((e: Error) => setError(e.message))
      .finally(() => setBusy(false));
  };

  const save = () => {
    const body: { theme?: ThemeDoc; layers?: Layer[] } = { layers: layers.filter((l) => l.scope === "personal") };
    if (editing && drafts[editing] && scopeOf(editing) === "personal") body.theme = drafts[editing];
    api
      .saveTheme(model, body)
      .then((r) => setMessage(`saved ${r.wrote.join(", ")}`))
      .catch((e: Error) => setError(e.message));
  };

  const newTheme = () => {
    const id = prompt("New theme id (lowercase letters, digits, - and _)");
    if (!id || !isThemeId(id) || !info) return;
    const base = editing ? themeOf(editing) : undefined;
    const t = base ? copyTheme(base, id) : blankTheme(id, id, info.model);
    setInfo({ ...info, themes: [...info.themes, { id, scope: "personal", theme: t, problem: null }] });
    setDrafts({ ...drafts, [id]: t });
    setLayers(addLayer(layers, id));
    setEditing(id);
  };

  if (!model) return <p className="muted">The recording names no game, so there's no template to theme.</p>;
  const current = editing ? themeOf(editing) : undefined;
  return (
    <div data-testid="themes">
      <div>
        Template <input value={model} onChange={(e) => setModel(e.target.value)} style={{ width: "10em" }} />
        {info && <span className="muted"> {info.title}</span>}
      </div>
      {error && <p className="error">{error}</p>}
      {info && (
        <>
          <ol data-testid="layers">
            {layers.map((l, i) => (
              <li key={l.theme}>
                <input type="checkbox" checked={l.enabled ?? true} onChange={() => setLayers(toggleLayer(layers, i))} />{" "}
                <button onClick={() => setEditing(l.theme)} disabled={editing === l.theme}>
                  {l.theme}
                </button>{" "}
                <span className="muted">{l.scope}</span> <button onClick={() => setLayers(moveLayer(layers, i, -1))}>up</button>
                <button onClick={() => setLayers(moveLayer(layers, i, 1))}>down</button>
                {l.scope === "personal" && <button onClick={() => setLayers(removeLayer(layers, i))}>remove</button>}
              </li>
            ))}
          </ol>
          <div>
            Add{" "}
            <select value="" onChange={(e) => e.target.value && setLayers(addLayer(layers, e.target.value))}>
              <option value="">a theme</option>
              {info.themes
                .filter((t) => !layers.some((l) => l.theme === t.id))
                .map((t) => (
                  <option key={t.id} value={t.id}>
                    {t.id} ({t.scope})
                  </option>
                ))}
            </select>{" "}
            <button onClick={newTheme}>new personal theme{editing ? ` from ${editing}` : ""}</button>
          </div>
          {current && (
            <fieldset>
              <legend>
                {current.title} ({editing}, {scopeOf(editing!)}){scopeOf(editing!) === "baked" && " - a baked theme; edits preview only, or make a personal copy"}
              </legend>
              <EffectsEditor theme={current} targets={targets} onChange={(t) => setDrafts({ ...drafts, [editing!]: t })} />
            </fieldset>
          )}
          <div>
            Preview on{" "}
            <select value={source} onChange={(e) => setSource(e.target.value)}>
              <option value="recording">this recording's input</option>
              {info.baselines.map((b) => (
                <option key={b} value={b}>
                  baseline {b}
                </option>
              ))}
            </select>{" "}
            at frame {Math.max(1, frame)}{" "}
            <button onClick={run} disabled={busy} data-testid="preview">
              {busy ? "drawing..." : "preview"}
            </button>{" "}
            <button onClick={save}>save my themes and stack</button>
            {message && <span className="muted"> {message}</span>}
          </div>
          {preview && (
            <div>
              {preview.frames.map((f) => (
                <img key={f.frame} alt={`frame ${f.frame}`} src={`data:image/png;base64,${f.png}`} style={{ width: 512, imageRendering: "pixelated" }} />
              ))}
              <p className="muted" data-testid="digest">
                state digest {preview.digest.slice(0, 16)} over {preview.length} frames
                {plain && (plain === preview.digest ? ", the same as with no theme" : `, NOT the same as with no theme (${plain.slice(0, 16)})`)}
              </p>
              <ReadabilityTable r={preview.readability} />
              {preview.style && (
                <p className="small">
                  Style: {preview.style.filter((m) => !m.pass).length} of {preview.style.length} measures fail
                  {preview.style
                    .filter((m) => !m.pass)
                    .map((m) => ` / ${m.part} ${m.metric} ${m.value.toFixed(2)} (wants ${m.bound})`)
                    .join("")}
                </p>
              )}
            </div>
          )}
        </>
      )}
    </div>
  );
}

registerView({ id: "themes", title: "Themes", order: 90, wide: true, component: Themes });
