// REVIEW REBUILD's side card, in the left margin's #rule-card: the clip
// picker, a small refinement panel and the sealed runs on request. The
// clips and their scores come from the scoring pipeline's status
// ($GP_HOME/work/refine/GAME/status.json, served at /api/rebuild/scores
// with every sealed row left out); with none yet, the open row's clips are
// listed unscored. Picking a clip plays it beside the rebuild a second
// before its first split (the page's own viewer, `show`). Sealed runs are
// listed only when the person asks (/api/sealed), and nothing about them
// is kept.
//
// game-printer kept this card in refine.js and read /api/refine. Both names
// belong to the refinement view now (gp model refine and gp rule, at
// /refine), so the card is here and its scores have their own route.

export class RefineView {
  constructor({ $, api, show, sessionClips, fps }) {
    this.$ = $;
    this.fps = fps; // () => the machine's frames a second
    this.api = api;
    this.show = show; // (clip) => plays it beside the rebuild
    this.sessionClips = sessionClips; // () => the open row's clips, unscored
    this.active = false;
    this.data = null;
    this.mtime = null;
    this.row = null; // the row to put first
    this.picked = null; // "mechanic/clip" or "sealed/id"
    this.sealed = null; // the sealed runs, once asked for
    this.status = "";
  }

  async enter(mechanic) {
    this.active = true;
    if (mechanic) this.row = mechanic;
    await this.refresh(true);
  }

  leave() {
    this.active = false;
    // Sealed runs are listed again only when asked again.
    this.sealed = null;
  }

  /** Reads the status again; redraws the card only when it changed. */
  async refresh(force = false) {
    let r;
    try {
      r = await this.api("api/rebuild/scores");
    } catch (e) {
      this.status = `the scores couldn't be read: ${e}`;
      return;
    }
    if (!force && r.mtime === this.mtime) return;
    this.mtime = r.mtime;
    // A second guard: nothing from a sealed row.
    const st = r.status;
    if (st) {
      st.rows = (st.rows ?? []).filter((x) => !String(x.mechanic).startsWith("sealed"));
      st.clips = (st.clips ?? []).filter((x) => !String(x.mechanic).startsWith("sealed"));
    }
    this.data = st;
    this.status = st ? `scores version ${st.version ?? "?"}${st.model?.commit ? ", model " + st.model.commit : ""}` : "";
    this.card();
  }

