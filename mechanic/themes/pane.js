// THEMES: the game's theme kit, its themes, making and fitting a theme,
// the readability and originality checks, theme clip review and theme
// refinement. Theme work comes once the game plays right with the
// original's pictures; each theme then gets its own clip review (a clip
// of the game drawn with the theme beside the same clip in the original
// art, frame for frame) and its own refinement (marks on that review,
// and rounds that fit and check the theme again on them).
//
// The server side is crates/gp-cli/src/theme_serve.rs (api/themes/...).
// Every command runs as background work there; none makes a paid call
// (generate and publish are dry runs).

const el = (tag, cls, text) => {
  const e = document.createElement(tag);
  if (cls) e.className = cls;
  if (text !== undefined) e.textContent = text;
  return e;
};
const btn = (text, title, on) => {
  const b = el("button", "", text);
  if (title) b.title = title;
  b.addEventListener("click", on);
  return b;
};
const pct = (v) => (v == null ? "" : `${(v * 100).toFixed(1)}%`);

/** Mounts THEMES in `body`. `page` is the shell's {api, toast, game}. */
export function mountThemes(body, page) {
  if (!document.querySelector('link[href$="themes/pane.css"]')) {
    const l = document.createElement("link");
    l.rel = "stylesheet";
    l.href = "themes/pane.css";
    document.head.append(l);
  }
  const st = { all: null, pick: null, job: null, review: null, at: 0, playing: null, marks: [] };
  let timer = null;

  const say = (t) => page.toast?.(String(t));
  async function run(what, extra = {}) {
    try {
      const r = await page.api("api/themes/run", { what, ...extra });
      st.job = r.job;
      say(`Started ${what} (job ${r.job}).`);
      await load();
    } catch (e) { say(e.message ?? e); }
  }

  async function load() {
    try { st.all = await page.api("api/themes/all"); } catch (e) {
      body.textContent = `THEMES can't read this game: ${e.message ?? e}`;
      return;
    }
    if (!st.pick && st.all.themes.length) st.pick = st.all.themes[0].key;
    if (st.marksFor !== st.pick) await loadMarks();
    draw();
    const busy = st.all.jobs.some((j) => !j.done);
    clearTimeout(timer);
    timer = setTimeout(() => void (busy || st.job ? refresh() : 0), busy ? 1500 : 5000);
  }
  async function refresh() {
    if (!body.isConnected || body.closest("[hidden]")) return;
    await load();
  }

  function draw() {
    const a = st.all;
    body.innerHTML = "";
    const top = el("p", "muted", `${a.title} (${a.model}). Theme work comes once the game plays right with the original's pictures. Everything here is written under ${a.work}; nothing goes into the games checkout, and nothing here pays for a call.`);
    body.append(top);
    const grid = el("div", "th-grid");
    const left = el("div", "th-col");
    const right = el("div", "th-col th-main");
    grid.append(left, right);
    body.append(grid);
    drawKit(left, a.kit);
    drawThemes(left, a);
    drawMake(left, a);
    drawRegistry(left, a.registry);
    drawJobs(left, a.jobs);
    drawTheme(right, a);
  }

  function drawKit(p, k) {
    const s = el("section", "th-box", "");
    s.id = "th-kit";
    s.append(el("h3", "", "The game's theme kit"));
    if (!k.file) {
      s.append(el("p", "muted", "No kit yet. Press WRITE THE KIT, or ask your agent to write it."));
    } else {
      s.append(el("p", "", `${k.art.length} art slots, ${k.sound.length} sound roles, from ${k.file}.`));
      if (k.current === false) s.append(el("p", "bad", "The kit's slot fingerprint isn't its model's: the model changed since. WRITE THE KIT writes it again here."));
      s.append(el("p", k.local ? "" : "muted", k.local ? `Its local part (tiles each frame is cut into, from your ROM) is in ${k.local}.` : "No local part yet: fitting art needs it."));
      const d = el("details");
      d.append(el("summary", "", "Art slots"));
      const t = el("table", "th-table");
      t.innerHTML = "<tr><th>slot</th><th>kind</th><th>role</th><th>size</th><th>frames</th><th>what</th></tr>";
      for (const x of k.art) {
        const r = t.insertRow();
        for (const v of [x.id, x.kind, x.role, `${x.size[0]}x${x.size[1]}`, x.frames, x.label || x.description]) r.insertCell().textContent = v;
        r.title = x.description;
      }
      d.append(t);
      s.append(d);
      const d2 = el("details");
      d2.append(el("summary", "", "Sound roles"));
      const t2 = el("table", "th-table");
      t2.innerHTML = "<tr><th>role</th><th>kind</th><th>frames</th><th>loops</th><th>cue</th></tr>";
      for (const x of k.sound) {
        const r = t2.insertRow();
        for (const v of [x.role, x.kind, x.frames ?? "", x.loops ? "yes" : "", x.cue]) r.insertCell().textContent = v;
      }
      d2.append(t2);
      s.append(d2);
    }
    s.append(btn("WRITE THE KIT", "Writes the game's theme kit from your copy of the game: what each picture is and its size. The parts from your copy stay on this computer", () => run("kit")));
    p.append(s);
  }

  function checkLine(r) {
    const c = r?.check;
    if (!c) return "not checked yet";
    const read = c.readability?.verdict ?? "?";
    const sc = c.originality?.scores ?? [];
    const refused = sc.filter((x) => x.refused).length;
    const geo = (c.geometry?.flags ?? []).filter((f) => f.verdict === "refuse").length;
    return `readability ${read}, originality ${refused ? `${refused} refused` : "passes"} of ${sc.length}, geometry ${geo ? `${geo} refused` : "passes"}`;
  }

  function drawThemes(p, a) {
    const s = el("section", "th-box");
    s.id = "th-list";
    s.append(el("h3", "", `Themes (${a.themes.length})`));
    if (!a.themes.length) s.append(el("p", "muted", "None yet: fit art below, or link a theme folder."));
    for (const t of a.themes) {
      const row = el("div", `th-theme${t.key === st.pick ? " on" : ""}`);
      const name = t.round_of ? `round ${t.round_of.n} of ${a.themes.find((x) => x.key === t.round_of.theme)?.title ?? t.round_of.theme}` : (t.title ?? t.id ?? "?");
      row.append(el("b", "", name), el("span", "muted", ` ${t.scope}${t.id ? `, id ${t.id}` : ""}`));
      row.append(el("div", "muted small", `${t.restyles ?? 0} restyles, ${t.sounds ?? 0} sounds, ${t.palettes ?? 0} palettes. ${checkLine(t.result)}${t.marks ? `. ${t.marks} marks` : ""}`));
      if (t.problem) row.append(el("div", "bad small", t.problem));
      row.addEventListener("click", async () => { st.pick = t.key; st.review = null; await loadMarks(); draw(); });
      s.append(row);
    }
    p.append(s);
  }

  function drawMake(p, a) {
    const s = el("section", "th-box");
    s.id = "th-make";
    s.append(el("h3", "", "Make a theme"));
    const sel = el("select");
    for (const c of a.styles) { const o = el("option", "", `${c.name} (${c.scope})`); o.value = c.name; sel.append(o); }
    s.append(el("p", "muted", "A style card says how a theme looks, apart from any game. SHEETS writes the starting grids and prompts for an image model. GENERATE is a dry run: it lists the calls and their cost and makes none."));
    const r1 = el("p");
    r1.append(el("span", "", "style "), sel, document.createTextNode(" "),
      btn("SHEETS", "Makes the starting sheets for a new theme in the style chosen", () => run("sheet", { style: sel.value })), document.createTextNode(" "),
      btn("GENERATE (DRY RUN)", "Says what drawing the pictures would cost. Nothing is drawn and nothing is paid", () => run("generate", { style: sel.value, sheets: src.value || undefined })));
    s.append(r1);
    s.append(el("p", "muted", "Fit art made already: a folder of filled sheets (sheets.json beside them) or of art by slot id. The fitted theme is checked when it's written."));
    const src = el("input");
    src.placeholder = "the sheets or art folder";
    src.size = 46;
    const kind = el("select");
    for (const k of ["sheets", "art"]) { const o = el("option", "", k); o.value = k; kind.append(o); }
    const nm = el("input");
    nm.placeholder = "name";
    nm.size = 10;
    nm.value = "fit";
    const r2 = el("p");
    r2.append(kind, document.createTextNode(" "), src, document.createTextNode(" "), nm, document.createTextNode(" "),
      btn("FIT", "Fits the finished pictures to the kit, then checks them", () => run("fit", { [kind.value]: src.value, name: nm.value })));
    s.append(r2);
    const dir = el("input");
    dir.placeholder = "a theme folder (holding theme.toml)";
    dir.size = 46;
    const r3 = el("p");
    r3.append(dir, document.createTextNode(" "), btn("LINK", "List a theme folder made elsewhere; with the folder above as its art, a round can fit it again", async () => {
      try {
        await page.api("api/themes/link", { dir: dir.value, ...(src.value ? { [kind.value]: src.value } : {}) });
        await load();
      } catch (e) { say(e.message ?? e); }
    }));
    s.append(r3);
    p.append(s);
  }

  function drawRegistry(p, r) {
    const s = el("section", "th-box");
    s.append(el("h3", "", "In the registry"));
    if (!r.themes.length) s.append(el("p", "muted", `No theme for this game in ${r.catalog ?? "the registry"}.`));
    for (const t of r.themes) s.append(el("p", "", `${t.title ?? t.name}: ${t.name} ${t.latest ?? ""}`));
    if (r.themes.length) s.append(el("p", "muted small", "These are content packages: a look the page's player draws (gp_present::art), installed with the game. The theme commands here check and review theme folders (theme.toml)."));
    p.append(s);
  }

  function drawJobs(p, jobs) {
    const s = el("section", "th-box");
    s.id = "th-jobs";
    s.append(el("h3", "", "Background work"));
    if (!jobs.length) s.append(el("p", "muted", "Nothing run yet."));
    for (const j of jobs.slice(0, 12)) {
      const ex = j.exits ?? [];
      const state = !j.done ? "running" : ex.every((x) => x === 0) ? "done" : `exit ${ex.filter((x) => x !== 0).join(", ")}`;
      const r = el("div", `th-job${j.n === st.job ? " on" : ""}`);
      r.append(el("b", "", `${j.n} ${j.what}`), el("span", j.done ? "muted" : "run", ` ${state}`));
      r.addEventListener("click", () => { st.job = j.n; draw(); });
      s.append(r);
    }
    if (st.job) {
      const pre = el("pre", "th-log", "...");
      pre.id = "th-log";
      s.append(pre);
      page.api(`api/themes/jobs/${st.job}`).then((r) => {
        pre.textContent = `${r.job.commands.join("\n")}\n\n${r.log || "(nothing printed yet)"}`;
        pre.scrollTop = pre.scrollHeight;
      }).catch((e) => { pre.textContent = String(e.message ?? e); });
    }
    p.append(s);
  }

  // ---- One theme: its checks, its clip review, its marks and rounds ----

  function drawTheme(p, a) {
    const t = a.themes.find((x) => x.key === st.pick);
    if (!t) { p.append(el("p", "muted", "Pick a theme.")); return; }
    const s = el("section", "th-box");
    s.id = "th-theme";
    s.append(el("h3", "", t.title ?? t.id ?? "theme"));
    s.append(el("p", "muted small", t.dir));
    const bar = el("p");
    bar.append(
      btn("CHECK", "Checks the theme: that it reads well, keeps to its style, keeps every size and hit box, and holds nothing from the original", () => run("check", { theme: t.key })), document.createTextNode(" "),
      btn("PUBLISH (DRY RUN)", "Runs every check and says what would be published. Nothing is published", () => run("publish", { theme: t.key })), document.createTextNode(" "),
      btn("ROUND", "A refinement round on this theme's marks: fit again when its art is known, then check", () => round(t)),
    );
    s.append(bar);
    drawChecks(s, t.result);
    drawPublish(s, t.result?.publish);
    p.append(s);
    const rv = el("section", "th-box");
    rv.id = "th-review";
    drawReview(rv, t, a);
    p.append(rv);
    const rf = el("section", "th-box");
    rf.id = "th-refine";
    drawRefine(rf, t);
    p.append(rf);
  }

  function drawChecks(s, r) {
    const c = r?.check;
    s.append(el("h4", "", "Checks"));
    if (!c) { s.append(el("p", "muted", "Not checked yet: CHECK runs them in the background.")); return; }
    if (c.error) { s.append(el("p", "bad", c.error)); return; }
    const rd = c.readability;
    if (rd) {
      s.append(el("p", rd.verdict === "refuse" ? "bad" : "", `Readability: ${rd.verdict}`));
      const t = el("table", "th-table");
      t.innerHTML = "<tr><th>group</th><th>distinct</th><th>contrast</th><th>verdict</th></tr>";
      for (const g of rd.groups ?? []) {
        const row = t.insertRow();
        for (const v of [g.group, pct(g.distinct), g.contrast?.toFixed(2), g.verdict]) row.insertCell().textContent = v ?? "";
        if (g.verdict === "refuse") row.className = "bad";
        if (g.why) row.title = g.why;
      }
      s.append(t);
    }
    const o = c.originality;
    if (o) {
      const sc = o.scores ?? [];
      const refused = sc.filter((x) => x.refused);
      s.append(el("p", refused.length ? "bad" : "", `Originality: ${refused.length ? `${refused.length} refused` : "passes"} (${sc.length} scores; art refused at ${o.art_refuse}, shape at ${o.shape_refuse}, melody at ${o.melody_refuse}; ${(o.rom_files ?? []).length} files with the ROM's bytes)`));
      const t = el("table", "th-table");
      t.innerHTML = "<tr><th>what</th><th>closeness</th><th>shape</th><th></th></tr>";
      const near = [...sc].sort((x, y) => (y.refused - x.refused) || (y.value - x.value)).slice(0, 14);
      for (const x of near) {
        const row = t.insertRow();
        for (const v of [x.what, x.value?.toFixed(3), x.shape?.toFixed(3) ?? "", x.refused ? "REFUSED" : x.judged === false ? "not judged" : ""]) row.insertCell().textContent = v ?? "";
        if (x.refused) row.className = "bad";
      }
      s.append(t);
    }
    const g = c.geometry;
    if (g) {
      const bad = (g.flags ?? []).filter((f) => f.verdict === "refuse");
      s.append(el("p", bad.length ? "bad" : "", `Geometry: ${g.checked} slots and stages, ${(g.flags ?? []).length} flags, ${bad.length} refused`));
      for (const f of bad.slice(0, 8)) s.append(el("p", "bad small", `${f.slot} ${f.at}: ${f.what}`));
    }
    for (const l of c.slots ?? []) {
      const refused = (l.findings ?? []).filter((f) => f.refused);
      if (refused.length) s.append(el("p", "bad small", `slots of ${l.theme}: ${refused.map((f) => f.message).join("; ")}`));
    }
  }

  function drawPublish(s, text) {
    if (!text) return;
    s.append(el("h4", "", "Publish, dry run (nothing was published)"));
    const lines = text.trim().split("\n");
    const last = lines.at(-1) ?? "";
    s.append(el("p", /refused|not checked/.test(last) ? "bad" : "", last));
    const d = el("details");
    d.append(el("summary", "", "All it printed"));
    d.append(el("pre", "th-log", text));
    s.append(d);
  }

  // Theme clip review: the clip drawn with the theme beside the same clip
  // in the original art, from one run, so both show the same frame.
  function drawReview(s, t, a) {
    s.append(el("h4", "", "Theme clip review"));
    const sel = el("select");
    sel.id = "th-clip";
    for (const c of a.clips) { const o = el("option", "", `${c.name} (${c.frames} frames)`); o.value = c.name; sel.append(o); }
    if (st.review?.clip) sel.value = st.review.clip;
    const go = btn("REVIEW", "Draw the clip with this theme and with the original art", async () => {
      go.disabled = true;
      go.textContent = "DRAWING...";
      try {
        st.review = await page.api("api/themes/review", { theme: t.key, clip: sel.value });
        st.at = 0;
        draw();
      } catch (e) { say(e.message ?? e); go.disabled = false; go.textContent = "REVIEW"; }
    });
    const head = el("p");
    head.append(sel, document.createTextNode(" "), go);
    s.append(head);
    const r = st.review;
    if (!r || r.theme !== t.key) {
      s.append(el("p", "muted", "Pick a clip of the game and REVIEW: it plays on the game's model, drawn once with the original art and once with this theme."));
      return;
    }
    s.append(el("p", "muted small", `${r.clip}: ${r.length} frames after a start of ${r.start}, every ${r.every === 1 ? "frame" : `${r.every}th frame`} drawn; state digest ${String(r.digest).slice(0, 16)}. Readability on this theme: ${r.readability?.verdict ?? "?"}.${(r.not_run ?? []).length ? ` Hooks not run (no module): ${r.not_run.join(", ")}.` : ""} Click the theme's side to mark what's wrong there.`));
    const pair = el("div", "th-pair");
    const mk = (label, side) => {
      const f = el("figure");
      const wrap = el("div", "th-shot");
      const img = el("img");
      img.alt = label;
      img.dataset.side = side;
      wrap.append(img);
      f.append(wrap, el("figcaption", "", label));
      return { f, img, wrap };
    };
    const o = mk("ORIGINAL ART", "o");
    const th = mk("THEME", "t");
    pair.append(o.f, th.f);
    s.append(pair);
    const ctl = el("div", "th-ctl");
    const scrub = el("input");
    scrub.type = "range";
    scrub.min = 0;
    scrub.max = r.frames.length - 1;
    scrub.value = st.at;
    const lab = el("span", "", "");
    const show = () => {
      const n = r.frames[st.at];
      // Both sides change together, once both have loaded.
      const a1 = new Image(), a2 = new Image();
      let k = 0;
      const both = () => {
        if (++k < 2) return;
        o.img.src = a1.src;
        th.img.src = a2.src;
        lab.textContent = `frame ${n} of ${r.length}`;
        drawMarks(th.wrap, r.clip, n);
      };
      a1.onload = a2.onload = both;
      a1.onerror = a2.onerror = both;
      a1.src = `${r.base}/o/${n}.png`;
      a2.src = `${r.base}/t/${n}.png`;
      scrub.value = st.at;
    };
    const step = (d) => { st.at = Math.max(0, Math.min(r.frames.length - 1, st.at + d)); show(); };
    scrub.addEventListener("input", () => { st.at = Number(scrub.value); show(); });
    const play = btn("PLAY", "Play both sides together", () => {
      if (st.playing) { clearInterval(st.playing); st.playing = null; play.textContent = "PLAY"; return; }
      play.textContent = "PAUSE";
      st.playing = setInterval(() => {
        if (!s.isConnected || st.at >= r.frames.length - 1) { clearInterval(st.playing); st.playing = null; play.textContent = "PLAY"; return; }
        step(1);
      }, Math.max(16, (1000 / 60) * r.every));
    });
    ctl.append(btn("<", "A frame back", () => step(-1)), play, btn(">", "A frame on", () => step(1)), scrub, lab);
    s.append(ctl);
    const note = el("input");
    note.id = "th-note";
    note.placeholder = "what's wrong at the point you click (optional)";
    note.size = 50;
    const slot = el("select");
    slot.id = "th-slot";
    slot.title = "The slot the mark is about; left as it is, the slot of what's drawn where you click";
    slot.append(el("option", "", "slot: what's under the click"));
    slot.options[0].value = "";
    for (const x of a.kit.art ?? []) { const o = el("option", "", x.id); o.value = x.id; slot.append(o); }
    const mrow = el("p");
    mrow.append(note, document.createTextNode(" "), slot);
    s.append(mrow);
    th.img.addEventListener("click", async (e) => {
      const rect = th.img.getBoundingClientRect();
      const x = Math.round(((e.clientX - rect.left) / rect.width) * th.img.naturalWidth);
      const y = Math.round(((e.clientY - rect.top) / rect.height) * th.img.naturalHeight);
      try {
        const res = await page.api("api/themes/marks", { theme: t.key, clip: r.clip, frame: r.frames[st.at], x, y, note: note.value, slot: slot.value || undefined });
        st.marks = res.marks;
        note.value = "";
        say(`Marked frame ${r.frames[st.at]}${res.marks.at(-1)?.slot ? ` on ${res.marks.at(-1).slot}` : ""}.`);
        drawMarks(th.wrap, r.clip, r.frames[st.at]);
        await load();
      } catch (err) { say(err.message ?? err); }
    });
    show();
  }

  function drawMarks(wrap, clip, frame) {
    for (const d of wrap.querySelectorAll(".th-mark")) d.remove();
    const img = wrap.querySelector("img");
    const W = img?.naturalWidth || 256, H = img?.naturalHeight || 240;
    for (const m of st.marks) {
      if (m.clip !== clip || Math.abs(m.frame - frame) > 2 || m.x < 0) continue;
      const d = el("div", "th-mark");
      d.style.left = `${(m.x / W) * 100}%`;
      d.style.top = `${(m.y / H) * 100}%`;
      d.title = `${m.slot ?? ""} ${m.note}`;
      wrap.append(d);
    }
  }

  async function loadMarks() {
    if (!st.pick) return;
    st.marksFor = st.pick;
    try { st.marks = (await page.api(`api/themes/marks?theme=${st.pick}`)).marks; } catch (e) { st.marks = []; }
  }

  function drawRefine(s, t) {
    s.append(el("h4", "", "Refinement"));
    if (!st.marks.length) s.append(el("p", "muted", "No marks yet. Mark what's wrong on the theme's side of a clip review; ROUND then fits the theme again (when its art is known) and checks it, and says what became of each mark."));
    const t1 = el("table", "th-table");
    if (st.marks.length) t1.innerHTML = "<tr><th>#</th><th>clip</th><th>frame</th><th>slot</th><th>note</th><th></th></tr>";
    for (const m of st.marks) {
      const r = t1.insertRow();
      for (const v of [m.id, m.clip, m.frame, m.slot ?? "", m.note]) r.insertCell().textContent = v ?? "";
      const c = r.insertCell();
      c.append(btn("X", "Take this mark off", async () => {
        st.marks = (await page.api("api/themes/marks", { theme: t.key, delete: m.id })).marks;
        await load();
      }));
    }
    s.append(t1);
    for (const r of (t.rounds ?? []).slice().reverse()) {
      const d = el("div", "th-round");
      if (r.running) { d.append(el("p", "run", `Round ${r.n}: running.`)); s.append(d); continue; }
      d.append(el("p", "", `Round ${r.n}: ${(r.did ?? []).join("; ")}. Readability ${r.readability ?? "?"}, originality ${r.originality_refused ? `${r.originality_refused} refused` : "passes"}, geometry ${r.geometry_refused ? `${r.geometry_refused} refused` : "passes"}.`));
      const tt = el("table", "th-table");
      for (const m of r.marks ?? []) {
        const row = tt.insertRow();
        for (const v of [`mark ${m.mark}`, m.slot, m.note, m.state, m.originality == null ? "" : Number(m.originality).toFixed(3)]) row.insertCell().textContent = v ?? "";
        if (/still/.test(m.state)) row.className = "bad";
      }
      d.append(tt);
      s.append(d);
    }
  }

  async function round(t) {
    try {
      const r = await page.api("api/themes/round", { theme: t.key });
      st.job = r.job;
      say(`Round ${r.round}: ${r.did.join("; ")}.`);
      await load();
    } catch (e) { say(e.message ?? e); }
  }

  return {
    async load() { await loadMarks(); await load(); },
  };
}
