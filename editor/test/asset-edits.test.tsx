// The live panel's sprite edits (docs/sprite-editing.md): an edit card
// draws its previews, shots and checks, offers accept and reject only while
// the edit is open, and a decision is posted as a person's input.
import { describe, expect, it } from "vitest";
import { renderToString } from "react-dom/server";
import { Api, type AssetEdit } from "../src/api";
import { EditCard, decidable, editSentence } from "../src/AssetEdits";

const edit = (status: AssetEdit["data"]["status"]): AssetEdit => ({
  item: "edit:1",
  text: "edit 1 proposed: player's sprite (base), recolor",
  data: {
    status,
    entity: "player",
    slot: "tiles",
    frames: ["base"],
    ops: ["recolor"],
    prompt: "make his moss cap the color of his eye",
    critic: 0.944,
    checks: [{ name: "context.readability", verdict: "pass", message: "100% of the backgrounds read" }],
    previews: { edit: "/w/edit/previews/edit-1.png", context: "/w/edit/previews/edit-1-context.png" },
    shots: [{ row: 0, pose: "default/ground/still/right", baseline: "finish-level", frame: 1 }],
  },
});

describe("a sprite edit in the live panel", () => {
  it("draws its previews, shots and checks, with buttons while it's open", () => {
    const api = new Api("http://x");
    const html = renderToString(<EditCard e={edit("proposed")} api={api} run="live" decide={() => {}} />);
    expect(html).toContain("http://x/api/capture/live/edits/edit/1/edit");
    expect(html).toContain("http://x/api/capture/live/edits/edit/1/context");
    expect(html).toContain("default/ground/still/right");
    expect(html).toContain("context.readability pass");
    expect(html).toContain("accept (place it)");
    const done = renderToString(<EditCard e={edit("accepted")} api={api} run="live" decide={() => {}} />);
    expect(done).not.toContain("<button");
    expect(decidable(edit("placed"))).toBe(true);
    expect(decidable(edit("undone"))).toBe(false);
  });

  it("posts a decision as a person's input naming the item", async () => {
    const sent: { url: string; body: unknown }[] = [];
    const fetchFn = (async (url: string, init?: RequestInit) => {
      sent.push({ url, body: JSON.parse(String(init?.body)) });
      return new Response(JSON.stringify({ inputs: 2, decision: { status: "accepted" } }), { status: 200 });
    }) as typeof fetch;
    const api = new Api("", fetchFn);
    const r = await api.captureInput("live", { v: 1, actor: "person", kind: "accept", item: "edit:1" });
    expect(r.decision?.status).toBe("accepted");
    expect(sent[0].url).toBe("/api/capture/live/input");
    expect(sent[0].body).toEqual({ v: 1, actor: "person", kind: "accept", item: "edit:1" });
  });

  it("says an edit in plain words for the game view, with a watch button per moment", () => {
    expect(editSentence(edit("proposed"))).toBe(`The agent proposes a change to the player's picture: "make his moss cap the color of his eye"`);
    expect(editSentence(edit("accepted"))).toContain("You kept a change");
    const stone = edit("proposed");
    stone.data = { ...stone.data, entity: "", tile: "stone block", prompt: "make it orange" };
    expect(editSentence(stone)).toBe(`The agent proposes a change to the stone block's picture: "make it orange"`);
    const watched: [string, number][] = [];
    const html = renderToString(<EditCard e={edit("proposed")} api={new Api("")} run="live" decide={() => {}} plain watch={(b, f) => watched.push([b, f])} />);
    expect(html).toContain("The agent proposes a change");
    expect(html).not.toContain("edit:1");
    expect(html).toContain("watch");
  });
});
