// "Play the rebuild" (docs/show.md, "Play the rebuild"): the render screen
// turned into the playable rebuilt game. It is the run's preview without
// control=1, in an iframe that takes the keyboard and the gamepad, with a bar
// under it for the physics (a profile and its switches), the level, and the
// keys. The clip loop is paused by the caller while this is open.

import { useEffect, useRef, useState } from "react";
import type { Api } from "./api";

/** What preview.json says that this bar uses (docs/web.md). A preview built before `levels` has none. */
export type PreviewInfo = {
  title?: string;
  profiles?: string[];
  levels?: string[];
  switches?: { group: string; title?: string; switches: { name: string; description?: string }[] }[];
};

export const CONTROLS = "arrows move, Z jump, X run, Enter start, F9 widescreen, F10 dim flashes, 5-9 mute";

/** The profile the page is asked for: a preset and the switches on, joined by "+"; "retro" is the model as written. */
export function profileName(preset: string, on: string[]): string {
  const parts = [...(preset === "retro" ? [] : [preset]), ...on];
  return parts.length ? parts.join("+") : "retro";
}

/** The playable preview's page: no control=1, so the page reads the keyboard and the gamepad itself. */
export function playUrl(api: Pick<Api, "capturePreview">, run: string, profile: string, level?: string): string {
  const q = new URLSearchParams();
  if (profile !== "retro") q.set("profile", profile);
  if (level) q.set("level", level);
  const s = q.toString().replace(/%2B/g, "+");
  return api.capturePreview(run, s ? `index.html?${s}` : "index.html");
}

export function PlayRebuild({ api, run, back }: { api: Pick<Api, "capturePreview">; run: string; back: () => void }) {
  const [info, setInfo] = useState<PreviewInfo>({});
  const [preset, setPreset] = useState("retro");
  const [on, setOn] = useState<string[]>([]);
  const [level, setLevel] = useState("");
  const frame = useRef<HTMLIFrameElement>(null);
  useEffect(() => {
    let live = true;
    fetch(api.capturePreview(run, "preview.json"))
      .then((r) => (r.ok ? r.json() : null))
      .then((c: PreviewInfo | null) => live && setInfo(c ?? {}))
      .catch(() => live && setInfo({}));
    return () => {
      live = false;
    };
  }, [api, run]);
  const src = playUrl(api, run, profileName(preset, on), level || undefined);
  // The keys go to the game: focus the frame as it loads and whenever the screen is clicked.
  const focus = () => frame.current?.contentWindow?.focus();
  useEffect(() => {
    const t = setTimeout(focus, 50);
    return () => clearTimeout(t);
  }, [src]);
  const presets = ["retro", ...(info.profiles ?? []).filter((p) => p !== "retro")];
  const switches = (info.switches ?? []).flatMap((g) => g.switches.map((s) => ({ ...s, group: g.group })));
  const levels = info.levels ?? [];
  return (
    <div className="play-rebuild" data-testid="play-rebuild">
      <figure className="play-screen-host">
        <iframe
          key={src}
          ref={frame}
          title="the model, playable"
          src={src}
          allow="gamepad; autoplay"
          tabIndex={0}
          onLoad={focus}
          className="play-frame"
          data-testid="play-frame"
        />
      </figure>
      <div className="play-bar" data-testid="play-bar">
        {presets.length > 1 && (
          <label title="The physics: retro is the original's, normal fixes its known quirks, assist adds help">
            physics{" "}
            <select value={preset} onChange={(e) => setPreset(e.target.value)} data-testid="play-profile">
              {presets.map((p) => (
                <option key={p} value={p}>
                  {p}
                </option>
              ))}
            </select>
          </label>
        )}
        {switches.map((s) => (
          <label key={s.name} title={s.description}>
            <input type="checkbox" checked={on.includes(s.name)} onChange={(e) => setOn(e.target.checked ? [...on, s.name] : on.filter((x) => x !== s.name))} data-testid={`play-switch-${s.name}`} /> {s.name.replace(/_/g, " ")}
          </label>
        ))}
        {levels.length > 1 && (
          <label title="Start at a level">
            level{" "}
            <select value={level} onChange={(e) => setLevel(e.target.value)} data-testid="play-level">
              <option value="">first</option>
              {levels.map((l) => (
                <option key={l} value={l}>
                  {l}
                </option>
              ))}
            </select>
          </label>
        )}
        <button onClick={back} data-testid="play-back" title="Back to the clips">
          back to clips
        </button>
      </div>
      <p className="play-keys" data-testid="play-keys">
        {CONTROLS}
      </p>
    </div>
  );
}
