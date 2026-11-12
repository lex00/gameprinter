// "agents" in the header (docs/show.md, "How many at once"): how many
// clip agents the background work hands its stuck clips to at the same
// time, 0 for none, as a small stepper. The server keeps it with the run
// (`serve/clip_agent.rs`, `/api/capture/{run}/background/agents`).

import { useEffect, useState } from "react";
import type { Api } from "./api";
import { Stepper } from "./Calm";

interface Agents {
  agents: number;
  max: number;
}

/** How many agents at once, 0 to the server's most, and the setter. */
export function useAgents(api: Api, run: string) {
  const path = `/api/capture/${encodeURIComponent(run)}/background/agents`;
  const [state, setState] = useState<Agents>();
  useEffect(() => {
    let live = true;
    api
      .json<Agents>(path)
      .then((s) => live && setState(s))
      .catch(() => undefined);
    return () => {
      live = false;
    };
  }, [api, path]);
  const set = (agents: number) => {
    api
      .post<Agents>(path, { agents })
      .then(setState)
      .catch(() => undefined);
  };
  return { state, set };
}

/** "agents [-] N [+]": kept with the run. */
export function AgentsControl({ api, run }: { api: Api; run: string }) {
  const { state, set } = useAgents(api, run);
  const n = state?.agents ?? 2;
  return (
    <Stepper
      label="agents"
      value={n}
      min={0}
      max={state?.max ?? n}
      set={set}
      disabled={!state}
      testid="ctl-agents"
      title="How many Claude agents work on stuck clips at the same time, each in its own copy of the repo. Clips stuck on the same missing mechanic share one agent. 0 hands none off."
    />
  );
}
