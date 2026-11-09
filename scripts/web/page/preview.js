// The interpreter preview: the model running in the browser in gp_web.wasm
// (crates/gp-web), drawn on a 2D canvas from the pictures `gp gen bevy`
// draws. It needs no Bevy build. See docs/web.md.
//
// Query parameters: `run=NAME` replays runs/NAME.frames instead of reading
// the keyboard, with `from=N` running its first N frames at once and
// `to=M` stopping at frame M (the play panel's replay of a mark,
// docs/show.md); `test=1` first runs every input in runs/index.json
// through the module and compares its hashes with the native
// interpreter's; `profile=NAME` runs the model in one of its profiles
// (docs/model.md, "Profiles"): a preset such as `normal`, a switch, or
// switches and presets joined by `+` such as `normal+no_timer`, applied to
// the bundle by the module from `profiles.toml`, instead of the model as
// written, `retro`; `control=1` hands the frame to the page that embeds this
// one (the workspace's game view, docs/inspector.md): it has no clock of
// its own and answers messages from its parent, of its own origin only:
//   {gp: "frames", text, frame}  the input to play, as a frames file
//   {gp: "load-state", state, text, frame}
//                                the input to play from a model state (the
//                                canonical state's bytes in base64): every
//                                seek back starts again from that state
//   {gp: "seek", frame}          step to that model frame and draw it
//   {gp: "push", text}           more input played live (a frames file):
//                                appended, then stepped to its end and drawn
//   {gp: "snap", frame, id}      a picture of that frame, then back
//   {gp: "profile", name}        the model in that physics profile from
//                                the same start, stepped to the same frame
//                                (`retro` is the model as written); it
//                                answers {gp: "profile", name, frame}
// It posts {gp: "ready"} once loaded, {gp: "at", frame, of} after each
// seek and {gp: "snap", id, url} with each picture.
//
// Played from the keyboard or a gamepad (no `run` or `control`), the page
// has the runtime shell (docs/web.md, "The shell"; docs/shell.md): the
// settings screen's Saves line, Shift+1 to 4 to save to a slot and 1 to 4
// to load it, the autosave at each level start, held Backspace or left
// trigger to rewind through a ring of states, and the bumpers for quick
// save and load once turned on; Console only turns saves and rewind off.
// F8 steps the scale (1x to 4x, then fit), F9 draw-only widescreen (off,
// 16:10, 16:9), F10 flash dimming, and 5 to 9 mute a sound channel
// (gp_present::view; none of them changes a step). F2 pauses, F3 plays one
// frame while paused and F4 steps slow motion (1/2, 1/4), pacing only.
// `level=NAME` starts at that level (preview.json lists them). A slot is a
// save record naming the build that made it (gp_sim::save, the generated
// game's format), kept per game in IndexedDB (storage.js); a save of
// another build is refused. The options are kept per game in localStorage.
//
// Esc, F1 or the controller's right stick press (standard button 11) opens the
// standard settings screen (docs/settings.md) over the game: the mode
// (retro, normal or another preset) and every switch of profiles.toml, drawn
// by the module (gp_present::settings, as the generated game draws it). The
// game holds while it is open. A change starts the game again from power-on
// in the new settings, which are kept in localStorage per model
// (`gp-settings:MODEL`); `profile=` in the URL wins over them. In control
// mode the parent opens it and hands it keys with {gp: "settings", key}
// (open, up, down, left, right, choose, back), and the page answers
// {gp: "settings", open} and, on closing, {gp: "settings", open: false,
// profile, restart}: the parent keeps the setting and starts the rebuild
// again in it. Played from the keyboard, the page that embeds the preview
// (the public page's Settings button) can send {gp: "settings", key:
// "open"} too, with the same effect as pressing Esc.
//
// Timings and results go to window.__gp for scripts/web/measure.mjs.

import { withLevel } from "./level.js";
import { AUTOSAVE_SLOT, BINDINGS, NES, NES2, SCREEN, QUICK_SLOTS, SHELL_SETTINGS, exportName, slotName, whenName } from "./shell-keys.js";
import { openStorage } from "./storage.js";
import { ModuleHost } from "./module-host.js";

const t = (window.__gp = { marks: {}, hashes: [], results: null, error: null });
const mark = (n) => {
  if (!(n in t.marks)) t.marks[n] = performance.now();
};
mark("script");
addEventListener("error", (e) => {
  t.error ??= String(e.message);
});

const text = new TextDecoder();
const enc = new TextEncoder();
const hex = (bytes) => Array.from(bytes, (b) => b.toString(16).padStart(2, "0")).join("");

class Preview {
  // `host` is the module in its Worker (module-host.js).
  constructor(host) {
    this.host = host;
    this.last = new Uint8Array(0);
  }
  has(name) {
    return this.host.has(name);
  }
  // Calls an export; what it left in its output buffer is `this.last`.
  run(name, ...args) {
    const { r, out } = this.host.call(name, args);
    this.last = out;
    return r;
  }
  out() {
    return text.decode(this.last);
  }
  check(code) {
    if (code !== 0) throw new Error(this.out());
    return this.out();
  }
  // Returns the hooks' label (crate, version, code hash), empty for a model
  // without hooks. Throws when the model names a hook the module lacks.
  load(bundle) {
    return this.check(this.run("gp_load", bundle));
  }
  // The bundle in a profile (any set of switches), from the text of
  // profiles.toml; null from a gp_web.wasm built before switches.
  profileBundle(bundle, profiles, name) {
    if (!this.has("gp_profile_bundle")) return null;
    const [t, n] = [enc.encode(profiles), enc.encode(name)];
    const code = this.run("gp_profile_bundle", bundle, t, n);
    if (code !== 0) throw new Error(this.out());
    return this.last;
  }
  step(buttons, reset) {
    this.check(this.run("gp_step", buttons, reset ? 1 : 0));
  }
  hash() {
    return hex(this.host.call("gp_hash", [], 3).out);
  }
  state() {
    return JSON.parse(this.check(this.run("gp_draw_state")));
  }
  // The canonical model state's bytes, which loadState puts back; null
  // from a gp_web.wasm built before gp_state.
  stateBytes() {
    if (!this.has("gp_state")) return null;
    if (this.run("gp_state") !== 0) throw new Error(this.out());
    return this.last;
  }
  // The game as a save record (gp_sim::save), the bytes the generated
  // game's save slots hold too; null from a gp_web.wasm built before it.
  saveRecord(profile) {
    if (!this.has("gp_save_record")) return null;
    const n = enc.encode(profile);
    if (this.run("gp_save_record", n) !== 0) throw new Error(this.out());
    return this.last;
  }
  // Loads a save record: {loaded: true}, {refused: sentence} for a save of
  // another build or game, or {profile} when it was saved in another
  // profile (nothing loaded).
  loadRecord(bytes, profile) {
    const n = enc.encode(profile);
    const code = this.run("gp_load_record", bytes, n);
    if (code === 0) return { loaded: true };
    if (code === 2) return { profile: this.out() };
    return { refused: this.out() };
  }
  // The rewind ring, the shell standard's (gp_present::shell::Ring), held
  // by the module: false from a gp_web.wasm built before it.
  ringReset(num, den) {
    if (!this.has("gp_ring_reset")) return false;
    this.check(this.run("gp_ring_reset", num, den));
    return true;
  }
  ringClear() {
    this.check(this.run("gp_ring_clear"));
  }
  ringNote() {
    this.check(this.run("gp_ring_note"));
  }
  // One step of held rewind: true when a state was put back.
  ringBack() {
    const code = this.run("gp_ring_back");
    if (code === 1) throw new Error(this.out());
    return code === 0;
  }
  loadState(bytes) {
    if (!this.has("gp_load_state")) throw new Error("this preview's gp_web.wasm can't load a state; rebuild the run's preview");
    this.check(this.run("gp_load_state", bytes));
  }
  loadPresent(text) {
    const b = enc.encode(text);
    this.check(this.run("gp_load_present", b));
  }
  scene() {
    return JSON.parse(this.check(this.run("gp_scene")));
  }
  // Picture and sound options (gp_present::view), as play's --picture
  // takes them. False for a module built before them.
  view(list) {
    if (!this.has("gp_view")) return false;
    const b = enc.encode(list);
    this.check(this.run("gp_view", b));
    return true;
  }
  specDigest() {
    return this.check(this.run("gp_spec_digest"));
  }
  loadSound(text) {
    const b = enc.encode(text);
    this.check(this.run("gp_load_sound", b));
  }
  // The last frame's samples as floats, from the wasm synthesizer.
  audio() {
    const n = this.run("gp_audio");
    if (n <= 0) return null;
    const v = new Int16Array(this.last.buffer, this.last.byteOffset, n);
    const f = new Float32Array(n);
    for (let i = 0; i < n; i++) f[i] = v[i] / 32768;
    return f;
  }
  // The standard settings screen (gp_present::settings); false from a
  // module built before it.
  hasSettings() {
    return this.has("gp_settings_open");
  }
  settingsOpen(profiles, name) {
    const [t, n] = [enc.encode(profiles), enc.encode(name)];
    this.check(this.run("gp_settings_open", t, n));
  }
  // The open screen's Theme line: [id, title] pairs and the one shown. A
  // published theme's pair goes on with its author, state ("downloaded"
  // or "download") and size.
  settingsThemes(list, current) {
    if (!this.has("gp_settings_themes") || list.length < 2) return;
    const [l, c] = [enc.encode(list.map((x) => x.join("\t")).join("\n")), enc.encode(current)];
    this.check(this.run("gp_settings_themes", l, c));
  }
  // A key's number (SETTINGS_KEY): 0 nothing, 1 redraw, on a close
  // {closed, restart, profile, theme} (the look chosen comes on a second
  // line when the screen listed looks), or {slot: [what, n]} when the
  // player asked for something done with a save slot (save, load, delete,
  // download, or file to load one from a file).
  settingsKey(code) {
    return this.settingsOutcome(this.run("gp_settings_key", code));
  }
  settingsOutcome(r) {
    if (r === 4) {
      const [what, n] = this.out().split(" ");
      return { slot: [what, n === undefined ? null : Number(n)] };
    }
    if (r < 2) return r;
    const [profile, theme] = this.out().split("\n");
    return { closed: true, restart: r === 3, profile, theme: theme ?? null };
  }
  // Whether the module has the Saves line and the shell's settings on the
  // screen (2026-11-05).
  hasSaves() {
    return this.has("gp_settings_saves");
  }
  // The open screen's Console only and Quick save buttons lines, from the
  // settings' file text.
  settingsShell(text) {
    const b = enc.encode(text);
    this.check(this.run("gp_settings_shell", b));
  }
  // The shell's settings the last screen closed with, as file text, or null.
  settingsShellChosen() {
    return this.run("gp_settings_shell_chosen") === 0 ? this.out() : null;
  }
  // The open screen's Saves line: [slot, bytes or null, seconds or null]
  // for each slot, the profile played, the clock and the frame rate.
  settingsSaves(slots, profile, now, rate, files) {
    for (const [n, bytes, when] of slots) {
      const b = bytes ?? new Uint8Array(0);
      this.run("gp_settings_slot", n, b, when ?? -1);
    }
    const pr = enc.encode(profile);
    this.check(this.run("gp_settings_saves", pr, now, rate[0], rate[1], files ? 1 : 0));
  }
  // The open screen as text: a line each, the cursor's marked "> ", and
  // what is shown under the list last, after "| ". For the checks.
  settingsLines() {
    return this.has("gp_settings_lines") && this.run("gp_settings_lines") === 0 ? this.out().split("\n") : null;
  }
  settingsSay(text) {
    const b = enc.encode(text);
    this.check(this.run("gp_settings_say", b));
  }
  // Closes the screen after a load from it: as settingsKey's close.
  settingsCloseLoaded() {
    return this.settingsOutcome(this.run("gp_settings_close_loaded"));
  }
  // Whether the last frame started a level: the shell autosaves then.
  autosaveDue() {
    return this.has("gp_autosave_due") && this.host.call("gp_autosave_due", [], 0).r === 1;
  }
  // A theme's sound for a role, from a 16-bit WAV file's bytes
  // (gp_present::sound::RolePlayer); false from a module built before it.
  soundRole(name, wav) {
    if (!this.has("gp_sound_role")) return false;
    const n = enc.encode(name);
    this.check(this.run("gp_sound_role", n, wav));
    return true;
  }
  clearSoundRoles() {
    if (this.has("gp_sound_roles_clear")) this.run("gp_sound_roles_clear");
  }
  // The themes removed from the cache on the settings screen since the
  // last call, by id.
  settingsRemoved() {
    if (!this.has("gp_settings_removed")) return [];
    return this.run("gp_settings_removed") > 0 ? this.out().split("\n") : [];
  }
  // Whether the module reads packages (docs/theme-publishing.md).
  hasCatalogue() {
    return this.has("gp_catalogue");
  }
  blake3(bytes) {
    this.check(this.run("gp_blake3", bytes));
    return this.out();
  }
  // The themes of an index this build can draw.
  catalogue(index, pictures) {
    const [a, b] = [enc.encode(index), enc.encode(pictures.join("\n"))];
    const out = this.check(this.run("gp_catalogue", a, b));
    return out
      .split("\n")
      .filter((l) => l)
      .map((l) => {
        const [id, title, author, pkg, size, preview] = l.split("\t");
        return { id, title, author, package: pkg, size: Number(size), preview };
      });
  }
  // A package's look files from its manifest, checked against its id.
  packageLook(id, manifest) {
    const i = enc.encode(id);
    const out = this.check(this.run("gp_package_look", i, manifest));
    return out.split("\n").map((l) => {
      const [hash, size, path] = l.split("\t");
      return { hash, size: Number(size), path };
    });
  }
  settingsPicture(w, h) {
    this.check(this.run("gp_settings_draw", w, h));
    return new ImageData(new Uint8ClampedArray(this.last.buffer, this.last.byteOffset, w * h * 4), w, h);
  }
  runFrames(frames) {
    const b = enc.encode(frames);
    return this.check(this.run("gp_run_frames", b));
  }
}

