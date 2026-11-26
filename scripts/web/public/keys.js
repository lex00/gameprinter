// The public page's key legend and ALL KEYS list (docs/web.md, "The page's
// look"), as plain data: which rows the legend has for a game's layout and
// how a key is written on a key cap. Nothing here knows a game or its keys:
// the keys are the layout's own (page/controls.js LAYOUTS, read from the
// shell's table), handed in, so a pad game and an arcade game each get
// theirs. page.js draws what these return; keys.test.mjs checks them on
// both layouts.

// A pad's buttons by bit, for a layout that names none (controls.js: A, B,
// Select, Start, up, down, left, right).
const PAD = ["a", "b", "select", "start", "up", "down", "left", "right"];

// A key cap for a key: `text` on the cap, or `arrow` (up, right, down,
// left) for an arrow key, and `say`, the key in words for a screen reader.
// A key is a KeyboardEvent.code, or one already shortened ("X", "5").
const NAMED = {
  ArrowUp: { arrow: "up", say: "Up arrow" },
  ArrowRight: { arrow: "right", say: "Right arrow" },
  ArrowDown: { arrow: "down", say: "Down arrow" },
  ArrowLeft: { arrow: "left", say: "Left arrow" },
  ShiftLeft: { text: "L Shift", say: "Left Shift" },
  ShiftRight: { text: "R Shift", say: "Right Shift" },
  Escape: { text: "Esc", say: "Escape" },
  RB: { text: "RB", say: "right shoulder button" },
  LB: { text: "LB", say: "left shoulder button" },
};
export function capOf(key) {
  const k = String(key);
  if (NAMED[k]) return { text: "", arrow: null, ...NAMED[k] };
  const text = k.replace(/^Digit/, "").replace(/^Key/, "");
  return { text, arrow: null, say: text };
}

// The caps of a key as the player's list writes it: one key, a key with
// Shift ("Shift+1-4" is two caps) or a few keys ("X, Space").
export function capsOf(keys) {
  return String(keys)
    .split(", ")
    .map((k) => k.split("+").map(capOf));
}

// The legend's rows for a layout, in the order a player needs them: the
// coin, Start and Select, the arrows, then the buttons. Each row:
// `id` (the layout's name of the button, `move` for the arrows), `label`,
// `color` (a tube colour of arcade.css), `keys` (the keys shown: the ones
// the game takes from the shell when it has such, as a cabinet's 5 and 1,
// else the first two; for the arrows, left, up, down, right), `press` (the
// key a click on the row presses; null for the arrows, which no click
// presses) and `say` (the row in words).
const ROWS = [
  ["coin", "Coin", "yellow"],
  ["start", "Start", "green"],
  ["select", "Select", "violet"],
  ["move", "Move", "cyan"],
  ["button", "Action", "pink"],
  ["a", "A", "pink"],
  ["b", "B", "yellow"],
];
export function legendOf(layout) {
  const names = layout?.names ?? PAD;
  const keysOf = (n) => layout?.keys?.[names.indexOf(n)] ?? [];
  const rows = [];
  for (const [id, label, color] of ROWS) {
    if (id === "move") {
      const keys = ["left", "up", "down", "right"].map((d) => keysOf(d)[0]).filter(Boolean);
      if (keys.length) rows.push({ id, label, color, keys, press: null, say: `${label}, ${keys.map((k) => capOf(k).say).join(", ")}` });
      continue;
    }
    const all = keysOf(id);
    if (!all.length) continue;
    const own = all.filter((k) => layout.taken?.has(k));
    const keys = (own.length ? own : all).slice(0, 2);
    rows.push({ id, label, color, keys, press: keys[0], say: `${label}, key ${keys.map((k) => capOf(k).say).join(" or ")}` });
  }
  return rows;
}

// The game's own keys as a group of the ALL KEYS list, every key of each
// button: what the page shows before a game plays (the player's own list,
// with the saves and the rest, replaces it once one does).
export function playGroup(layout) {
  const names = layout?.names ?? PAD;
  const title = (n) => (n === "button" ? "Action" : n.charAt(0).toUpperCase() + n.slice(1));
  return { name: "Play", rows: names.map((n, i) => ({ name: title(n), keys: (layout?.keys?.[i] ?? []).join(", "), how: "" })).filter((r) => r.keys) };
}