  card() {
    const box = this.$("rule-card");
    box.innerHTML = "";
    const add = (parent, tag, text, cls) => {
      const e = document.createElement(tag);
      if (text !== undefined) e.textContent = text;
      if (cls) e.className = cls;
      parent.appendChild(e);
      return e;
    };
    const pct = (s) => (s && s.pct !== null && s.pct !== undefined ? `${s.pct.toFixed(1)}%` : "not scored");
    const secs = (f) => `${(f / this.fps()).toFixed(1)} s`;
    const st = this.data;

    add(box, "h3", "Clips");
    add(box, "p", "Straight-through score and first split. Picking one opens it a second before its split.", "r-note");
    const ul = add(box, "ul", undefined, "r-pick");
    let clips = (st?.clips ?? []).slice();
    if (!clips.length) clips = (this.sessionClips() ?? []).map((c) => ({ ...c, unscored: true }));
    // The open row's clips first.
    clips.sort((a, b) => (a.mechanic === this.row ? 0 : 1) - (b.mechanic === this.row ? 0 : 1));
    for (const c of clips) {
      const li = add(ul, "li", undefined, this.picked === `${c.mechanic}/${c.clip}` ? "picked" : "");
      add(li, "span", `${c.mechanic} ${c.clip}`, "r-name");
      add(li, "span", c.unscored ? "not scored" : pct(c.straight), "r-pct");
      const sp = c.first_split;
      add(li, "span", sp ? `split ${secs(sp.clip_frame ?? sp.frame - (c.start ?? 0))}` : c.unscored ? "" : "no split", "r-split");
      if (sp?.what) li.title = sp.what;
      li.addEventListener("click", () => {
        this.picked = `${c.mechanic}/${c.clip}`;
        this.card();
        this.show(c);
      });
    }
    if (!clips.length) add(ul, "li", "No clips yet.", "r-note");

    // The refinement numbers, small.
    const d = add(box, "details", undefined, "r-small");
    add(d, "summary", st ? `Refinement (${this.status})` : "Refinement: no scores yet");
    if (st) {
      for (const r of st.rows ?? []) add(d, "div", `${r.mechanic}: straight ${pct(r.straight)}, resync ${pct(r.resync)}${r.longest_exact ? ", longest exact " + secs(r.longest_exact.frames) : ""}`);
      const f = (x) => (x === null || x === undefined ? "?" : `${x.toFixed(1)}%`);
      for (const m of (st.merges ?? []).slice(0, 5)) {
        add(d, "div", `${m.commit ?? ""} ${m.title ?? ""}: ${f(m.before?.straight_pct)} -> ${f(m.after?.straight_pct)} straight`, "r-note");
      }
    } else {
      add(d, "div", "The scoring pipeline writes $GP_HOME/work/refine/GAME/status.json; the scores show here when it does. Until then the open row's clips are listed, unscored, and each shows how many of its frames agree once it is opened.", "r-note");
    }

    // Sealed runs: private, listed only on request.
    const sd = add(box, "div", undefined, "r-sealed");
    if (!this.sealed) {
      const b = add(sd, "button", "OPEN A SEALED RUN (PRIVATE)", "r-sealed-open");
      b.title = "Lists the sealed runs for you alone; no worker sees them and nothing is kept";
      b.addEventListener("click", async () => {
        try {
          this.sealed = (await this.api("api/sealed")).runs ?? [];
        } catch (e) {
          this.sealed = [];
          this.status = `the sealed runs couldn't be read: ${e}`;
        }
        this.card();
      });
    } else {
      add(sd, "h3", "Sealed runs (private)");
      const su = add(sd, "ul", undefined, "r-pick");
      for (const r of this.sealed) {
        const li = add(su, "li", undefined, this.picked === `sealed/${r.id}` ? "picked" : "");
        add(li, "span", r.id, "r-name");
        add(li, "span", r.frames ? secs(r.frames) : "", "r-pct");
        if (r.note) li.title = r.note;
        li.addEventListener("click", () => {
          this.picked = `sealed/${r.id}`;
          this.card();
          this.show({ sealed: r.id, start: r.start ?? 0, frames: r.frames ?? 600 });
        });
      }
      if (!this.sealed.length) add(su, "li", "None.", "r-note");
      const hide = add(sd, "button", "HIDE THE SEALED RUNS", "r-sealed-open");
      hide.addEventListener("click", () => { this.sealed = null; this.card(); });
    }
  }

  /** The clip to open on entering the tab: the one last picked, else the
   * open row's first, else the first listed. False when there is none. */
  showFirst() {
    const st = this.data;
    let clips = (st?.clips ?? []).slice();
    if (!clips.length) clips = (this.sessionClips() ?? []).map((c) => ({ ...c, unscored: true }));
    if (!clips.length) return false;
    const c = clips.find((x) => `${x.mechanic}/${x.clip}` === this.picked) ?? clips.find((x) => x.mechanic === this.row) ?? clips[0];
    this.picked = `${c.mechanic}/${c.clip}`;
    this.card();
    this.show(c);
    return true;
  }

  statusText() {
    return "Reviewing the rebuild: pick a clip at the side. A plays and pauses, left and right step a frame, the triggers a second, [ and ] jump to the difference before or after, up and down set the transparency, Select changes the layout, C or the left bumper outlines its entities, B goes back to the split, R or the right bumper plays the rebuild.";
  }
}