function parseFrames(s) {
  const lines = s.trim().split("\n");
  if (lines.shift() !== "gp-frames 1") throw new Error("not a frames file");
  // Controller 1's bits, then controller 2's when it holds any (in the
  // high byte, as gp_step takes them), then `r` on a reset.
  return lines.map((l) => {
    const [b, ...rest] = l.split(/\s+/);
    let bits = parseInt(b, 16);
    for (const w of rest) if (w && w !== "r") bits |= (parseInt(w, 16) & 0xff) << 8;
    return [bits, rest.includes("r")];
  });
}

// Where the game's files come from: with `files=idb:GAME` in the URL, the
// files a ROM lift stored in this browser (scripts/web/public/lift.js);
// else the installed game's play files (install.js). The page's own files,
// gp_web.wasm among them, are always fetched.
const stored = (() => {
  const m = /^idb:(.+)$/.exec(new URLSearchParams(location.search).get("files") ?? "");
  if (m) {
    return import("./lift.js").then(async ({ loadStored }) => {
      const files = await loadStored(m[1]);
      if (!files) throw new Error(`no lifted ${m[1]} is stored in this browser; load the ROM again`);
      return files;
    });
  }
  return import("./install.js").then(async ({ playFiles }) => {
    const files = await playFiles();
    if (!files) throw new Error("this game isn't installed in this browser; open the page to install it");
    return files;
  });
})();
// A `blob:` URL (a downloaded theme's file, from the cache) is fetched in
// either case.
async function load(url, init) {
  if (url.startsWith("blob:")) return fetch(url, init);
  const b = (await stored).get(url.split("?")[0]);
  return b ? new Response(b) : new Response(null, { status: 404 });
}

// A picture whose bytes are at hand (stored by the lift, or a downloaded
// theme's file, `blobs`) is decoded from them with createImageBitmap, not
// loaded from a `blob:` URL: WebKit loads each such URL through its
// network process, and a lifted ROM's hundreds of pictures (565 for
// Original Edition) asked for at once left some unanswered and the page
// held for 30 s and more (2026-11-07). Where createImageBitmap is missing,
// a URL is used.
const blobs = new Map();
async function image(src) {
  let blob = blobs.get(src);
  if (!blob && !src.startsWith("blob:")) {
    const b = (await stored).get(src);
    if (!b) throw new Error(`${src}: not stored`);
    blob = new Blob([b], { type: "image/png" });
  }
  if (blob && typeof createImageBitmap === "function") return createImageBitmap(blob);
  const url = blob && !src.startsWith("blob:") ? URL.createObjectURL(blob) : src;
  const img = new Image();
  img.src = url;
  return img
    .decode()
    .then(() => img)
    .finally(() => url !== src && URL.revokeObjectURL(url));
}

// The runtime shell's keys and buttons, from the standard's table that
// every target reads (gp_present::shell::keys, docs/shell.md), as
// shell-keys.js has it: the ones this preview has.
const mine = (a) => BINDINGS.filter((b) => b.action === a && b.on !== "bevy");
const webKeys = (a) => mine(a).filter((b) => !b.shift).flatMap((b) => b.keys);
const webButtons = (a) => mine(a).flatMap((b) => b.standard);
// The NES buttons from the keyboard: A, B, Select, Start, Up, Down, Left,
// Right from bit 0, as the Bevy build maps them. Controller 2: M and N for
// A and B, H Select, Y Start, I J K L the d-pad.
const KEYS = NES.map(webKeys);
const KEYS2 = NES2.map(webKeys);
const PAD = NES.map(webButtons);
// Each shell key's action by its code, with Shift held or not.
const SHELL_KEY = new Map(
  BINDINGS.filter((b) => b.on !== "bevy" && b.when === "play").flatMap((b) => b.keys.map((k) => [`${b.shift ? "Shift+" : ""}${k}`, b.action])),
);
const shellAction = (code, shift) => SHELL_KEY.get(`${shift ? "Shift+" : ""}${code}`);
const down = new Set();
let resetPressed = false;
// Keys pressed since the shell last looked, for the slot and scale keys.
const pressed = [];
addEventListener("keydown", (e) => {
  down.add(e.code);
  if (shellAction(e.code, false) === "reset") resetPressed = true;
  if (!e.repeat) pressed.push({ code: e.code, shift: e.shiftKey });
  e.preventDefault();
});
addEventListener("keyup", (e) => down.delete(e.code));
// A gamepad with the standard layout adds to the keyboard's buttons:
// A and B, Select, Start, then the d-pad or the left stick.
// The first gamepad is controller 1, a second one controller 2 (the high
// byte), as on the console.
const padBits = () => {
  let bits = 0;
  let n = 0;
  for (const g of navigator.getGamepads?.() ?? []) {
    if (!g || !g.connected) continue;
    const shift = n++ === 0 ? 0 : 8;
    const on = (i) => !!g.buttons[i]?.pressed;
    const [ax, ay] = [g.axes[0] ?? 0, g.axes[1] ?? 0];
    const stick = [false, false, false, false, ay < -0.5, ay > 0.5, ax < -0.5, ax > 0.5];
    const set = PAD.map((bs, bit) => bs.some(on) || stick[bit]);
    set.forEach((v, bit) => {
      if (v) bits |= 1 << (bit + shift);
    });
  }
  return bits;
};
// The settings screen's keys (gp_web::settings_key's numbers): up, down,
// left, right, choose, back, and the settings key itself (Esc and F1 open
// it when it is closed; Esc, being "back", closes it when it is open),
// from the keyboard and from a gamepad's standard layout.
const SETTINGS_KEY = Object.fromEntries(SCREEN.flatMap((a, n) => webKeys(a).map((k) => [k, n])));
const SETTINGS_NAMES = ["up", "down", "left", "right", "choose", "back", "open"];
const SETTINGS_PAD = SCREEN.flatMap((a, n) => webButtons(a).map((b) => [b, n]));

// The settings screen over the game: opened, keyed and drawn by the
// module, which holds its state. Presentation only: it never steps or
// reads the model; a closing with new settings hands back their name.
function settingsScreen(p, canvas, ctx, profilesText, looks) {
  let open = false;
  const off = document.createElement("canvas");
  const was = new Set();
  return {
    get open() {
      return open;
    },
    show(profile) {
      p.settingsOpen(profilesText, profile);
      if (looks) p.settingsThemes(looks.choices(), looks.current);
      // The shell's lines and the Saves line, from the page's shell.
      this.fill?.();
      open = true;
    },
    key(code) {
      const r = p.settingsKey(code);
      if (r?.closed) open = false;
      return r;
    },
    // Closed after a load from the Saves line.
    closeLoaded() {
      const r = p.settingsCloseLoaded();
      if (r?.closed) open = false;
      return r;
    },
    // The screen's keys pressed on any gamepad since the last look.
    padKeys() {
      const out = [];
      for (const g of navigator.getGamepads?.() ?? []) {
        if (!g || !g.connected) continue;
        for (const [b, code] of SETTINGS_PAD) {
          const id = `${g.index}:${b}`;
          const on = !!g.buttons[b]?.pressed;
          if (on && !was.has(id)) out.push(code);
          if (on) was.add(id);
          else was.delete(id);
        }
      }
      return out;
    },
    // Over what the game drew, at the canvas's size: the game's own
    // resolution, wider with widescreen.
    looks,
    draw() {
      if (!open) return;
      const [w, h] = [canvas.width, canvas.height];
      off.width = w;
      off.height = h;
      off.getContext("2d").putImageData(p.settingsPicture(w, h), 0, 0);
      ctx.drawImage(off, 0, 0);
    },
  };
}
const settingsStore = (model, kind = "settings") => {
  const key = `gp-${kind}:${model}`;
  return {
    get() {
      try {
        return window.localStorage.getItem(key);
      } catch {
        return null;
      }
    },
    set(v) {
      try {
        window.localStorage.setItem(key, v);
      } catch {
        // Storage blocked: the setting lasts this visit.
      }
    },
  };
};

