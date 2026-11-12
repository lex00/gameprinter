// The refinement view (/refine): the rule `gp rule jump` found
// ($GP_HOME/mechanics/GAME/jump/refine.json, at /api/rule) and the last
// `gp model refine --lockstep` run on the game (/api/refine): the clips
// checked against the ROM and the numbers it wrote. Read again every few
// seconds, so a run in a terminal shows here as it ends.

const $ = (id) => document.getElementById(id);

async function api(path) {
  const r = await fetch(path);
  if (!r.ok) throw new Error(`${path}: ${r.status}`);
  return r.json();
}

function add(parent, tag, text, cls) {
  const e = document.createElement(tag);
  if (text !== undefined) e.textContent = text;
  if (cls) e.className = cls;
  parent.appendChild(e);
  return e;
}

function row(tbody, cells, cls) {
  const tr = add(tbody, "tr", undefined, cls);
  for (const c of cells) add(tr, "td", String(c ?? ""));
}

function showRule(r) {
  const rule = r.rule;
  $("r-rule-status").textContent = r.status ?? (rule ? "" : "Nothing yet. gp rule jump GAME finds it from the clips.");
  const ul = $("r-rule");
  ul.innerHTML = "";
  if (!rule) return;
  $("r-rule-status").textContent = `${r.status ? r.status + ". " : ""}Fitted on ${rule.fitted_on}: matches ${rule.matched} of ${rule.total} frames in the air.`;
  for (const c of rule.rule ?? []) {
    const li = add(ul, "li");
    add(li, "span", c.source, `r-src r-${c.source}`);
    add(li, "span", `${c.name}: ${c.value} ${c.unit}`);
    li.title = c.from ?? "";
  }
  const t = $("r-clips");
  const body = t.querySelector("tbody");
  body.innerHTML = "";
  for (const c of rule.clips ?? []) {
    const m = (c.jumps ?? []).flatMap((j) => j.match ?? []);
    const ok = m.filter(Boolean).length;
    row(body, [`${c.session} ${c.clip}`, m.length, ok], ok < m.length ? "r-bad" : "");
  }
  t.hidden = !(rule.clips ?? []).length;
}

function showRun(r) {
  const run = r.run;
  if (!run) return;
  const e = run.exact ?? {};
  $("r-run-status").textContent = `${run.model}: ${e.before} of ${e.clips} clips exact before, ${e.after} after; ${run.written?.length ?? 0} numbers written, ${run.questions ?? 0} questions.`;
  const w = $("r-written");
  const wb = w.querySelector("tbody");
  wb.innerHTML = "";
  for (const x of run.written ?? []) row(wb, [x.param, x.old, x.new, x.clip]);
  w.hidden = !(run.written ?? []).length;
  const c = $("r-checks");
  const cb = c.querySelector("tbody");
  cb.innerHTML = "";
  for (const [id, s] of Object.entries(run.checks ?? {})) {
    if (s.status === "checked") continue;
    row(cb, [id, s.status === "unchecked" ? "no curve recorded" : s.status === "differs" ? "differs: not used" : `can't be played: ${s.why}`], s.status === "unchecked" ? "" : "r-bad");
  }
  c.hidden = !cb.children.length;
}

async function refresh() {
  try {
    showRule(await api("/api/rule?mechanic=jump"));
    showRun(await api("/api/refine"));
  } catch (e) {
    $("r-run-status").textContent = `couldn't read the refinement: ${e.message}`;
  }
}

void refresh();
setInterval(refresh, 3000);
