// REFINE's queue as pictures (game-printer's editor/src/Filmstrip.tsx, the
// half for the clips of the game's tests): every clip of the tests as a
// small picture of the original at its start, grouped by the mechanic it
// was recorded for, its border the colour of its state and its state in a
// word under its name. A click opens the clip on CLIPS. The pictures come
// from gp serve's clip pictures (generated/starts/CLIP/picture.png), made
// once a clip by replaying it, so they load as they scroll into view.

import { clipPicture, clipState, h, openClip } from "./fold.js";

/** The mechanic a clip of the tests was recorded for: dk.walk.004 is walk. */
export function groupOf(id) {
  const p = String(id).split(".");
  return p.length >= 3 ? p.slice(1, -1).join(".") : "other clips";
}

/** "52 clips. 35 play the same, 14 differ, 3 are waiting to be scored." */
export function picturesSays(s) {
  const ids = s?.active ?? [];
  if (!ids.length) return "No clips in the game's tests yet. Record some on CLIPS.";
  const n = { same: 0, differ: 0, waiting: 0, cant: 0 };
  for (const id of ids) {
    const st = clipState(s.scores?.[id]);
    if (st === "matches" || st === "fixed") n.same += 1;
    else if (st === "not matching") n.differ += 1;
    else if (st === "can't start") n.cant += 1;
    else n.waiting += 1;
  }
  const parts = [];
  if (n.same) parts.push(`${n.same} ${n.same === 1 ? "plays" : "play"} the same`);
  if (n.differ) parts.push(`${n.differ} ${n.differ === 1 ? "differs" : "differ"}`);
  if (n.cant) parts.push(`${n.cant} can't start`);
  if (n.waiting) parts.push(`${n.waiting} ${n.waiting === 1 ? "is" : "are"} waiting to be scored`);
  const list = parts.length > 1 ? `${parts.slice(0, -1).join(", ")} and ${parts[parts.length - 1]}` : parts[0];
  return `${ids.length === 1 ? "1 clip" : `${ids.length} clips`}. ${list[0].toUpperCase()}${list.slice(1)}. Click a picture to open the clip.`;
}

/** The pictures of the hub's clips into `el`; `pics` keeps each clip's
 * image between draws so a redraw doesn't load it again. */
export function drawPictures(el, x, pics) {
  const s = x.state;
  const says = h("p", "rf-muted", picturesSays(s));
  says.dataset.testid = "pictures-says";
  el.append(says);
  const groups = new Map();
  for (const id of s?.active ?? []) {
    const g = groupOf(id);
    if (!groups.has(g)) groups.set(g, []);
    groups.get(g).push(id);
  }
  const box = h("div", "rf-pictures");
  box.dataset.testid = "pictures";
  for (const [g, ids] of groups) {
    const sec = h("section", "rf-pic-group");
    sec.append(h("h4", "", `${g} (${ids.length})`));
    const row = h("div", "rf-pic-row");
    for (const id of ids) {
      const st = clipState(s.scores?.[id]);
      const b = h("button", `rf-pic-card ${st.replace(/[^a-z]/g, "-")}`);
      b.dataset.testid = "picture-card";
      b.dataset.clip = id;
      b.title = `${s.names?.[id] ?? id}: open this clip on the screen`;
      b.addEventListener("click", () => openClip(x.run, id));
      let pic = pics.get(id);
      if (!pic) { pic = clipPicture(x, id); pics.set(id, pic); }
      b.append(pic, h("span", "rf-name", id), h("span", "rf-state", st));
      row.append(b);
    }
    sec.append(row);
    box.append(sec);
  }
  el.append(box);
}