const held = (keys) => keys.reduce((acc, ks, bit) => (ks.some((k) => down.has(k)) ? acc | (1 << bit) : acc), 0);
const buttons = () => held(KEYS) | (held(KEYS2) << 8) | padBits();

// A theme's particles (docs/formats/theme.md): a pure function of the
// frame, the same formulas as crates/gp-theme/src/particles.rs.
function unit(seed, i, salt) {
  let x = (Math.imul(seed, 0x9e3779b9) + Math.imul(i, 0x85ebca6b) + Math.imul(salt, 0xc2b2ae35)) >>> 0;
  x = (x ^ (x >>> 16)) >>> 0;
  x = Math.imul(x, 0x7feb352d) >>> 0;
  x = (x ^ (x >>> 15)) >>> 0;
  x = Math.imul(x, 0x846ca68b) >>> 0;
  x = (x ^ (x >>> 16)) >>> 0;
  return (x >>> 8) / 16777216;
}

const rem = (a, b) => ((a % b) + b) % b;
const TURN = Math.PI * 2;

function dots(e, frame, view, bodies) {
  const t = frame % 1000000;
  const out = [];
  const boxes = e.role ? bodies : [[0, 0, view[0], view[1]]];
  boxes.forEach(([bx, by, bw, bh], b) => {
    for (let i = 0; i < e.count; i++) {
      const k = i + b * 1000;
      const [u, v, w] = [unit(e.seed, k, 1), unit(e.seed, k, 2), unit(e.seed, k, 3)];
      if (e.motion === "fireflies") {
        const r = e.role ? 12 : 0;
        out.push({
          x: bx - r + rem(u * (bw + 2 * r) + 6 * Math.sin(t * 0.02 + w * TURN), bw + 2 * r),
          y: by - r + rem(v * (bh + 2 * r) + 4 * Math.cos(t * 0.017 + u * TURN), bh + 2 * r),
          w: 1, h: 1, a: 0.5 + 0.5 * Math.sin(t * 0.05 + w * TURN),
        });
      } else if (e.motion === "snow") {
        out.push({ x: bx + rem(u * bw + 3 * Math.sin(t * 0.03 + w * TURN), bw), y: by + rem(v * bh + t * (0.3 + 0.4 * w), bh), w: 1, h: 1, a: 0.9 });
      } else if (e.motion === "rain") {
        out.push({ x: bx + rem(u * bw - t, bw), y: by + rem(v * bh + t * (3 + 2 * w), bh), w: 1, h: 4, a: 0.6 });
      } else {
        const age = rem(t + w * 30, 30);
        out.push({ x: bx + u * bw + (v - 0.5) * age * 0.6, y: by + bh - age * (0.8 + v) + 0.02 * age * age, w: 1, h: 1, a: 1 - age / 30 });
      }
    }
  });
  return out;
}

// The bundle in a profile: the model as written for `retro`; otherwise
// the module applies the switches to it from profiles.toml, or, from an
// older module, a preset's own bundle in profiles/.
const fetchBytes = (url, what) =>
  load(url, { cache: "no-store" }).then((r) => {
    if (!r.ok) throw new Error(`the rebuilt game has no ${what}`);
    return r.arrayBuffer().then((b) => new Uint8Array(b));
  });
let startLevel = null;
const fetchBundle = async (profile, p) => {
  const base = withLevel(await fetchBytes("bundle.json", "bundle"), startLevel);
  if (!profile || profile === "retro") return base;
  const text = await load("profiles.toml", { cache: "no-store" }).then((r) => (r.ok ? r.text() : null));
  const made = text === null ? null : p.profileBundle(base, text, profile);
  return made ?? fetchBytes(`profiles/${encodeURIComponent(profile)}.json`, `${profile} profile`);
};

// The cache of downloaded themes (docs/theme-publishing.md, "In the
// game"): Cache Storage where the page has it, else IndexedDB. Keys are
// `index/<game>`, `pkg/<package id>`, `blob/<hash>` and `theme/<id>` (the
// package id a theme was downloaded from).
function themeCache() {
  const base = `${location.origin}/__gp-themes/`;
  if (typeof caches !== "undefined") {
    const c = caches.open("gp-themes");
    return {
      kind: "Cache Storage",
      async get(k) {
        const r = await (await c).match(base + k);
        return r ? new Uint8Array(await r.arrayBuffer()) : null;
      },
      async put(k, b) {
        await (await c).put(base + k, new Response(b));
      },
      async del(k) {
        await (await c).delete(base + k);
      },
      async keys() {
        return (await (await c).keys()).map((r) => r.url.slice(base.length));
      },
    };
  }
  const db = new Promise((ok, no) => {
    const r = indexedDB.open("gp-themes", 1);
    r.onupgradeneeded = () => r.result.createObjectStore("files");
    r.onsuccess = () => ok(r.result);
    r.onerror = () => no(r.error);
  });
  const go = async (mode, f) => {
    const tx = (await db).transaction("files", mode);
    const req = f(tx.objectStore("files"));
    return new Promise((ok, no) => {
      tx.oncomplete = () => ok(req.result);
      tx.onerror = () => no(tx.error);
    });
  };
  return {
    kind: "IndexedDB",
    async get(k) {
      const v = await go("readonly", (s) => s.get(k));
      return v ? new Uint8Array(v) : null;
    },
    put: (k, b) => go("readwrite", (s) => s.put(b, k)),
    del: (k) => go("readwrite", (s) => s.delete(k)),
    keys: () => go("readonly", (s) => s.getAllKeys()),
  };
}

// The themes published for this game: its index from the first mirror
// that answers (preview.json's `catalogue`, or `mirror=URL` in the URL),
// else the copy cached the last time, and the cache of the looks
// downloaded. Every file is checked against its hash before it is kept
// and again before it is drawn. Presentation only: the model never reads
// a theme.
async function themeCatalogue(cfg, p, q) {
  const game = cfg.catalogue?.game;
  const mirrors = q.getAll("mirror").length ? q.getAll("mirror") : cfg.catalogue?.mirrors ?? [];
  const out = { entries: [], source: "none", mirrors, downloaded: new Set(), cache: null };
  if (!game || !p.hasCatalogue() || q.has("test") || q.has("run") || q.has("control")) return out;
  let cache;
  try {
    cache = themeCache();
    await cache.keys();
  } catch {
    return out;
  }
  out.cache = cache;
  let index = null;
  for (const m of mirrors) {
    try {
      const r = await fetch(`${m.replace(/\/$/, "")}/index/themes/${game}.toml`, { cache: "no-store" });
      if (!r.ok) continue;
      index = await r.text();
      await cache.put(`index/${game}`, enc.encode(index));
      out.source = "mirror";
      out.mirror = m;
      break;
    } catch {
      // Offline, or the mirror is down: the next, then the cached copy.
    }
  }
  if (index === null) {
    const b = await cache.get(`index/${game}`);
    if (b) {
      index = text.decode(b);
      out.source = "cache";
    }
  }
  if (index === null) return out;
  try {
    out.entries = p.catalogue(index, cfg.pictures);
  } catch (e) {
    console.warn(`the themes index of ${game} doesn't read: ${e.message}`);
    return out;
  }
  for (const e of out.entries) {
    const kept = await cache.get(`theme/${e.id}`);
    if (kept && text.decode(kept) === e.package && (await cache.get(`pkg/${e.package}`))) out.downloaded.add(e.id);
  }
  // A key from the first mirror that has it.
  const fetchKey = async (k) => {
    for (const m of mirrors) {
      try {
        const r = await fetch(`${m.replace(/\/$/, "")}/${k}`);
        if (r.ok) return new Uint8Array(await r.arrayBuffer());
      } catch {
        // The next mirror.
      }
    }
    throw new Error(`${k} is on no mirror (offline?)`);
  };
  // Downloads a theme's look into the cache: the manifest checked against
  // the package id, each file against its hash.
  out.download = async (e) => {
    const manifest = await fetchKey(`packages/${e.package}`);
    const files = p.packageLook(e.package, manifest);
    for (const f of files) {
      if (await cache.get(`blob/${f.hash}`)) continue;
      const b = await fetchKey(`blobs/${f.hash}`);
      if (b.length !== f.size || p.blake3(b) !== f.hash) throw new Error(`${f.path}: hash mismatch, not kept`);
      await cache.put(`blob/${f.hash}`, b);
    }
    await cache.put(`pkg/${e.package}`, manifest);
    await cache.put(`theme/${e.id}`, enc.encode(e.package));
    out.downloaded.add(e.id);
  };
  // A downloaded theme as a look, its files from the cache, each checked
  // again before use.
  out.look = async (e) => {
    const manifest = await cache.get(`pkg/${e.package}`);
    if (!manifest) throw new Error(`${e.id} isn't downloaded`);
    const files = p.packageLook(e.package, manifest);
    const urls = new Map();
    let lookText = "";
    for (const f of files) {
      const b = await cache.get(`blob/${f.hash}`);
      if (!b || p.blake3(b) !== f.hash) throw new Error(`${f.path}: missing or changed in the cache`);
      if (f.path === "look.txt") lookText = text.decode(b);
      const type = f.path.endsWith(".png") ? "image/png" : f.path.endsWith(".wav") ? "audio/wav" : "text/plain";
      const blob = new Blob([b], { type });
      const url = URL.createObjectURL(blob);
      // image() decodes a picture from its bytes, not its URL.
      if (type === "image/png") blobs.set(url, blob);
      urls.set(f.path, url);
    }
    const look = parseLook(e.id, lookText);
    look.title = e.title;
    look.src = (f) => urls.get(f);
    look.published = true;
    return look;
  };
  // Takes a theme out of the cache: its record, its manifest, and each of
  // its files no other downloaded theme holds.
  out.remove = async (id) => {
    const e = out.entries.find((x) => x.id === id);
    if (!e) return;
    const manifest = await cache.get(`pkg/${e.package}`);
    const keep = new Set();
    for (const o of out.entries) {
      if (o.id === id || !out.downloaded.has(o.id)) continue;
      const m = await cache.get(`pkg/${o.package}`);
      if (m) for (const f of p.packageLook(o.package, m)) keep.add(f.hash);
    }
    if (manifest) for (const f of p.packageLook(e.package, manifest)) if (!keep.has(f.hash)) await cache.del(`blob/${f.hash}`);
    await cache.del(`pkg/${e.package}`);
    await cache.del(`theme/${id}`);
    out.downloaded.delete(id);
  };
  return out;
}

// A look's manifest (gp_present::looks).
function parseLook(id, text) {
  const look = { id, title: id, original: false, own_sound: false, files: [] };
  for (const line of text.split("\n").slice(1)) {
    const [k, ...v] = line.trim().split(" ");
    if (k === "title") look.title = v.join(" ");
    else if (k === "original") look.original = true;
    else if (k === "own_sound") look.own_sound = true;
    else if (k === "file") look.files.push(v.join(" "));
  }
  return look;
}

