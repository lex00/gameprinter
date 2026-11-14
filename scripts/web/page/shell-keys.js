// The runtime shell's keys and gamepad buttons and its policy, as every
// target reads them (docs/shell.md): generated from gp_present::shell by
// `gp self shell`. Don't edit it; change the table and run `just docs`.
//
// A key is its KeyboardEvent.code; `standard` is each button's index in
// the browser's standard gamepad layout. `on` says which targets have a
// binding (both or web); the preview reads the ones it has. `needs`
// says what of the shell's settings it needs: always, not-console-only (off
// while Console only is on) or quick-buttons (Quick save buttons on).

export const BINDINGS = [
  { action: "a", keys: ["KeyX", "Space"], shift: false, buttons: ["South"], standard: [0], on: "both", when: "play", needs: "always" },
  { action: "b", keys: ["KeyZ", "ShiftLeft"], shift: false, buttons: ["West"], standard: [2], on: "both", when: "play", needs: "always" },
  { action: "b", keys: [], shift: false, buttons: ["East"], standard: [1], on: "both", when: "play", needs: "always" },
  { action: "select", keys: ["ShiftRight", "Tab"], shift: false, buttons: ["Select"], standard: [8], on: "both", when: "play", needs: "always" },
  { action: "start", keys: ["Enter"], shift: false, buttons: ["Start"], standard: [9], on: "both", when: "play", needs: "always" },
  { action: "up", keys: ["ArrowUp", "KeyW"], shift: false, buttons: ["DPadUp"], standard: [12], on: "both", when: "play", needs: "always" },
  { action: "down", keys: ["ArrowDown", "KeyS"], shift: false, buttons: ["DPadDown"], standard: [13], on: "both", when: "play", needs: "always" },
  { action: "left", keys: ["ArrowLeft", "KeyA"], shift: false, buttons: ["DPadLeft"], standard: [14], on: "both", when: "play", needs: "always" },
  { action: "right", keys: ["ArrowRight", "KeyD"], shift: false, buttons: ["DPadRight"], standard: [15], on: "both", when: "play", needs: "always" },
  { action: "a2", keys: ["KeyM"], shift: false, buttons: [], standard: [], on: "both", when: "play", needs: "always" },
  { action: "b2", keys: ["KeyN"], shift: false, buttons: [], standard: [], on: "both", when: "play", needs: "always" },
  { action: "select2", keys: ["KeyH"], shift: false, buttons: [], standard: [], on: "both", when: "play", needs: "always" },
  { action: "start2", keys: ["KeyY"], shift: false, buttons: [], standard: [], on: "both", when: "play", needs: "always" },
  { action: "up2", keys: ["KeyI"], shift: false, buttons: [], standard: [], on: "both", when: "play", needs: "always" },
  { action: "down2", keys: ["KeyK"], shift: false, buttons: [], standard: [], on: "both", when: "play", needs: "always" },
  { action: "left2", keys: ["KeyJ"], shift: false, buttons: [], standard: [], on: "both", when: "play", needs: "always" },
  { action: "right2", keys: ["KeyL"], shift: false, buttons: [], standard: [], on: "both", when: "play", needs: "always" },
  { action: "reset", keys: ["F5"], shift: false, buttons: [], standard: [], on: "both", when: "play", needs: "always" },
  { action: "settings", keys: ["Escape", "F1"], shift: false, buttons: ["RightThumb"], standard: [11], on: "both", when: "play", needs: "always" },
  { action: "rewind", keys: ["Backspace"], shift: false, buttons: ["LeftTrigger2"], standard: [6], on: "both", when: "play", needs: "not-console-only" },
  { action: "pause", keys: ["F2"], shift: false, buttons: [], standard: [], on: "both", when: "play", needs: "always" },
  { action: "frame", keys: ["F3"], shift: false, buttons: [], standard: [], on: "both", when: "play", needs: "always" },
  { action: "slow", keys: ["F4"], shift: false, buttons: [], standard: [], on: "both", when: "play", needs: "always" },
  { action: "scale", keys: ["F8"], shift: false, buttons: [], standard: [], on: "web", when: "play", needs: "always" },
  { action: "widescreen", keys: ["F9"], shift: false, buttons: [], standard: [], on: "web", when: "play", needs: "always" },
  { action: "dim-flashes", keys: ["F10"], shift: false, buttons: [], standard: [], on: "web", when: "play", needs: "always" },
  { action: "mute-pulse1", keys: ["Digit5"], shift: false, buttons: [], standard: [], on: "web", when: "play", needs: "always" },
  { action: "mute-pulse2", keys: ["Digit6"], shift: false, buttons: [], standard: [], on: "web", when: "play", needs: "always" },
  { action: "mute-triangle", keys: ["Digit7"], shift: false, buttons: [], standard: [], on: "web", when: "play", needs: "always" },
  { action: "mute-noise", keys: ["Digit8"], shift: false, buttons: [], standard: [], on: "web", when: "play", needs: "always" },
  { action: "mute-dmc", keys: ["Digit9"], shift: false, buttons: [], standard: [], on: "web", when: "play", needs: "always" },
  { action: "save-1", keys: ["Digit1"], shift: true, buttons: [], standard: [], on: "web", when: "play", needs: "not-console-only" },
  { action: "save-2", keys: ["Digit2"], shift: true, buttons: [], standard: [], on: "web", when: "play", needs: "not-console-only" },
  { action: "save-3", keys: ["Digit3"], shift: true, buttons: [], standard: [], on: "web", when: "play", needs: "not-console-only" },
  { action: "save-4", keys: ["Digit4"], shift: true, buttons: [], standard: [], on: "web", when: "play", needs: "not-console-only" },
  { action: "load-1", keys: ["Digit1"], shift: false, buttons: [], standard: [], on: "web", when: "play", needs: "not-console-only" },
  { action: "load-2", keys: ["Digit2"], shift: false, buttons: [], standard: [], on: "web", when: "play", needs: "not-console-only" },
  { action: "load-3", keys: ["Digit3"], shift: false, buttons: [], standard: [], on: "web", when: "play", needs: "not-console-only" },
  { action: "load-4", keys: ["Digit4"], shift: false, buttons: [], standard: [], on: "web", when: "play", needs: "not-console-only" },
  { action: "quick-save", keys: [], shift: false, buttons: ["RightTrigger"], standard: [5], on: "web", when: "play", needs: "quick-buttons" },
  { action: "quick-load", keys: [], shift: false, buttons: ["LeftTrigger"], standard: [4], on: "web", when: "play", needs: "quick-buttons" },
  { action: "nav-up", keys: ["ArrowUp", "KeyW"], shift: false, buttons: ["DPadUp"], standard: [12], on: "both", when: "screen", needs: "always" },
  { action: "nav-down", keys: ["ArrowDown", "KeyS"], shift: false, buttons: ["DPadDown"], standard: [13], on: "both", when: "screen", needs: "always" },
  { action: "nav-left", keys: ["ArrowLeft", "KeyA"], shift: false, buttons: ["DPadLeft"], standard: [14], on: "both", when: "screen", needs: "always" },
  { action: "nav-right", keys: ["ArrowRight", "KeyD"], shift: false, buttons: ["DPadRight"], standard: [15], on: "both", when: "screen", needs: "always" },
  { action: "nav-choose", keys: ["Enter", "Space", "KeyX"], shift: false, buttons: ["South"], standard: [0], on: "both", when: "screen", needs: "always" },
  { action: "nav-back", keys: ["Escape", "Backspace"], shift: false, buttons: ["East"], standard: [1], on: "both", when: "screen", needs: "always" },
  { action: "nav-back", keys: ["KeyZ"], shift: false, buttons: [], standard: [], on: "web", when: "screen", needs: "always" },
  { action: "nav-open", keys: ["F1"], shift: false, buttons: ["RightThumb"], standard: [11], on: "both", when: "screen", needs: "always" },
];

