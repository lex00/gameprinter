// Pure helpers for the themes view (Themes.tsx): editing a stack's layers
// and a theme's live effects without touching the server. Kept apart from
// the component so the tests can run them directly.

import type { Effect, Layer, PaletteRule, ThemeDoc } from "../api";

/** Layers as the server orders them: baked first, then personal, each in order. */
export function ordered(layers: Layer[]): Layer[] {
  return [...layers.filter((l) => l.scope === "baked"), ...layers.filter((l) => l.scope === "personal")];
}

/** Moves layer `i` up (-1) or down (+1) within its scope; across scopes it stays. */
export function moveLayer(layers: Layer[], i: number, by: -1 | 1): Layer[] {
  const j = i + by;
  if (j < 0 || j >= layers.length || layers[i].scope !== layers[j].scope) return layers;
  const out = [...layers];
  [out[i], out[j]] = [out[j], out[i]];
  return out;
}

export function toggleLayer(layers: Layer[], i: number): Layer[] {
  return layers.map((l, k) => (k === i ? { ...l, enabled: !(l.enabled ?? true) } : l));
}

/** Adds a personal layer for a theme, on top, unless it's in the stack. */
export function addLayer(layers: Layer[], theme: string): Layer[] {
  if (layers.some((l) => l.theme === theme)) return layers;
  return [...layers, { theme, scope: "personal" }];
}

export function removeLayer(layers: Layer[], i: number): Layer[] {
  return layers.filter((_, k) => k !== i);
}

/** The rule for exactly `target`, or an empty one. */
export function ruleFor(theme: ThemeDoc, target: string): PaletteRule {
  return theme.palette?.find((r) => r.target === target) ?? { target };
}

type RuleField = "hue" | "saturation" | "lightness" | "tint" | "tint_amount";

/** Sets one field of the rule for `target`; an empty value removes it, and a rule with nothing left goes. */
export function setRule(theme: ThemeDoc, target: string, field: RuleField, value: string): ThemeDoc {
  const rule: PaletteRule = { ...ruleFor(theme, target) };
  if (value.trim() === "") delete rule[field];
  else if (field === "tint") rule.tint = value;
  else {
    const n = Number(value);
    if (!Number.isFinite(n)) return theme;
    rule[field] = n;
  }
  const rest = (theme.palette ?? []).filter((r) => r.target !== target);
  const empty = Object.keys(rule).every((k) => k === "target");
  return { ...theme, palette: empty ? rest : [...rest, rule] };
}

/** The theme's effect of a kind (and target, for those with one). */
export function effectOf(theme: ThemeDoc, kind: Effect["kind"], target?: string): Effect | undefined {
  return theme.effect?.find((e) => e.kind === kind && (target === undefined || ("target" in e && e.target === target)));
}

/** Puts an effect in place of the one of its kind and target, or removes that one with `undefined`. */
export function setEffect(theme: ThemeDoc, kind: Effect["kind"], target: string | undefined, e: Effect | undefined): ThemeDoc {
  const same = (x: Effect) => x.kind === kind && (target === undefined || ("target" in x && x.target === target));
  const rest = (theme.effect ?? []).filter((x) => !same(x));
  return { ...theme, effect: e ? [...rest, e] : rest };
}

/** A new personal theme with no effects. */
export function blankTheme(id: string, title: string, model: string): ThemeDoc {
  return { format: "0.1.0", id, title, license: "CC0-1.0", for: [model] };
}

/** A copy of a theme under a new id, for editing a baked one as the player's own. */
export function copyTheme(theme: ThemeDoc, id: string): ThemeDoc {
  return { ...theme, id, title: `${theme.title} (mine)`, restyle: undefined };
}

export function isThemeId(s: string): boolean {
  return /^[a-z][a-z0-9_-]*$/.test(s);
}
