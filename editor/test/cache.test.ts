import { describe, expect, it } from "vitest";
import { Api } from "../src/api";
import { FrameCache, MAX_STATES, type Recording } from "../src/pane";

const recording = { id: "abcd", manifest: { frames: 2000 } } as unknown as Recording;

function fakeApi(seen: string[]) {
  return new Api("", (input, init) => {
    const url = String(input);
    seen.push(url);
    const signal = (init as RequestInit | undefined)?.signal;
    if (signal?.aborted) return Promise.reject(new DOMException("aborted", "AbortError"));
    const frame = Number(url.split("/frames/")[1]);
    const body = url.includes("/events")
      ? { id: "abcd", frames: [1, 64], from: "event stream", events: [] }
      : { id: "abcd", frame, from: "trace", fields: { ram: "00" } };
    return Promise.resolve(new Response(JSON.stringify(body)));
  });
}

describe("frame cache", () => {
  it("fetches each frame once and keeps the newest", async () => {
    const seen: string[] = [];
    const cache = new FrameCache(fakeApi(seen), recording);
    await cache.pane(10);
    await cache.pane(11);
    expect(seen.filter((u) => u.endsWith("/frames/10"))).toHaveLength(1);
    for (let f = 0; f < MAX_STATES + 50; f++) await cache.state(f);
    expect(cache.size).toBe(MAX_STATES);
  });

  it("forgets an aborted fetch, so it's tried again", async () => {
    const seen: string[] = [];
    const cache = new FrameCache(fakeApi(seen), recording);
    const abort = new AbortController();
    abort.abort();
    await expect(cache.pane(20, abort.signal)).rejects.toThrow();
    await cache.pane(20);
    expect(seen.filter((u) => u.endsWith("/frames/20"))).toHaveLength(2);
  });
});