// Controller 1's and 2's buttons by bit, A to Right.
export const NES = ["a", "b", "select", "start", "up", "down", "left", "right"];
export const NES2 = ["a2", "b2", "select2", "start2", "up2", "down2", "left2", "right2"];
// The settings screen's keys, in its key numbers' order.
export const SCREEN = ["nav-up", "nav-down", "nav-left", "nav-right", "nav-choose", "nav-back", "nav-open"];

// The rewind ring: a state every `every` frames, `seconds` of them.
export const REWIND = { every: 2, seconds: 20 };
// The slots the number keys reach, where a slot is kept, the stamp of
// when it was saved beside it, and the autosave's slot.
export const QUICK_SLOTS = 4;
export const slotName = (n) => `saves/slot-${n}.gpsave`;
export const whenName = (n) => `saves/slot-${n}.when`;
export const AUTOSAVE_SLOT = 0;
// The shell's own settings, kept through storage as this file
// (gp_present::shell::Settings).
export const SHELL_SETTINGS = "settings/shell.txt";
export const exportName = (game, n) => `${game}-slot${n}.gpsave`;
// The sentences a refused save gets (gp_sim::save).
export const REFUSED = { otherBuild: "Can't load a save from another version", otherGame: "Can't load a save of another game", damaged: "Can't load this save: it is damaged" };