// The looks a build offers: the one drawn into assets/ (preview.json's
// `look`), and each one themes/index.json lists, with its look.txt read.
// `original` is listed only where the assets were lifted.
async function findLooks(cfg, catalogue) {
  const own = cfg.look ?? ["original", "Original"];
  const found = [];
  if (cfg.looks) {
    const index = await load("themes/index.json").then((r) => (r.ok ? r.json() : { themes: [] })).catch(() => ({ themes: [] }));
    for (const { id } of index.themes ?? []) {
      const text = await load(`themes/${id}/look.txt`).then((r) => (r.ok ? r.text() : null)).catch(() => null);
      if (!text || !text.startsWith("gp-look 1")) continue;
      const look = parseLook(id, text);
      look.src = (f) => `themes/${id}/${f}`;
      found.push(look);
    }
  }
  return {
    own,
    found,
    current: own[0],
    catalogue,
    // [id, title], then each published theme not built in as [id, title,
    // author, "downloaded" or "download", size].
    choices() {
      const out = [own];
      for (const l of found) {
        if (l.published) continue;
        if ((l.original || l.id === "original") && !cfg.lifted) continue;
        if (!out.some(([id]) => id === l.id)) out.push([l.id, l.title]);
      }
      for (const e of catalogue?.entries ?? []) {
        if (out.some(([id]) => id === e.id)) continue;
        out.push([e.id, e.title, e.author, catalogue.downloaded.has(e.id) ? "downloaded" : "download", String(e.size)]);
      }
      return out;
    },
  };
}

// The game's own module when it ships one (a game with hooks has to); the
// page's own interpreter, which has no game's hooks, for a game without.
let moduleSource = "page";
async function moduleBytes() {
  let bytes = (await stored).get("module.wasm");
  if (!bytes && /^idb:/.test(new URLSearchParams(location.search).get("files") ?? "")) {
    // A lifted ROM's files are the lift's; the module is the installed game's.
    const { playFiles } = await import("./install.js");
    bytes = (await playFiles())?.get("module.wasm");
  }
  if (bytes) {
    moduleSource = "game";
    return bytes;
  }
  return fetch("gp_web.wasm").then((r) => r.arrayBuffer());
}

function showError(message) {
  const p = document.createElement("p");
  p.id = "error";
  p.setAttribute("role", "alert");
  p.textContent = message;
  p.style.cssText = "color: #fdd; background: #511; font: 16px/1.4 system-ui, sans-serif; max-width: 720px; margin: 16px auto; padding: 12px 16px; white-space: pre-wrap;";
  document.body.prepend(p);
}

const openHost = (bytes) =>
  ModuleHost.open(bytes, (message) => {
    t.error ??= message;
    t.stopped = message;
    showError(message);
  });

