// One app (the owner, 2026-11-12: "i cant tell if its watch or mechanic and
// just having multiple screens separate will confuse people"): the editor's
// one header of tabs. "make clips" is the main screen (Unified.tsx); "help
// with mechanics", "review rebuild" and "free play on the model" are the
// mechanic page's three tabs, the page itself (gp serve's /mechanic/GAME/,
// with its own header hidden); "agent's log" opens the watch panel of the
// mechanic open now beside whichever view is shown (/watch/GAME/MECHANIC/).
// The view is in the URL (`view=`), so `gp mechanic start` opens the right
// one.

import { useEffect, useRef, useState } from "react";
import type { Api } from "./api";

export type View = "clips" | "mech" | "refine" | "free";

export const VIEWS: { id: View; label: string; title: string }[] = [
  { id: "clips", label: "make clips", title: "Play the original, make clips, and watch the background work score and fix them" },
  { id: "mech", label: "help with mechanics", title: "Record a clip of a mechanic and hand it to your agent; the quest log and the talk" },
  { id: "refine", label: "review rebuild", title: "A clip on the original with the rebuilt game's rule drawn over it" },
  { id: "free", label: "free play on the model", title: "Play the rebuilt game alone" },
];

const enc = encodeURIComponent;

/** The view a URL asks for: `view=mechanic` (as gp mechanic start opens it) is "help with mechanics". */
export function readView(search: string): View {
  const v = new URLSearchParams(search).get("view");
  if (v === "mechanic" || v === "mech") return "mech";
  return VIEWS.some((x) => x.id === v) ? (v as View) : "clips";
}

/** The URL with the view written in, the rest kept. */
export function writeView(search: string, v: View): string {
  const p = new URLSearchParams(search);
  if (v === "clips") p.delete("view");
  else p.set("view", v);
  const s = p.toString();
  return s ? `?${s}` : "?";
}

/** The mechanic page's address in the editor. */
export function mechanicUrl(api: Pick<Api, "url">, run: string, tab: View): string {
  return api.url(`/mechanic/${enc(run)}/?embedded=1&tab=${tab}`);
}

/** The watch panel's address. */
export function watchUrl(api: Pick<Api, "url">, game: string, topic: string): string {
  return api.url(`/watch/${enc(game)}/${enc(topic)}/`);
}

/** The one header's tabs, and the agent's log beside them. */
export function ViewTabs({ view, setView, log, setLog }: { view: View; setView: (v: View) => void; log: boolean; setLog: (on: boolean) => void }) {
  return (
    <nav className="view-tabs" data-testid="view-tabs">
      {VIEWS.map((v) => (
        <button key={v.id} className={view === v.id ? "on" : undefined} aria-pressed={view === v.id} title={v.title} onClick={() => setView(v.id)} data-testid={`view-${v.id}`}>
          {v.label}
        </button>
      ))}
      <button className={log ? "on log" : "log"} aria-pressed={log} onClick={() => setLog(!log)} data-testid="view-log" title="What your agent is doing: its log, pins and problem table, and a box for your notes">
        agent's log
      </button>
    </nav>
  );
}

/** The mechanic page in the editor, kept loaded while its tabs are switched. */
export function MechanicView({ api, run, view }: { api: Api; run: string; view: View }) {
  const frame = useRef<HTMLIFrameElement>(null);
  // The address the frame first opened at, with the tab asked for; later
  // tabs go by message, so a recording in progress isn't reloaded away.
  const [src] = useState(() => mechanicUrl(api, run, view));
  useEffect(() => {
    frame.current?.contentWindow?.postMessage({ gp: "tab", tab: view }, location.origin);
  }, [view]);
  return <iframe ref={frame} className="mechanic-view" title="help with mechanics" src={src} data-testid="mechanic-view" allow="gamepad" />;
}

/** The mechanic open now in the game's page, for the agent's log beside it. */
export function useMechanic(api: Api, run: string | undefined): string | undefined {
  const [m, setM] = useState<string>();
  useEffect(() => {
    if (!run) return;
    let live = true;
    const get = () =>
      api
        .json<{ mechanic: string }>(`/api/mechanic/${enc(run)}`)
        .then((r) => live && setM(r.mechanic))
        .catch(() => undefined);
    void get();
    const t = setInterval(get, 5000);
    return () => {
      live = false;
      clearInterval(t);
    };
  }, [api, run]);
  return m;
}

/** The watch panel: the agent's log, pins and problem rows of a session. */
export function LogPanel({ api, game, topic }: { api: Api; game: string; topic?: string }) {
  if (!topic) return <aside className="log-panel" data-testid="log-panel"><p className="hint">No mechanic is open for this game yet: open one in "help with mechanics".</p></aside>;
  return (
    <aside className="log-panel" data-testid="log-panel">
      <iframe title="agent's log" src={watchUrl(api, game, topic)} data-testid="log-frame" />
    </aside>
  );
}
