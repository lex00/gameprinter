// The clips the tool makes by itself (docs/show.md, "Clips made by the
// tool"): the server finds starts across the game in the run's recordings
// and makes the scenario ladder's sequences at each, as background work.
// This is the page's client for it.

import type { Api } from "./api";
import type { GenState, Mark } from "./stripfold";

const enc = encodeURIComponent;

export class GenClient {
  constructor(private readonly api: Api) {}
  state(run: string): Promise<GenState> {
    return this.api.json(`/api/capture/${enc(run)}/generated`);
  }
  /** Makes the clips again from the start, in the background. */
  start(run: string): Promise<GenState> {
    return this.api.post(`/api/capture/${enc(run)}/generated/start`, {});
  }
  stop(run: string): Promise<GenState> {
    return this.api.post(`/api/capture/${enc(run)}/generated/stop`, {});
  }
  mark(run: string, clip: string, mark: Mark): Promise<GenState> {
    return this.api.post(`/api/capture/${enc(run)}/generated/clips/${enc(clip)}`, { mark });
  }
  /** The original's picture at a start. */
  picture(run: string, start: string): string {
    return this.api.url(`/api/capture/${enc(run)}/generated/starts/${enc(start)}/picture.png`);
  }
  /** The player's state bytes at a start. */
  startState(run: string, start: string): Promise<Uint8Array> {
    return this.api.bytes(`/api/capture/${enc(run)}/generated/starts/${enc(start)}/state`);
  }
}

/** Whether it is making clips now. */
export function generating(g: GenState | undefined): boolean {
  return !!g?.requested && !g?.finished;
}
