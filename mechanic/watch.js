// The watch panel on a page of its own (gp mechanic watch): what the agent is
// doing, from /api/watch once a second, and a box for the person's notes,
// which reach the agent as turns (gp mechanic show and wait). Redrawn only
// when something changed, so a playing clip isn't restarted every poll.

const $ = (id) => document.getElementById(id);

function el(tag, cls, text) {
  const e = document.createElement(tag);
  if (cls) e.className = cls;
  if (text !== undefined) e.textContent = text;
  return e;
}

function timed(line) {
  const li = el("li");
  li.append(el("span", "at", line.at ?? ""), document.createTextNode(line.text ?? ""));
  return li;
}

let lastKey = "";

async function refresh() {
  const r = await fetch("/api/watch");
  if (!r.ok) throw new Error(`${r.status} ${await r.text()}`);
  const w = await r.json();
  const key = JSON.stringify(w);
  if (key === lastKey) return;
  lastKey = key;
  $("title").textContent = `${w.game} / ${w.topic}`;
  document.title = `${w.topic}: watch`;
  const status = $("w-status");
  status.innerHTML = "";
  if (w.status) {
    status.className = "";
    status.append(el("span", "at", w.status.at ?? ""), document.createTextNode(w.status.text ?? ""));
  } else {
    status.className = "muted";
    status.textContent = "Nothing yet. Your agent writes here with gp mechanic log, pin and problem.";
  }
  const pins = $("w-pins");
  pins.innerHTML = "";
  for (const p of w.pins ?? []) {
    const fig = el("figure", p.keep ? "keep" : "");
    const src = `/api/watch/files/${encodeURIComponent(p.file)}`;
    let media;
    if (p.kind === "video") {
      media = el("video");
      Object.assign(media, { src, muted: true, loop: true, autoplay: true, controls: true, playsInline: true });
    } else {
      media = el("img");
      Object.assign(media, { src, alt: p.title || p.file, loading: "lazy" });
    }
    fig.append(media, el("figcaption", "", `${p.n}. ${p.title || p.file}`));
    pins.append(fig);
  }
  const table = $("w-problems");
  const body = table.querySelector("tbody");
  body.innerHTML = "";
  for (const row of w.problems ?? []) {
    const tr = el("tr");
    for (const [v, cls] of [[row.n], [row.where], [row.what], [row.fix], [row.state, `st-${row.state}`]]) tr.append(el("td", cls ?? "", String(v ?? "")));
    body.append(tr);
  }
  table.hidden = !(w.problems ?? []).length;
  const talk = $("w-talk");
  talk.innerHTML = "";
  for (const t of [...(w.talk ?? [])].reverse()) talk.append(el("li", t.by === "agent" ? "by-agent" : "", `${t.by === "player" ? "you" : t.by}: ${t.text}`));
  const log = $("w-log");
  log.innerHTML = "";
  for (const line of [...(w.log ?? [])].reverse()) log.append(timed(line));
}

$("w-say").addEventListener("submit", async (e) => {
  e.preventDefault();
  const text = $("w-text").value.trim();
  if (!text) return;
  const r = await fetch("/api/talk", { method: "POST", headers: { "Content-Type": "application/json" }, body: JSON.stringify({ text }) });
  if (r.ok) $("w-text").value = "";
  void refresh();
});

void refresh().catch(() => undefined);
setInterval(() => void refresh().catch(() => undefined), 1000);