async function main() {
  const q = new URLSearchParams(location.search);
  const [gameModule, cfg, presentText, profilesText] = await Promise.all([
    moduleBytes(),
    load("preview.json").then((r) => r.json()),
    load("assets/present.txt").then((r) => r.text()),
    load("profiles.toml", { cache: "no-store" }).then((r) => (r.ok ? r.text() : 'format = "0.2.0"\n')).catch(() => 'format = "0.2.0"\n'),
  ]);
  // The setting kept for keyboard play; scripted runs, tests and control
  // mode play what the URL says, retro without it.
  const store = settingsStore(cfg.model ?? cfg.title);
  const kept = q.has("profile") || q.has("test") || q.has("run") || q.has("control") ? null : store.get();
  let profile = q.get("profile") ?? kept ?? "retro";
  mark("instantiated");
  const wanted = q.get("level");
  if (wanted !== null && !q.has("test")) {
    // A level the model hasn't is refused with the ones it has, not played as the first.
    if (cfg.levels && !cfg.levels.includes(wanted)) throw new Error(`no level \`${wanted}\`; this model has ${cfg.levels.join(", ")}`);
    startLevel = wanted;
  }
  const host = await openHost(gameModule);
  host.frameLimit = (10000 * cfg.frame_rate[1]) / cfg.frame_rate[0];
  // Which module runs, and each frame's times, for the checks.
  t.module = moduleSource;
  t.timing = host.timing;
  const p = new Preview(host);
  let bundle;
  try {
    bundle = await fetchBundle(profile, p);
  } catch (e) {
    // A kept setting this model no longer has: retro.
    if (kept === null) throw e;
    profile = "retro";
    bundle = await fetchBundle(profile, p);
  }

  if (q.has("test")) {
    const names = await (await fetch("runs/index.json")).json();
    const results = [];
    for (const name of names) {
      const [frames, want] = await Promise.all(
        [`runs/${name}.frames`, `runs/${name}.hashes`].map((u) => fetch(u).then((r) => r.text())),
      );
      p.load(bundle);
      const t0 = performance.now();
      const got = p.runFrames(frames);
      const ms = performance.now() - t0;
      const a = got.trim().split("\n");
      const b = want.trim().split("\n");
      const first = a.findIndex((h, i) => h !== b[i]);
      results.push({ name, frames: b.length - 1, agrees: first < 0 && a.length === b.length, first, ms });
    }
    t.results = results;
    t.specDigest = p.specDigest();
    mark("tested");
  }

  // The hooks the module runs the model with, for the checks.
  t.hooks = p.load(bundle);
  // The frame, the level and the bodies being played, for the checks.
  t.state = () => p.state();
  // What the frame draws (the module's scene, gp_web::Preview::scene), for
  // the page check's layer order: the bodies against the HUD.
  t.scene = () => p.scene();
  p.loadPresent(presentText);
  // The model's sound (docs/sound.md): played by the same synthesizer as
  // the Bevy build, through WebAudio once a key is pressed.
  // preview.json says whether there is one; a page built before it asks.
  const soundText = cfg.sound === false ? null : await load("assets/sound.json").then((r) => (r.ok ? r.text() : null)).catch(() => null);
  let audio = null;
  let audioAt = 0;
  let audioAsked = 0;
  // A browser runs a context once the person has acted on the page: Chrome
  // once this frame or the page around it has had a click or a key (the
  // ROM's Load button is one), Safari only when it is made or resumed
  // during one. The context was made on a key or a click in this frame
  // only, so a game played with a gamepad, whose buttons are no gesture to
  // a browser, had no sound until a key or a click (check-page.mjs,
  // 2026-11-05: 0 frames of sound from Start with a gamepad under Chrome's
  // rule). It is now made as the game loads when the page
  // has had a gesture, resumed on every gesture in this frame, and the
  // embedding page resumes it on its own.
  //
  // In Safari a context made in this frame as it loads stays held even
  // after the click that loaded it (the user, 2026-11-05: a gamepad player
  // heard nothing until a key). So a page that offers one (the public
  // page, page.js `__gpAudio`) keeps the context: it makes or resumes it
  // in its own clicks, such as the click on "Load your own ROM", and the
  // context outlives this frame. `shared(make)` is that context, made when
  // `make` is set; null when the page offers none.
  const shared = (make) => {
    try {
      return parent !== window && typeof parent.__gpAudio === "function" ? parent.__gpAudio(make) : undefined;
    } catch {
      return undefined;
    }
  };
  const sound = () => {
    if (!audio && soundText) audio = shared(false) ?? null;
    return audio;
  };
  const unlock = () => {
    if (!soundText) return;
    audioAsked = performance.now();
    try {
      const s = shared(true);
      if (s !== undefined) {
        audio = s;
        return;
      }
      if (!audio) audio = new AudioContext({ sampleRate: 44100 });
      if (audio.state !== "running") audio.resume().catch(() => {});
    } catch {
      // A browser with no audio output: the game plays on in silence.
    }
  };
  t.unlockAudio = unlock;
  t.audioState = () => sound()?.state ?? "none";
  if (soundText) {
    p.loadSound(soundText);
    for (const e of ["keydown", "keyup", "pointerdown", "pointerup", "mousedown", "click", "touchend"]) addEventListener(e, unlock, true);
    const acted = (w) => {
      try {
        return !!w.navigator.userActivation?.hasBeenActive;
      } catch {
        return false;
      }
    };
    if (!sound() && (acted(window) || (parent !== window && acted(parent)))) unlock();
  }
  const play = () => {
    const f = p.audio();
    const audio = sound();
    if (!audio || !f) return;
    // A context the browser hasn't started yet (until its output starts,
    // or until a gesture it takes) has a clock that stands still. A frame
    // queued then waits, and every later frame behind it, so the sound
    // trailed the game by as long as the context waited: held 4 s, every
    // sound came 4.02 s late for the rest of the game (check-page.mjs,
    // 2026-11-04). Its frames are dropped instead, and the sound starts
    // with the game's next frame once it runs.
    if (audio.state !== "running") return;
    const now = audio.currentTime;
    audioAt = Math.max(audioAt, now + 0.05);
    // At most 0.25 s ahead: a frame further on is dropped, so the lead
    // can't grow after a slow stretch either.
    if (audioAt > now + 0.25) return;
    const b = audio.createBuffer(1, f.length, 44100);
    b.copyToChannel(f, 0);
    const src = audio.createBufferSource();
    src.buffer = b;
    src.connect(audio.destination);
    src.start(audioAt);
    audioAt += f.length / 44100;
  };
  mark("model-loaded");
  // Every picture the presentation names, by number (gp_present).
  const pics = await Promise.all(cfg.pictures.map((f) => image(`assets/${f}`)));
  const ownPics = pics.slice();
  // Themes found beside the bundle (docs/formats/theme.md, "Themes found
  // at run time"): themes/index.json lists looks, each a folder of the
  // files of assets/ it changes and look.txt. The settings screen lists
  // them; a change draws the next frame in it, with no restart.
  const catalogue = await themeCatalogue(cfg, p, q);
  const looks = await findLooks(cfg, catalogue);
  t.catalogue = () => ({
    source: catalogue.source,
    store: catalogue.cache?.kind ?? null,
    themes: catalogue.entries.map((e) => ({ ...e, downloaded: catalogue.downloaded.has(e.id) })),
  });
  const lookStore = settingsStore(cfg.model ?? cfg.title, "theme");
  let lookPresent = presentText;
  const tinted = new Map();
  // The look's sound per role, or the build's own role files.
  const applyRoles = async (look) => {
    if (!soundText) return;
    p.clearSoundRoles();
    const roles = look
      ? look.own_sound
        ? []
        : look.files.filter((f) => /^sound\/roles\/.+\.wav$/.test(f)).map((f) => [f.slice(12, -4), look.src(f)])
      : (JSON.parse(soundText).roles ?? []).map((r) => [r, `assets/sound/roles/${r}.wav`]);
    for (const [r, url] of roles) {
      const b = await load(url).then((x) => (x.ok ? x.arrayBuffer() : null));
      if (b) p.soundRole(r, new Uint8Array(b));
    }
  };
  const applyLook = async (id) => {
    // A published theme: downloaded first when it isn't, then drawn from
    // the cache.
    const e = catalogue.entries.find((x) => x.id === id);
    if (e && id !== looks.own[0] && !looks.found.some((l) => l.id === id && !l.published)) {
      if (!catalogue.downloaded.has(id)) await catalogue.download(e);
      const l = await catalogue.look(e);
      const at = looks.found.findIndex((x) => x.id === id);
      if (at >= 0) looks.found.splice(at, 1);
      looks.found.push(l);
    }
    const look = looks.found.find((l) => l.id === id && id !== looks.own[0]) ?? null;
    // Every file read first, then the module takes the present text, and
    // only then is the look the page's: a look that fails to load leaves
    // the one before whole.
    const next = [];
    for (let i = 0; i < pics.length; i++) {
      const f = cfg.pictures[i];
      next[i] = look?.files.includes(f) ? await image(look.src(f)) : ownPics[i];
    }
    // Through load(), as the pictures go through image(): a look of a
    // build lifted from the visitor's ROM is in IndexedDB, not on the
    // server (fetched, the page's 404 was read as the look's present text,
    // and choosing Paper on the settings screen in ROM mode failed with
    // "present text doesn't start with `gp-present 15`", 2026-11-04).
    const text = look?.files.includes("present.txt") ? await load(look.src("present.txt")).then((r) => r.text()) : presentText;
    p.loadPresent(text);
    // Kept only once the module takes it: a restart in new settings loads
    // the model afresh and then this text, so a look that failed here made
    // every later restart fail with no presentation, and the game froze
    // (Normal back to Retro after choosing Paper in ROM mode, 2026-11-04).
    lookPresent = text;
    pics.splice(0, pics.length, ...next);
    looks.current = look ? look.id : looks.own[0];
    tinted.clear();
    await applyRoles(look);
    t.theme = looks.current;
  };
  looks.apply = applyLook;
  await applyRoles(null);
  // `own-look` (the public page, when a person's ROM starts) opens in the
  // build's own look whatever look was kept from an earlier visit: a kept
  // Paper over a ROM's game looks like the paper version, as if the ROM
  // never loaded (2026-11-07). The settings screen still changes it.
  const wantLook = q.get("theme") ?? (q.has("test") || q.has("run") || q.has("own-look") ? null : lookStore.get());
  if (wantLook && looks.choices().some(([id]) => id === wantLook)) {
    try {
      await applyLook(wantLook);
    } catch (e) {
      // A kept theme that is no longer downloaded and can't be fetched.
      console.warn(`theme ${wantLook} not drawn: ${e.message}`);
    }
  }
  t.theme = looks.current;
  const theme = cfg.theme ?? { particles: [] };
  const overlay = theme.overlay ? await image(`assets/${theme.overlay}`) : null;
  mark("assets-loaded");

  const canvas = document.getElementById("game");
  const [vl, vt, vw, vh] = cfg.visible;
  canvas.width = vw;
  canvas.height = vh;
  // The pixels' shape: the page shows the canvas at the window's size.
  canvas.style.width = `${cfg.window[0]}px`;
  canvas.style.height = `${cfg.window[1]}px`;
  canvas.style.imageRendering = "pixelated";
  const ctx = canvas.getContext("2d");
  ctx.imageSmoothingEnabled = false;
  const script = q.get("run") ? parseFrames(await (await fetch(`runs/${q.get("run")}.frames`)).text()) : null;

  // A white mask in a color, made once per mask and color (emptied when
  // the look changes).
  const tint = (pic, [r, g, b]) => {
    const key = `${pic}:${r},${g},${b}`;
    let c = tinted.get(key);
    if (!c) {
      const m = pics[pic];
      c = document.createElement("canvas");
      c.width = m.width;
      c.height = m.height;
      const x = c.getContext("2d");
      x.fillStyle = `rgb(${r},${g},${b})`;
      x.fillRect(0, 0, m.width, m.height);
      x.globalCompositeOperation = "destination-in";
      x.drawImage(m, 0, 0);
      tinted.set(key, c);
    }
    return c;
  };
  const put = (img, x, y, fx, fy) => {
    if (!fx && !fy) {
      ctx.drawImage(img, x, y);
      return;
    }
    ctx.save();
    ctx.translate(x + (fx ? img.width : 0), y + (fy ? img.height : 0));
    ctx.scale(fx ? -1 : 1, fy ? -1 : 1);
    ctx.drawImage(img, 0, 0);
    ctx.restore();
  };

  const draw = (pl = p) => {
    const s = pl.scene();
    const [cx, cy] = s.camera;
    // The window drawn: the game's view, or wider with widescreen
    // (gp_present::view). A module built before it draws the view.
    const win = s.window ?? { x: cx + vl, y: cy + vt, w: vw, h: vh, view_x: 0, wall: 0, hud: [cx, cy] };
    if (canvas.width !== win.w) {
      canvas.width = win.w;
      ctx.imageSmoothingEnabled = false;
      canvas.dispatchEvent(new Event("gp-resize"));
    }
    // World pixels to canvas pixels: the window's corner.
    const ox = -win.x;
    const oy = -win.y;
    // The HUD's screen pixels to canvas pixels.
    const hx = win.hud[0] - win.x;
    const hy = win.hud[1] - win.y;
    ctx.fillStyle = `rgb(${s.clear.join(",")})`;
    ctx.fillRect(0, 0, win.w, vh);
    // Bodies behind the level first, so it covers them where it draws.
    const body = (b, i) => {
      if (!b) return;
      const [pic, x, y, fx, fy] = b;
      ctx.globalAlpha = (s.alpha?.[i] ?? 255) / 255;
      // The parts the console's sprite limit drops (gp_present::view::
      // sprite_cuts), from the picture's top left on the screen: drawn
      // through a clip of the picture's box less them.
      const cut = s.cut?.[i] ?? [];
      if (cut.length === 0) {
        put(pics[pic], x + ox, y + oy, fx, fy);
        return;
      }
      ctx.save();
      ctx.beginPath();
      ctx.rect(x + ox, y + oy, pics[pic].width, pics[pic].height);
      for (const [cx0, cy0, cw, ch] of cut) ctx.rect(x + ox + cx0, y + oy + cy0, cw, ch);
      ctx.clip("evenodd");
      put(pics[pic], x + ox, y + oy, fx, fy);
      ctx.restore();
    };
    // Each pass paints its bodies in the module's `order` (gp_present's
    // Scene::paint_order: the console's sprite table, the earlier sprite
    // on top), as the native picture and the Bevy build do. In body order
    // the points a barrel jump scores were painted over the barrel the
    // original draws over them (the user, 2026-11-08: "a score floating
    // off a barrel" looked like a ghost). A module built before it gives
    // none: body order.
    const order = s.order ?? s.bodies.map((_, i) => i);
    const pass = (want) => order.forEach((i) => want(i) && body(s.bodies[i], i));
    pass((i) => s.behind?.[i]);
    ctx.globalAlpha = 1;
    if (s.level !== null) ctx.drawImage(pics[s.level], ox, oy);
    for (const [pic, color] of s.tints) ctx.drawImage(tint(pic, color), ox, oy);
    // The cells the game changed (a coin taken, a block used), over the
    // level and its tints. A module built before them has none.
    for (const [pic, x, y, fx, fy] of s.cells ?? []) put(pics[pic], x + ox, y + oy, fx, fy);
    // The HUD's background part (its layer and the fields in background
    // palettes): the console's background, so the bodies go over it, as
    // the hero goes over the status row on the NES. A module built before
    // hud_sprites draws the whole HUD over the bodies, as it did.
    const hudBg = s.hud_sprites === undefined ? 0 : s.hud.length - s.hud_sprites;
    const hud = (from, to) =>
      s.hud.slice(from, to).forEach(([pic, x, y, fx, fy], j) => {
        put(pics[pic], x + hx, y + hy, fx, fy);
        // The layer's cycling colors go over it, under the fields.
        if (from + j === 0) for (const [m, color] of s.hud_tints) ctx.drawImage(tint(m, color), hx, hy);
      });
    hud(0, hudBg);
    // Then the rest, and last the bodies in front of every other.
    pass((i) => !s.behind?.[i] && !s.front?.[i]);
    pass((i) => !s.behind?.[i] && s.front?.[i]);
    for (const [pic, x, y, fx, fy, a] of s.ghosts ?? []) {
      ctx.globalAlpha = a / 255;
      put(pics[pic], x + ox, y + oy, fx, fy);
    }
    ctx.globalAlpha = 1;
    if (win.wall > 0) {
      ctx.fillStyle = "rgba(0,0,0,0.45)";
      ctx.fillRect(0, 0, win.wall, vh);
    }
    // The HUD's sprites over the bodies.
    hud(hudBg, s.hud.length);
    // The theme's particles and overlay, over the camera's view: drawn
    // from the state, never written to it.
    if (theme.particles.length || overlay) {
      const st = pl.state();
      for (const e of theme.particles) {
        const bodies = st.bodies
          .filter(([kind, , , , alive]) => alive && cfg.kinds[kind]?.role === e.role)
          .map(([kind, x, y]) => [x - cx, y - cy, ...cfg.kinds[kind].size]);
        ctx.fillStyle = `rgb(${e.color.join(",")})`;
        // Over the game's view, where it is in the window.
        for (const d of dots(e, st.frame, cfg.view, bodies)) {
          ctx.globalAlpha = Math.max(0, Math.min(1, d.a));
          ctx.fillRect(Math.floor(d.x) - vl + win.view_x, Math.floor(d.y) - vt, d.w, d.h);
        }
        ctx.globalAlpha = 1;
      }
      // Stretched across the window: the overlay is the view's size.
      if (overlay) ctx.drawImage(overlay, -vl, -vt, win.w + cfg.view[0] - vw, cfg.view[1]);
    }
  };

  const st = p.hasSettings() ? settingsScreen(p, canvas, ctx, profilesText, looks) : null;
  // For the checks: the screen as text, and the state hash now.
  t.settingsLines = () => (st?.open ? p.settingsLines() : null);
  t.hashNow = () => p.hash();
  // The game's name its saves are kept under, and the digest its older
  // keys named.
  t.game = cfg.model ?? cfg.title;
  t.cfgDigest = cfg.spec_digest ?? null;
  // For the checks: whether the screen is open, and the setting played.
  Object.defineProperty(t, "settingsOpen", { get: () => !!st?.open, enumerable: true });
  Object.defineProperty(t, "profile", { get: () => profile, enumerable: true });

  if (q.has("control")) {
    // A second model for still pictures, so asking for one never moves the
    // one being watched (seeking back starts a model over from power-on).
    const stills = new Preview(await openHost(gameModule));
    control(p, stills, bundle, presentText, canvas, draw, profile, st);
    placeInView(canvas, cfg.visible, cfg.view);
    return;
  }

  // Fixed timestep at the model's frame rate; the clock decides when a
  // frame runs, never what it does.
  const tick = (1000 * cfg.frame_rate[1]) / cfg.frame_rate[0];
  t.hashes.push(p.hash());
  let last = performance.now();
  let acc = 0;
  let frame = 0;
  // The page's side of the shell: another mode for a save's load, and the
  // screen closed after one.
  const shellHost = {
    restartIn: (name) => restartIn(name, true),
    screenOpen: () => !!st?.open,
    closeLoaded: () => {
      const r = st?.closeLoaded();
      if (r?.closed) onClosed(r, true);
    },
  };
  const sh = script ? null : shell(p, cfg, canvas, () => profile, shellHost);
  if (st && sh) st.fill = () => sh.fillScreen();
  // Buttons held as the settings screen closed, kept from the game until
  // let go, so the Enter that closed it isn't Start.
  let mask = 0;
  let restarting = false;
  // For the checks: a restart in new settings under way (the loop holds).
  Object.defineProperty(t, "restarting", { get: () => restarting, enumerable: true });
  // The game again from power-on in a new setting: the switches change the
  // model before it runs.
  // `quiet` for a load's own restart: the load says what happened. True
  // when the game runs in `name`.
  const restartIn = async (name, quiet = false) => {
    restarting = true;
    let ok = true;
    try {
      bundle = await fetchBundle(name, p);
      profile = name;
      t.hooks = p.load(bundle);
      p.loadPresent(lookPresent);
      if (soundText) {
        p.loadSound(soundText);
        await applyRoles(looks.found.find((l) => l.id === looks.current && l.id !== looks.own[0]) ?? null);
      }
      frame = 0;
      t.hashes = [p.hash()];
      sh?.restarted(quiet ? null : `Settings: ${name}. The game starts again from power-on`);
    } catch (e) {
      ok = false;
      sh?.restarted(`Settings not applied: ${e?.message ?? e}`);
    }
    restarting = false;
    return ok;
  };
  // Esc, F1 or the right stick's press opens the settings screen (so does
  // the embedding page's message); while it is open the game holds and the
  // keys are the screen's.
  let openAsked = false;
  if (!script) {
    addEventListener("message", (e) => {
      const m = e.data;
      if (e.origin === location.origin && m && m.gp === "settings" && m.key === "open") openAsked = true;
    });
  }
  const settingsFrame = () => {
    if (!st || script) return false;
    const pads = st.padKeys();
    if (!st.open) {
      const i = pressed.findIndex((e) => webKeys("settings").includes(e.code));
      if (i < 0 && !pads.includes(6) && !openAsked) return false;
      if (i >= 0) pressed.splice(i, 1);
      openAsked = false;
      st.show(profile);
      sh?.say("Settings: arrows move, Enter toggles, Esc closes");
      return true;
    }
    const codes = pressed.splice(0).map((e) => SETTINGS_KEY[e.code]).filter((c) => c !== undefined);
    // The page's Settings button pressed again closes the screen, as the
    // settings key does; before, the ask waited and opened the screen
    // again as soon as Esc closed it.
    if (openAsked) codes.push(6);
    openAsked = false;
    for (const c of [...codes, ...pads]) {
      const r = st.key(c);
      for (const id of p.settingsRemoved()) {
        void catalogue.remove?.(id).then(() => sh?.say(`Theme removed from the cache: ${id}`));
      }
      // A save slot asked for on the Saves line: done by the shell, the
      // screen staying open (a load closes it).
      if (r?.slot) {
        void sh?.slotAsked(r.slot);
        continue;
      }
      if (!r?.closed) continue;
      onClosed(r);
      break;
    }
    return st.open;
  };
  // The screen closed. After a load (`loaded`) the game runs in the save's
  // mode for this play, so the mode kept stays as it was and isn't played.
  function onClosed(r, loaded = false) {
    store.set(r.profile);
    if (r.theme && r.theme !== looks.current) {
      const title = looks.choices().find(([id]) => id === r.theme)?.[1] ?? r.theme;
      const fetching = catalogue.entries.some((e) => e.id === r.theme) && !catalogue.downloaded.has(r.theme);
      if (fetching) sh?.say(`Downloading ${title}...`);
      applyLook(r.theme).then(
        () => {
          lookStore.set(r.theme);
          sh?.say(`Theme: ${title}${fetching ? ", downloaded and checked" : ""}`);
        },
        (e) => sh?.say(`Theme not changed: ${e.message}`),
      );
    }
    sh?.screenClosed();
    mask = buttons();
    if (loaded) return;
    // The same switches under another name (an empty normal) play on.
    if (r.restart) void restartIn(r.profile);
    else profile = r.profile;
  }
  const end = script && q.has("to") ? Math.min(script.length, Number(q.get("to"))) : script?.length ?? 0;
  if (script && q.has("from")) {
    const from = Math.min(end, Number(q.get("from")));
    for (; frame < from; frame++) {
      const [b, r] = script[frame];
      p.step(b, r);
    }
  }
  let held = false;
  let soundNoteAt = 0;
  // A gamepad player whose sound the browser still holds (no click or key
  // has started it: Safari takes neither a gamepad's buttons nor, in a
  // page that keeps no context, the click that loaded the game) is asked
  // once, over the game and before it steps, for the one click or key
  // that starts the sound. Never shown while the sound runs.
  let prompt = null;
  let prompted = false;
  const askForSound = () => {
    prompted = true;
    prompt = document.createElement("div");
    prompt.id = "sound-prompt";
    prompt.textContent = "Click or press a key to play with sound";
    Object.assign(prompt.style, {
      position: "fixed", inset: "0", display: "flex", alignItems: "center", justifyContent: "center",
      background: "rgba(0, 0, 0, 0.6)", color: "#fff", font: "bold 22px system-ui, sans-serif", cursor: "pointer", zIndex: 5,
    });
    document.body.append(prompt);
    t.soundPrompt = true;
    const done = (e) => {
      // `unlock` has run already (it listens first), so this only takes
      // the key or click away from the game.
      e.preventDefault();
      e.stopPropagation();
      prompt.remove();
      prompt = null;
      t.soundPrompt = false;
      for (const k of ["keydown", "pointerdown"]) removeEventListener(k, done, true);
    };
    for (const k of ["keydown", "pointerdown"]) addEventListener(k, done, true);
  };
  const loop = (now) => {
    acc = Math.min(acc + (now - last), 250);
    last = now;
    if (restarting || settingsFrame() || prompt) {
      // The game holds while the screen is open: no step, no time owed.
      acc = 0;
      draw();
      st?.draw();
      requestAnimationFrame(loop);
      return;
    }
    // Slow motion stretches the step (1, 2 or 4 times); what a step does is the same.
    const slow = sh ? sh.slow() : 1;
    while (acc >= tick * slow) {
      acc -= tick * slow;
      if (script) {
        if (frame >= end) break;
        const [b, r] = script[frame];
        p.step(b, r);
      } else if (sh && (held = sh.before())) {
        // The shell put a state back (a rewind or a load), or frame advance
        // is holding the tick: no step.
        if (held === "back") t.hashes.push(p.hash());
        continue;
      } else {
        const b = buttons();
        // Half a second after the sound was last asked for, so a context
        // still starting isn't taken for one held.
        if (!prompted && soundText && padBits() && sound()?.state !== "running" && performance.now() - audioAsked > 500) {
          askForSound();
          break;
        }
        // Played from a gamepad while the browser holds the sound: say
        // what starts it, as no button of a gamepad can.
        if (soundText && padBits() && sound()?.state !== "running" && performance.now() > soundNoteAt) {
          soundNoteAt = performance.now() + 5000;
          sh?.say("Sound starts once you press a key or click the game (a browser rule)");
        }
        mask &= b;
        p.step(b & ~mask, resetPressed);
        resetPressed = false;
      }
      frame += 1;
      if (slow === 1) play();
      t.hashes.push(p.hash());
      sh?.after();
    }
    draw();
    if (!("first-frame" in t.marks)) requestAnimationFrame(() => mark("first-frame"));
    requestAnimationFrame(loop);
  };
  draw();
  requestAnimationFrame(loop);
}

