// A capture run's sprite edits in the live panel (docs/sprite-editing.md):
// each edit an agent proposed or placed with `assets_edit`, its before and
// after picture, the recorded frames it shows in (one row a pose, before
// beside after), its checks, and accept and reject buttons. A decision is
// applied by the server when it's sent, so the status updates on the next
// poll; the agent sees it through `assets_history`.

import { useEffect, useState } from "react";
import type { Api, AssetEdit } from "./api";

/** How often the edits are asked for while a run is open. */
export const EDITS_POLL_MS = 1000;

export function useEdits(api: Api, run: string, pollMs = EDITS_POLL_MS): { edits: AssetEdit[]; reload: () => void } {
  const [edits, setEdits] = useState<AssetEdit[]>([]);
  const [tick, setTick] = useState(0);
  useEffect(() => {
    let live = true;
    let timer: ReturnType<typeof setTimeout> | undefined;
    const poll = async () => {
      try {
        const r = await api.captureEdits(run);
        if (live) setEdits(r.edits);
      } catch {
        // A run without edits, or a server without the route: nothing to show.
      }
      if (live) timer = setTimeout(poll, pollMs);
    };
    void poll();
    return () => {
      live = false;
      if (timer) clearTimeout(timer);
    };
  }, [api, run, pollMs, tick]);
  return { edits, reload: () => setTick((t) => t + 1) };
}

/** Edits a person can still decide: proposed or placed by the agent. */
export const decidable = (e: AssetEdit): boolean => e.data.status === "proposed" || e.data.status === "placed";

/** Jumps the game view to a recorded moment: a baseline and a frame. */
export type Watch = (baseline: string, frame: number) => void;

/** An edit in plain words, for the game view: who changed whose picture, how. */
export function editSentence(e: AssetEdit): string {
  const d = e.data;
  // A level tile kind's look names the tile ("the stone block's picture").
  const who = (d.tile || d.entity).replace(/_/g, " ");
  const what = d.prompt ? `"${d.prompt}"` : (d.ops ?? []).join(", ");
  const verb: Record<AssetEdit["data"]["status"], string> = {
    proposed: "The agent proposes a change to",
    placed: "The agent changed",
    accepted: "You kept a change to",
    rejected: "You turned down a change to",
    undone: "The agent undid a change to",
  };
  const frames = d.frames && d.frames.length > 1 ? ` (${d.frames.length} pictures)` : "";
  return `${verb[d.status]} the ${who}'s picture${frames}: ${what}`;
}

export function EditCard({ e, api, run, decide, watch, plain }: { e: AssetEdit; api: Api; run: string; decide: (accept: boolean) => void; watch?: Watch; plain?: boolean }) {
  const [copy, seq] = e.item.split(":");
  const d = e.data;
  return (
    <div className={`asset-edit ${d.status}`} data-testid="asset-edit">
      {plain ? (
        <p>{editSentence(e)}</p>
      ) : (
        <>
          <p>
            <b>{e.item}</b> {d.status}
            {d.decided_by ? ` by ${d.decided_by}` : ""}: {d.tile || d.entity}
            {d.frames && d.frames.length ? ` (${d.frames.join(", ")})` : ""}, {(d.ops ?? []).join(", ")}
          </p>
          {d.prompt && <p className="prompt">"{d.prompt}"</p>}
        </>
      )}
      {d.previews?.edit && <img alt="before and after" src={api.captureEditPreview(run, copy, seq, "edit")} />}
      {d.previews?.context && (
        <figure>
          <img alt="in the recorded frames, before and after" src={api.captureEditPreview(run, copy, seq, "context")} />
          <figcaption>
            {(d.shots ?? []).map((s) => (
              <span key={s.row} className="shot" title={`${s.baseline} frame ${s.frame}`}>
                {s.row + 1}: {s.pose}
                {watch && (
                  <button onClick={() => watch(s.baseline, s.frame)} title="play the two games at this moment">
                    watch
                  </button>
                )}
              </span>
            ))}
          </figcaption>
        </figure>
      )}
      <ul className="checks">
        {(d.checks ?? []).map((c, i) => (
          <li key={i} className={c.verdict}>
            {`${c.name} ${c.verdict}: ${c.message}`}
          </li>
        ))}
        {d.critic !== undefined && d.critic !== null && <li>critic {Number(d.critic).toFixed(2)}</li>}
      </ul>
      {decidable(e) && (
        <p>
          <button onClick={() => decide(true)}>{d.status === "proposed" ? "accept (place it)" : "accept (keep it)"}</button>
          <button onClick={() => decide(false)}>{d.status === "proposed" ? "reject" : "reject (undo it)"}</button>
        </p>
      )}
    </div>
  );
}

/**
 * The run's sprite edits. In the game view (`plain`), only those still to
 * decide, in plain words, with a watch button per recorded moment that
 * plays both games there; in Details, every edit with its ids and checks.
 */
export function AssetEdits({ api, run, watch, plain }: { api: Api; run: string; watch?: Watch; plain?: boolean }) {
  const { edits: all, reload } = useEdits(api, run);
  const edits = plain ? all.filter(decidable) : all;
  const [said, setSaid] = useState<string>();
  if (!edits.length) return null;
  const decide = (item: string, accept: boolean) => {
    api
      .captureInput(run, { v: 1, actor: "person", kind: accept ? "accept" : "reject", item })
      .then((r) => {
        setSaid(r.decision_error ? `${item}: ${r.decision_error}` : `${item} ${String(r.decision?.status ?? "sent")}`);
        reload();
      })
      .catch((err: Error) => setSaid(err.message));
  };
  return (
    <section className="asset-edits" data-testid="asset-edits">
      {plain ? (
        <h2>{edits.length === 1 ? "A change to a picture, for you to decide" : `${edits.length} changes to pictures, for you to decide`}</h2>
      ) : (
        <h3>Sprite edits ({edits.filter(decidable).length} to decide)</h3>
      )}
      {said && <p className="hint">{said}</p>}
      {[...edits].reverse().map((e) => (
        <EditCard key={e.item} e={e} api={api} run={run} decide={(accept) => decide(e.item, accept)} watch={watch} plain={plain} />
      ))}
    </section>
  );
}
