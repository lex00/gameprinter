// One of the editor's views inside gp serve's one page (the mechanic page
// at /mechanic/GAME/, mechanic/details/): /index.html?embed=VIEW&run=GAME
// draws that view alone, with no header of its own, in the page's colors
// (styles.css, `.embed`). The page shows it in its REFINE tab or its
// Details drawer, so the person stays on one page.
//
// - `game`: a recording of the game measured against the rebuilt game
//   (GameView.tsx): both games at the same moment, the lockstep panel
//   and its curves, the moments, the scenario ladder, and pause, point
//   and tell (what is said goes to the agent in the mechanic's talk).
// - `inspector`: the trace inspector (App.tsx's Workspace with only
//   "inspector"): a recording's frames and every registered view.

import { useEffect, useState } from "react";
import type { Api, GameWords } from "./api";
import { GameView } from "./GameView";
import { personInput, sendInput, useLive } from "./Live";

/** The views ?embed= takes, besides `inspector` (App.tsx). */
export const EMBED_VIEWS = ["game"] as const;

export function Embed({ api, view, run }: { api: Api; view: string; run?: string }) {
  useEffect(() => {
    document.documentElement.classList.add("embed");
  }, []);
  if (!run) return <p className="hint">Give the game: ?embed={view}&amp;run=GAME.</p>;
  if (view === "game") return <EmbedGame api={api} run={run} />;
  return <p className="error">No view {view} to embed: {["inspector", ...EMBED_VIEWS].join(", ")}.</p>;
}

function EmbedGame({ api, run }: { api: Api; run: string }) {
  const events = useLive(api, run);
  const [words, setWords] = useState<GameWords>();
  const [error, setError] = useState<string>();
  const [said, setSaid] = useState<string>();
  useEffect(() => {
    let live = true;
    api
      .captureGame(run)
      .then((g) => live && setWords(g))
      .catch((e: Error) => live && setError(e.message));
    return () => {
      live = false;
    };
  }, [api, run]);
  const send = sendInput(api, run, [], events.state, setSaid);
  return (
    <div className="app embed" data-testid="embed-game">
      {error && <p className="error">{error}</p>}
      {said && <p className="hint">{said}</p>}
      <GameView api={api} run={run} words={words} live={events.state} play={events.play} send={(i) => send(personInput(i))} />
    </div>
  );
}