// The runtime shell for keyboard and gamepad play (docs/shell.md): slots,
// the autosave, rewind and the preview's own options, on the standard's
// shared parts. A slot holds a save record (gp_save_record) with a stamp of
// when it was saved beside it, kept by the shell's storage (storage.js, the
// WIT's interface over IndexedDB) under the standard's names (slotName,
// whenName), and is put back with gp_load_record; the rewind ring is the
// module's (gp_ring_*). The settings screen's Saves line lists the slots
// (gp_settings_saves) and hands back what the player asked of one, which
// `slotAsked` does here. A load or a rewind steps on exactly as the game
// would have from there; nothing here changes what a step does.
//
// `host` is the page's: `restartIn(name)` starts the game again from
// power-on in another mode, and `closeLoaded()` closes the settings screen
// after a load from it.
function shell(p, cfg, canvas, profileNow, host) {
  const ls = (() => {
    try {
      return window.localStorage;
    } catch {
      return null;
    }
  })();
  const lsGet = (k) => {
    try {
      return ls?.getItem(k) ?? null;
    } catch {
      return null;
    }
  };
  const lsSet = (k, v) => {
    try {
      ls?.setItem(k, v);
    } catch {
      // Storage full or blocked: the setting just isn't kept.
    }
  };
  const lsDel = (k) => {
    try {
      ls?.removeItem(k);
    } catch {
      // Blocked: nothing was kept there.
    }
  };
  const game = cfg.model ?? cfg.title;
  // The preview's own options (scale, widescreen, flash dimming, mute) are
  // kept by the game, so a page update keeps them (2026-11-05). Before,
  // they were kept by the build's digest, `gp-shell:<digest>:`, and every
  // update lost them: such a key is read once, moved and removed.
  const oldKey = (k) => `gp-shell:${cfg.spec_digest ?? cfg.title}:${k}`;
  const key = (k) => `gp-view:${game}:${k}`;
  const get = (k) => {
    const v = lsGet(key(k));
    if (v !== null) return v;
    const old = lsGet(oldKey(k));
    if (old !== null) {
      lsSet(key(k), old);
      lsDel(oldKey(k));
    }
    return old;
  };
  const set = (k, v) => lsSet(key(k), v);
  const b64 = (bytes) => btoa(Array.from(bytes, (c) => String.fromCharCode(c)).join(""));
  const unb64 = (s) => Uint8Array.from(atob(s), (c) => c.charCodeAt(0));
  // A save record from a profile and a state (gp_sim::save, version 2), for
  // a slot kept before records: "GPSV", the version, then the model's name,
  // the digest and the profile, each a u16 length and UTF-8, then the state.
  const record = (digest, profile, state) => {
    const parts = [game, digest, profile].map((x) => enc.encode(x));
    const out = new Uint8Array(8 + parts.reduce((n, x) => n + 2 + x.length, 0) + state.length);
    const v = new DataView(out.buffer);
    out.set(enc.encode("GPSV"), 0);
    v.setUint32(4, 2, true);
    let at = 8;
    for (const x of parts) {
      v.setUint16(at, x.length, true);
      out.set(x, at + 2);
      at += 2 + x.length;
    }
    out.set(state, at);
    return out;
  };
  // The shell's own settings (gp_present::shell::Settings), as their file
  // holds them: Console only, and the quick save buttons.
  const parseShell = (s) => ({ console_only: /^console_only\s*=\s*true\s*$/m.test(s), quick_buttons: /^quick_buttons\s*=\s*true\s*$/m.test(s) });
  const shellText = (s) => `console_only = ${s.console_only}\nquick_buttons = ${s.quick_buttons}\n`;
  let settings = parseShell("");
  // Each binding's need of those settings, from the table.
  const needs = new Map(BINDINGS.filter((b) => b.on !== "bevy").map((b) => [b.action, b.needs]));
  const allowed = (act) => {
    const n = needs.get(act) ?? "always";
    if (n === "not-console-only") return !settings.console_only;
    if (n === "quick-buttons") return settings.quick_buttons && !settings.console_only;
    return true;
  };
  // The slots and their stamps by storage name, read once the storage
  // opens; a save is kept here at once and written behind.
  const kept = new Map();
  let files = null;
  let writes = Promise.resolve();
  const opened = openStorage(game)
    .then(async (s) => {
      // Slots kept in localStorage before the shell's storage are moved
      // into it, and their keys removed: a record by game (`gp-save:`,
      // 2026-11-04 to 2026-11-05), or the state and profile as JSON by
      // the build's digest (`gp-shell:`, before 2026-11-04), which becomes
      // a record when it is of this page's build and of the mode played.
      let moved = 0;
      for (let n = 1; n <= QUICK_SLOTS; n++) {
        const byGame = `gp-save:${game}:slot${n}`;
        const rec = lsGet(byGame);
        const json = cfg.spec_digest ? lsGet(oldKey(`slot${n}`)) : null;
        let bytes = rec ? unb64(rec) : null;
        if (!bytes && json) {
          const { profile: was, state } = JSON.parse(json);
          if (was === profileNow()) bytes = record(p.specDigest(), was, unb64(state));
        }
        if (!bytes) continue;
        // A slot already in the storage was saved later: it stays.
        if ((await s.read(slotName(n))) === null) {
          await s.write(slotName(n), bytes);
          moved++;
        }
        if (rec) lsDel(byGame);
        if (json) lsDel(oldKey(`slot${n}`));
      }
      t.movedSlots = moved;
      for (const name of await s.list("saves/")) {
        if (!kept.has(name)) kept.set(name, await s.read(name));
      }
      const shellFile = await s.read(SHELL_SETTINGS);
      if (shellFile) settings = parseShell(text.decode(shellFile));
      files = s;
      return s;
    })
    .catch(() => null);
  const keep = (name, bytes) =>
    (writes = writes
      .then(() => opened)
      .then((s) => {
        if (!s) throw new Error("this browser keeps no files for the page");
        return bytes === null ? s.delete(name) : s.write(name, bytes);
      }));
  const slotBytes = (n) => kept.get(slotName(n)) ?? null;
  const stamp = (n) => {
    const w = kept.get(whenName(n));
    return w ? Number(text.decode(w)) : null;
  };
  const rate = cfg.frame_rate[0] / cfg.frame_rate[1];
  const can = p.stateBytes() !== null && p.ringReset(cfg.frame_rate[0], cfg.frame_rate[1]);
  // Slots need a module that writes save records (2026-11-04); rewind
  // needs the module's ring (2026-11-05).
  const canSave = can && p.saveRecord(profileNow()) !== null;
  const bar = document.createElement("div");
  Object.assign(bar.style, { font: "12px monospace", color: "#ccc", textAlign: "center", padding: "6px" });
  canvas.after(bar);
  const canView = p.view("");
  const help = () =>
    (!can
      ? "F8 scale (this gp_web.wasm has no gp_state; rebuild it for saves and rewind)"
      : settings.console_only
        ? "Console only: no saves or rewind, F8 scale"
        : `${canSave ? "Shift+1-4 save, 1-4 load, " : ""}hold Backspace to rewind, F8 scale${settings.quick_buttons && canSave ? ", RB quick save, LB quick load" : ""}`) +
    (canView ? ", F9 widescreen, F10 dim flashes, 5-9 mute a channel" : "") +
    ", F2 pause, F3 one frame, F4 slow motion" +
    (p.hasSettings() ? ", Esc settings" : "");
  // Picture and sound options, kept per game.
  const wides = ["off", "16:10", "16:9"];
  const channels = ["pulse1", "pulse2", "triangle", "noise", "dmc"];
  let wide = wides.includes(get("wide")) ? get("wide") : "off";
  let dim = get("dim") === "1";
  const muted = new Set((get("mute") ?? "").split("+").filter((c) => channels.includes(c)));
  const applyView = () => {
    if (!canView) return;
    p.view(`wide=${wide},dim=${dim ? "on" : "off"},mute=${[...muted].join("+")}`);
  };
  applyView();
  // Practice tools that only change the pace (docs/generator.md, "Practice
  // tools"): F2 holds the tick, F3 lets one through while held, F4 steps
  // the speed through full, 1/2 and 1/4. Hitboxes and level select's menu
  // are not here (level select is the page's `level=` parameter).
  const speeds = [1, 2, 4];
  let speed = 1;
  let halted = false;
  let advance = 0;
  let note = "";
  let noteUntil = 0;
  const say = (m) => {
    note = m;
    noteUntil = performance.now() + 2500;
  };
  const show = () => {
    bar.textContent = note && performance.now() < noteUntil ? note : help();
  };
  const frameOf = (bytes) => Number(new DataView(bytes.buffer, bytes.byteOffset).getBigUint64(8, true));
  const time = (f) => {
    const s = Math.floor(f / rate);
    return `${Math.floor(s / 60)}:${String(s % 60).padStart(2, "0")}`;
  };
  const modeTitle = (m) => (m.includes("+") ? "Custom" : m.charAt(0).toUpperCase() + m.slice(1).replace(/_/g, " "));
  const label = (n) => (n === AUTOSAVE_SLOT ? "Resume" : `slot ${n}`);
  // Scale: whole multiples of the window's size, then fit the page.
  const scales = [1, 2, 3, 4, "fit"];
  // A kept scale wins; else the page's `scale=` (the public page asks for
  // fit), else 1x.
  const asked = get("scale") ?? new URLSearchParams(location.search).get("scale");
  let scale = scales.includes(asked) ? asked : Number(asked) || 1;
  const applyScale = () => {
    // The window's width follows the canvas's, wider with widescreen.
    const [w, h] = [Math.round((cfg.window[0] * canvas.width) / cfg.visible[2]), cfg.window[1]];
    let k = scale;
    if (scale === "fit") k = Math.min((innerWidth - 16) / w, (innerHeight - 40) / h);
    canvas.style.width = `${Math.floor(w * k)}px`;
    canvas.style.height = `${Math.floor(h * k)}px`;
  };
  addEventListener("resize", applyScale);
  canvas.addEventListener("gp-resize", applyScale);
  applyScale();
  show();
  // The slot the quick save buttons use: the last one saved or loaded.
  let lastSlot = 1;
  let persistAsked = false;
  // Saves the game into slot n (0 is the autosave, written quietly) with
  // the time it was saved. The player's first save asks the browser to
  // keep the saves for good (navigator.storage.persist()).
  const save = (n) => {
    const bytes = p.saveRecord(profileNow());
    const now = enc.encode(String(Math.floor(Date.now() / 1000)));
    kept.set(slotName(n), bytes);
    kept.set(whenName(n), now);
    void keep(slotName(n), bytes)
      .then(() => keep(whenName(n), now))
      .catch((e) => say(`${n === AUTOSAVE_SLOT ? "Autosave" : `Slot ${n}`} not kept: ${e?.message ?? e}`));
    if (n === AUTOSAVE_SLOT) return;
    lastSlot = n;
    if (!persistAsked) {
      persistAsked = true;
      t.persistAsked = true;
      void opened.then((s) => s?.persist()).then((v) => (t.persisted = !!v));
    }
    say(`Saved to slot ${n} at ${time(frameOf(p.stateBytes()))}`);
  };
  // Loads a save record; in another mode the game is started again in that
  // mode for this play first (the kept setting stays). Returns what was
  // said: null when it loaded.
  const loadBytes = async (bytes, what) => {
    let got = p.loadRecord(bytes, profileNow());
    let inMode = "";
    if (got.profile !== undefined) {
      const was = got.profile;
      if (!(await host.restartIn(was))) return `${what} is in ${modeTitle(was)}, which this game can't start in`;
      got = p.loadRecord(bytes, profileNow());
      inMode = `, in ${modeTitle(was)}`;
    }
    if (got.refused !== undefined) return got.refused;
    if (got.profile !== undefined) return `${what} is in ${modeTitle(got.profile)}`;
    p.ringClear();
    say(`Loaded ${what}${inMode}, ${time(frameOf(p.stateBytes()))}`);
    return null;
  };
  // Loads slot n from a quick key; true when a state was put back now.
  const load = (n) => {
    const slot = slotBytes(n);
    if (!slot) {
      say(files || n > QUICK_SLOTS ? `Slot ${n} is empty` : `Slot ${n} is empty (the saves are still being read)`);
      return false;
    }
    lastSlot = n;
    const got = p.loadRecord(slot, profileNow());
    if (got.profile !== undefined) {
      void loadBytes(slot, `slot ${n}`).then((m) => m && say(m));
      return false;
    }
    if (got.refused !== undefined) {
      say(got.refused);
      return false;
    }
    p.ringClear();
    say(`Loaded slot ${n}, ${time(frameOf(p.stateBytes()))}`);
    return true;
  };
  // The quick save buttons and rewind's trigger on any gamepad: pressed
  // since the last look, and held.
  const padWas = new Set();
  const padActions = () => {
    const out = [];
    let rewind = false;
    const quick = [
      ["quick-save", BINDINGS.find((b) => b.action === "quick-save")?.standard ?? []],
      ["quick-load", BINDINGS.find((b) => b.action === "quick-load")?.standard ?? []],
    ];
    const rw = BINDINGS.filter((b) => b.action === "rewind" && b.on !== "bevy").flatMap((b) => b.standard);
    for (const g of navigator.getGamepads?.() ?? []) {
      if (!g || !g.connected) continue;
      if (rw.some((i) => g.buttons[i]?.pressed)) rewind = true;
      for (const [act, ix] of quick) {
        for (const i of ix) {
          const id = `${g.index}:${i}`;
          const on = !!g.buttons[i]?.pressed;
          if (on && !padWas.has(id)) out.push(act);
          if (on) padWas.add(id);
          else padWas.delete(id);
        }
      }
    }
    return { out, rewind };
  };
  t.shellSettings = () => ({ ...settings });
  t.lastSlot = () => lastSlot;
  return {
    say,
    // The storage, once open, for the page's checks.
    storage: () => opened,
    // Every write handed to the storage, done.
    written: () => writes,
    // The game started again from power-on in new settings: the ring is
    // of the old game, and the module's options went with it.
    restarted(m) {
      if (can) p.ringReset(cfg.frame_rate[0], cfg.frame_rate[1]);
      applyView();
      if (m) say(m);
      show();
    },
    // The settings screen's shell lines and Saves line, as it opens.
    fillScreen() {
      if (!p.hasSaves()) return;
      p.settingsShell(shellText(settings));
      if (!canSave) return;
      const slots = [];
      for (let n = AUTOSAVE_SLOT; n <= QUICK_SLOTS; n++) slots.push([n, slotBytes(n), stamp(n)]);
      p.settingsSaves(slots, profileNow(), Math.floor(Date.now() / 1000), cfg.frame_rate, !!files);
    },
    // The screen closed: the shell's settings chosen on it are kept.
    screenClosed() {
      const chosen = p.hasSaves() ? p.settingsShellChosen() : null;
      if (chosen === null || chosen === shellText(settings)) return;
      const was = settings;
      settings = parseShell(chosen);
      void keep(SHELL_SETTINGS, enc.encode(chosen)).catch(() => {});
      if (settings.console_only !== was.console_only) say(settings.console_only ? "Console only: saves, loads and rewind are off" : "Saves, loads and rewind are on again");
      else say(settings.quick_buttons ? "Quick save buttons on: RB saves, LB loads" : "Quick save buttons off");
      show();
    },
    // What the player asked of a slot on the settings screen: [what, n].
    async slotAsked([what, n]) {
      const tell = (m) => {
        if (host.screenOpen()) p.settingsSay(m);
        else say(m);
      };
      const refresh = () => {
        if (host.screenOpen()) this.fillScreen();
      };
      try {
        if (what === "save") {
          save(n);
          refresh();
          tell(`Saved to slot ${n}`);
        } else if (what === "delete") {
          kept.delete(slotName(n));
          kept.delete(whenName(n));
          if (lastSlot === n) lastSlot = 1;
          await keep(slotName(n), null);
          await keep(whenName(n), null);
          refresh();
          tell(`${n === AUTOSAVE_SLOT ? "Resume" : `Slot ${n}`} deleted`);
        } else if (what === "download") {
          await writes;
          const s = await opened;
          if (!s || !slotBytes(n)) throw new Error(`slot ${n} is empty`);
          const file = exportName(game, n);
          await s.export(slotName(n), file);
          t.downloaded = file;
          tell(`${n === AUTOSAVE_SLOT ? "Resume" : `Slot ${n}`} downloaded as ${file}`);
        } else if (what === "load" || what === "file") {
          let bytes = what === "load" ? slotBytes(n) : null;
          let name = what === "load" ? label(n) : null;
          if (what === "file") {
            const s = await opened;
            const picked = s ? await s.import() : null;
            if (!picked) return tell("No file chosen");
            bytes = picked.bytes;
            name = picked.fileName;
          }
          if (!bytes) return tell(`${name} is empty`);
          const refused = await loadBytes(bytes, name);
          if (refused) return tell(refused);
          if (what === "load" && n !== AUTOSAVE_SLOT) lastSlot = n;
          host.closeLoaded();
        }
      } catch (e) {
        tell(`Not done: ${e?.message ?? e}`);
      }
    },
    // Before a step: the keys, then rewind. True when a state was put
    // back, so this tick doesn't step.
    before() {
      let back = false;
      const pads = padActions();
      const acts = pressed.splice(0).map(({ code, shift }) => shellAction(code, shift) ?? shellAction(code, false));
      for (const act of [...acts, ...pads.out]) {
        // A key with Shift held does what it does alone unless the table
        // gives Shift+key its own action (Shift+1 saves).
        if (!act || !allowed(act)) continue;
        if (act === "scale") {
          scale = scales[(scales.indexOf(scale) + 1) % scales.length];
          set("scale", String(scale));
          applyScale();
          say(`Scale ${scale === "fit" ? "fit" : `${scale}x`}`);
        }
        if (act === "widescreen" && canView) {
          wide = wides[(wides.indexOf(wide) + 1) % wides.length];
          set("wide", wide);
          applyView();
          say(`Widescreen ${wide}`);
        }
        if (act === "dim-flashes" && canView) {
          dim = !dim;
          set("dim", dim ? "1" : "0");
          applyView();
          say(`Flash dimming ${dim ? "on" : "off"}`);
        }
        if (act === "pause") {
          halted = !halted;
          say(halted ? "Paused, F3 plays one frame" : "Playing");
        }
        if (act === "frame" && halted) advance += 1;
        if (act === "slow") {
          speed = speeds[(speeds.indexOf(speed) + 1) % speeds.length];
          say(speed === 1 ? "Full speed" : `Slow motion, 1/${speed}`);
        }
        const ch = /^mute-(.+)$/.exec(act)?.[1];
        if (ch && channels.includes(ch) && canView) {
          if (muted.has(ch)) muted.delete(ch);
          else muted.add(ch);
          set("mute", [...muted].join("+"));
          applyView();
          say(`${ch} ${muted.has(ch) ? "muted" : "on"}`);
        }
        if (!canSave) continue;
        if (act === "quick-save") save(lastSlot);
        if (act === "quick-load" && load(lastSlot)) back = true;
        const [, what, n] = /^(save|load)-(\d+)$/.exec(act) ?? [];
        if (what === "save") save(Number(n));
        else if (what === "load" && load(Number(n))) back = true;
      }
      const rewinding = allowed("rewind") && (webKeys("rewind").some((k) => down.has(k)) || pads.rewind);
      if (!back && can && rewinding && p.ringBack()) back = true;
      show();
      if (back) return "back";
      if (halted) {
        if (advance === 0) return "hold";
        advance -= 1;
      }
      return false;
    },
    slow: () => speed,
    // After a step: keep the state for rewinding, and the autosave at each
    // level start (gp_present::shell::autosaves), into slot 0, "Resume".
    after() {
      if (can) p.ringNote();
      if (canSave && !settings.console_only && p.autosaveDue()) save(AUTOSAVE_SLOT);
    },
  };
}

