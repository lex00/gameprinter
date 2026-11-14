// A game's events as they happen, for the views embedded in gp serve's one
// page (Embed.tsx, App.tsx): it takes the run's events as server-sent
// events (polling when the stream fails) and folds them (livefold.ts,
// playfold.ts); a person's answers, pins and notes go to the run as inputs.

import { useEffect, useState } from "react";
import { CAPTURE_FORMAT, type Api, type CaptureEvent, type CaptureInput, type CaptureRun, type EventSourceLike } from "./api";
import { emptyLive, fold, type LiveState } from "./livefold";
import { emptyPlay, foldPlay, type PlayState } from "./playfold";

/** How often the panel asks for new events when it polls. */
export const POLL_MS = 500;
/** The most events `GET /api/capture/{run}/events` returns at once. */
const PAGE = 2000;

/** How the panel gets events: pushed (server-sent events) or by asking. */
export type Transport = "stream" | "poll";

/** What a subscription tells its owner. */
export interface Sink<E> {
  events: (events: E[]) => void;
  transport: (t: Transport) => void;
  error: (message: string | undefined) => void;
}
export type LiveSink = Sink<CaptureEvent>;

/** Where a subscription gets events: a page after a seq, and a stream when there is one. */
export interface Feed<E> {
  page(after: number, signal: AbortSignal): Promise<E[]>;
  stream(after: number): EventSourceLike | undefined;
}

/**
 * Follows a feed of events numbered by `seq`: the stream first (the
 * browser resumes it with Last-Event-ID), and polling from the last event it
 * gave when the stream fails or the platform has none. Each event is passed
 * on once, in order. Returns the function that stops it.
 */
export function follow<E extends { seq: number }>(feed: Feed<E>, sink: Sink<E>, pollMs = POLL_MS): () => void {
  let live = true;
  let seq = 0;
  let timer: ReturnType<typeof setTimeout> | undefined;
  const abort = new AbortController();
  const take = (events: E[]) => {
    const fresh = events.filter((e) => e.seq > seq);
    if (!fresh.length) return;
    seq = fresh[fresh.length - 1].seq;
    sink.events(fresh);
  };
  const poll = async () => {
    let full = false;
    try {
      const events = await feed.page(seq, abort.signal);
      if (!live) return;
      full = events.length >= PAGE;
      take(events);
      sink.error(undefined);
    } catch (e) {
      if (live) sink.error((e as Error).message);
    }
    // A full page means more are waiting: ask again at once.
    if (live) timer = setTimeout(poll, full ? 0 : pollMs);
  };
  const source = feed.stream(0);
  if (source) {
    sink.transport("stream");
    source.onmessage = (m) => {
      if (!live) return;
      try {
        take([JSON.parse(m.data) as E]);
        sink.error(undefined);
      } catch {
        // Not an event; skip it.
      }
    };
    source.onerror = () => {
      source.close();
      if (!live) return;
      sink.transport("poll");
      void poll();
    };
  } else {
    sink.transport("poll");
    void poll();
  }
  return () => {
    live = false;
    abort.abort();
    source?.close();
    if (timer) clearTimeout(timer);
  };
}

/** Follows a capture run's events ([`follow`] on its routes). */
export function subscribe(api: Api, run: string, sink: LiveSink, pollMs = POLL_MS): () => void {
  return follow<CaptureEvent>(
    {
      page: async (after, signal) => (await api.captureEvents(run, after, signal)).events,
      stream: (after) => api.captureStream(run, after),
    },
    sink,
    pollMs,
  );
}

/** A run's events folded twice: as a capture run (livefold) and as a play session (playfold). */
export interface RunEvents {
  state: LiveState;
  play: PlayState;
  error?: string;
  transport?: Transport;
}

export function useLive(api: Api, run: string | undefined, pollMs = POLL_MS): RunEvents {
  const [state, setState] = useState<{ live: LiveState; play: PlayState }>(() => ({ live: emptyLive(), play: emptyPlay() }));
  const [error, setError] = useState<string>();
  const [transport, setTransport] = useState<Transport>();
  useEffect(() => {
    setState({ live: emptyLive(), play: emptyPlay() });
    setError(undefined);
    setTransport(undefined);
    if (!run) return;
    return subscribe(
      api,
      run,
      {
        events: (evs) => setState((s) => ({ live: fold(s.live, evs), play: foldPlay(s.play, evs) })),
        transport: setTransport,
        error: setError,
      },
      pollMs,
    );
  }, [api, run, pollMs]);
  return { state: state.live, play: state.play, error, transport };
}

/** An input from the person in the panel, in the current format. */
export function personInput(i: Omit<CaptureInput, "v" | "actor">): CaptureInput {
  return { v: CAPTURE_FORMAT, actor: "person", ...i };
}

/** Sends a person's input to a run and says what happens next. */
export function sendInput(api: Api, run: string | undefined, runs: CaptureRun[], state: LiveState, said: (s: string) => void) {
  return (input: CaptureInput) => {
    if (!run) return;
    api
      .captureInput(run, input)
      .then((r) =>
        said(
          runs.find((x) => x.run === run)?.watched || state.watch
            ? `input ${r.inputs} sent; the run's watcher applies it`
            : `input ${r.inputs} sent; the next round applies it (gp capture resume ${run}, or watch it: gp capture watch ${run})`,
        ),
      )
      .catch((e: Error) => said(e.message));
  };
}
