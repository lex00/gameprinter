// The rebuilt game's side of a side by side (docs/inspector.md, "The same
// commands to both"): the run's interpreter preview in control mode, in an
// iframe, driven by messages (scripts/web/page/preview.js). The game view
// and a scenario's side by side both use it: load the original's state at
// the start with the presses from there, then seek by steps from it.

/** The model side: the run's preview in control mode, in an iframe. */
export class ModelPreview {
  private ready = false;
  private queued: unknown[] = [];
  private snaps = new Map<number, (url: string) => void>();
  private next = 1;
  /** The physics profile shown (docs/model.md, "Profiles"): "retro" is the model as written. */
  private physics = "retro";
  /** What the preview said went wrong, such as a module too old to load a state. */
  onError?: (message: string) => void;
  constructor(private readonly frame: () => HTMLIFrameElement | null) {}
  private post(m: unknown) {
    const w = this.frame()?.contentWindow;
    if (!this.ready || !w) this.queued.push(m);
    else w.postMessage(m, location.origin);
  }
  /** The iframe is loading another page: wait for it to say ready again. */
  reset() {
    this.ready = false;
  }
  receive(e: MessageEvent) {
    if (e.source !== this.frame()?.contentWindow || e.origin !== location.origin) return;
    const m = e.data as { gp?: string; id?: number; url?: string };
    if (m?.gp === "ready") {
      this.ready = true;
      const q = this.queued;
      this.queued = [];
      q.forEach((x) => this.post(x));
      // A page loaded again starts in retro: it gets the chosen physics after its start.
      if (this.physics !== "retro") this.post({ gp: "profile", name: this.physics });
    } else if (m?.gp === "snap" && m.id !== undefined) {
      this.snaps.get(m.id)?.(m.url ?? "");
      this.snaps.delete(m.id);
    } else if (m?.gp === "error") {
      this.onError?.((m as { message?: string }).message ?? "the rebuilt game failed");
    }
  }
  /** The model started from `state` (the original's, translated), to play `text` from there. */
  loadState(state: string, text: string, frame: number) {
    // A new start replaces any seek still waiting for the old one.
    this.queued = this.queued.filter((m) => (m as { gp: string }).gp !== "seek");
    this.post({ gp: "load-state", state, text, frame });
  }
  /** More model frames played live (a frames file's lines), appended and shown: free play's rebuilt game. */
  push(text: string) {
    this.post({ gp: "push", text });
  }
  seek(frame: number) {
    if (!this.ready) this.queued = this.queued.filter((m) => (m as { gp: string }).gp !== "seek");
    this.post({ gp: "seek", frame });
  }
  /** The rebuilt game in another physics profile, from the same start at the same frame. Only this view changes. */
  profile(name: string) {
    if (name === this.physics) return;
    this.physics = name;
    // A page not ready yet gets it once it is (receive).
    if (this.ready) this.post({ gp: "profile", name });
  }
  snap(frame: number): Promise<string> {
    const id = this.next++;
    return new Promise((resolve) => {
      this.snaps.set(id, resolve);
      this.post({ gp: "snap", frame, id });
    });
  }
}