// The embedded mode: the parent says which frame to show. Seeking back
// starts the model over from power-on, so a frame is always the same
// steps from the same start.
// The page that embeds this compares it with the original's whole frame
// (the view, 256 by 240 on the NES), so the visible part sits where it is
// in the view and the rows outside it stay black: drawn over the original,
// the two line up pixel for pixel.
function placeInView(canvas, visible, view) {
  const [vl, vt, vw, vh] = visible;
  const [w, h] = view;
  if (vl === 0 && vt === 0 && vw === w && vh === h) return;
  document.documentElement.style.height = "100%";
  Object.assign(document.body.style, { position: "relative", height: "100%", overflow: "hidden" });
  Object.assign(canvas.style, {
    position: "absolute",
    margin: "0",
    left: `${(vl / w) * 100}%`,
    top: `${(vt / h) * 100}%`,
    width: `${(vw / w) * 100}%`,
    height: `${(vh / h) * 100}%`,
  });
}

function control(p, stills, bundle, presentText, canvas, drawGame, profile, st) {
  // The settings screen over the picture while the parent has it open.
  const draw = (pl = p) => {
    drawGame(pl);
    if (pl === p) st?.draw();
  };
  canvas.style.width = "100%";
  canvas.style.height = "auto";
  document.body.style.background = "#000";
  let steps = [];
  // The state the steps start from: power-on, or one loaded (the
  // original's, translated), so frame n is always n steps from it.
  let start = null;
  // A model and the frame it's at: the one watched, and the one stills are taken on.
  const watched = { p, at: 0 };
  const still = { p: stills, at: 0 };
  const reset = (m) => {
    m.p.load(bundle);
    m.p.loadPresent(presentText);
    if (start) m.p.loadState(start);
    m.at = 0;
  };
  const step = (m, n) => {
    const to = Math.max(0, Math.min(Number(n) || 0, steps.length));
    if (to < m.at) reset(m);
    for (; m.at < to; m.at++) {
      const [b, r] = steps[m.at];
      m.p.step(b, r);
    }
  };
  const seek = (n) => {
    step(watched, n);
    draw(watched.p);
  };
  const tell = (m) => parent.postMessage(m, location.origin);
  addEventListener("message", (e) => {
    const m = e.data;
    if (e.origin !== location.origin || !m || typeof m !== "object" || typeof m.gp !== "string") return;
    try {
      if (m.gp === "frames" || m.gp === "load-state") {
        start = m.gp === "load-state" ? Uint8Array.from(atob(m.state), (c) => c.charCodeAt(0)) : null;
        steps = parseFrames(m.text);
        reset(watched);
        reset(still);
        seek(m.frame ?? 0);
      } else if (m.gp === "seek") {
        seek(m.frame);
      } else if (m.gp === "push") {
        for (const x of parseFrames(m.text)) steps.push(x);
        seek(steps.length);
      } else if (m.gp === "bundle" || m.gp === "profile") {
        // The model changed (the refinement loop wrote new numbers into
        // the run's bundle), or another physics profile was chosen: load
        // it and show the same frame again, the same steps from the same
        // start.
        const next = m.gp === "profile" ? String(m.name ?? "retro") : profile;
        fetchBundle(next, watched.p)
          .then((b) => {
            bundle = b;
            profile = next;
            const at = watched.at;
            reset(watched);
            reset(still);
            seek(at);
            tell(m.gp === "profile" ? { gp: "profile", name: profile, frame: watched.at } : { gp: "bundle", frame: watched.at });
          })
          .catch((err) => tell({ gp: "error", message: String(err) }));
        return;
      } else if (m.gp === "theme") {
        // The parent picks a look: drawn from the next frame, no restart.
        st?.looks?.apply(String(m.id)).then(() => {
          draw(watched.p);
          tell({ gp: "theme", id: st.looks.current });
        });
        return;
      } else if (m.gp === "settings") {
        // The parent opens the screen and hands it keys; on a close it
        // keeps the setting and starts the rebuild again in it.
        if (!st) throw new Error("this preview's gp_web.wasm has no settings screen; rebuild it");
        const code = SETTINGS_NAMES.indexOf(String(m.key));
        if (code < 0) return;
        if (!st.open) {
          if (code !== 6) return;
          st.show(typeof m.profile === "string" ? m.profile : profile);
          tell({ gp: "settings", open: true });
        } else {
          const r = st.key(code);
          if (r?.closed) {
            if (r.theme && r.theme !== st.looks?.current) void st.looks?.apply(r.theme);
            tell({ gp: "settings", open: false, profile: r.profile, restart: r.restart, theme: r.theme });
          }
        }
        draw(watched.p);
        return;
      } else if (m.gp === "snap") {
        step(still, m.frame);
        draw(still.p);
        const url = canvas.toDataURL();
        draw(watched.p);
        tell({ gp: "snap", id: m.id, url });
        return;
      } else return;
      tell({ gp: "at", frame: watched.at, of: steps.length });
    } catch (err) {
      t.error = String(err?.stack ?? err);
      tell({ gp: "error", message: String(err) });
    }
  });
  draw();
  mark("first-frame");
  tell({ gp: "ready" });
}

main().catch((e) => {
  t.error = String(e?.stack ?? e);
  console.error(e);
  // On the page too, so a refused model (one naming ext hooks this module
  // has no crate for) never looks like a game that just does less.
  showError(`The preview didn't start: ${e?.message ?? e}`);
});
