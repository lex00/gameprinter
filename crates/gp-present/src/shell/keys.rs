//! The shell's keys and gamepad buttons: one table every target reads
//! (docs/shell.md, "Keys and buttons"). The web preview reads it as
//! scripts/web/page/shell-keys.js, which `gp self shell` writes from this
//! table.
//!
//! A key is named by its `KeyboardEvent.code`, (`KeyX`, `Digit1`, `F5`); a button by its
//! `GamepadButton` name, which [`STANDARD_BUTTONS`] places in the
//! browser's standard gamepad layout. A binding says whether the web
//! preview alone has it or every target ([`On`]). A binding
//! also says what it needs of the shell's settings ([`Needs`]).

/// Which targets have a binding.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum On {
    Both,
    /// The web preview only.
    Web,
}

impl On {
    pub fn name(self) -> &'static str {
        match self {
            On::Both => "both",
            On::Web => "web",
        }
    }

    /// Whether a target has it.
    pub fn has(self, target: On) -> bool {
        self == On::Both || self == target
    }
}

/// When a binding acts.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum When {
    /// While the game plays.
    Play,
    /// While a screen of the shell (the settings screen, the pause menu)
    /// is open and the game holds.
    Screen,
}

impl When {
    pub fn name(self) -> &'static str {
        match self {
            When::Play => "play",
            When::Screen => "screen",
        }
    }
}

/// What a binding needs of the shell's settings
/// (`super::slots::Settings::allows`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Needs {
    /// It always acts.
    Always,
    /// Off while "Console only" is on: saves, loads and rewind.
    NotConsoleOnly,
    /// Only with "Quick save buttons" on (and "Console only" off).
    QuickButtons,
}

impl Needs {
    pub fn name(self) -> &'static str {
        match self {
            Needs::Always => "always",
            Needs::NotConsoleOnly => "not-console-only",
            Needs::QuickButtons => "quick-buttons",
        }
    }
}

/// One action's keys and buttons.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Binding {
    /// What it does, such as `a` (controller 1's A), `a2` (controller 2's),
    /// `rewind` or `save-1`.
    pub action: &'static str,
    pub keys: &'static [&'static str],
    /// The keys act with Shift held (and not without it).
    pub shift: bool,
    pub buttons: &'static [&'static str],
    pub on: On,
    pub when: When,
    pub needs: Needs,
}

const fn b(
    action: &'static str,
    keys: &'static [&'static str],
    buttons: &'static [&'static str],
    on: On,
    when: When,
) -> Binding {
    Binding {
        action,
        keys,
        shift: false,
        buttons,
        on,
        when,
        needs: Needs::Always,
    }
}

const fn shifted(action: &'static str, keys: &'static [&'static str]) -> Binding {
    Binding {
        action,
        keys,
        shift: true,
        buttons: &[],
        on: On::Web,
        when: When::Play,
        needs: Needs::NotConsoleOnly,
    }
}

/// A binding that saves, loads or rewinds: off while "Console only" is on.
const fn extra(b: Binding) -> Binding {
    Binding {
        needs: Needs::NotConsoleOnly,
        ..b
    }
}

/// A quick save or load button: only with "Quick save buttons" on.
const fn quick(action: &'static str, buttons: &'static [&'static str]) -> Binding {
    Binding {
        action,
        keys: &[],
        shift: false,
        buttons,
        on: On::Web,
        when: When::Play,
        needs: Needs::QuickButtons,
    }
}

use On::{Both, Web};
use When::{Play, Screen};

/// Controller 1's buttons in the bits an input log carries, A, B, Select,
/// Start, Up, Down, Left, Right from bit 0, by action.
pub const NES: [&str; 8] = ["a", "b", "select", "start", "up", "down", "left", "right"];

/// Controller 2's, in the same order.
pub const NES2: [&str; 8] = [
    "a2", "b2", "select2", "start2", "up2", "down2", "left2", "right2",
];

/// The settings screen's keys, in the order its key numbers take them
/// (gp-web's `gp_settings_key`): up, down, left, right, choose, back, and
/// the settings key itself.
pub const SCREEN: [&str; 7] = [
    "nav-up",
    "nav-down",
    "nav-left",
    "nav-right",
    "nav-choose",
    "nav-back",
    "nav-open",
];

/// Every binding.
pub const BINDINGS: &[Binding] = &[
    // Controller 1. A gamepad's left stick is its D-pad too
    // (`crate::pad`).
    b("a", &["KeyX", "Space"], &["South"], Both, Play),
    b("b", &["KeyZ", "ShiftLeft"], &["West"], Both, Play),
    b("b", &[], &["East"], Both, Play),
    b("select", &["ShiftRight", "Tab"], &["Select"], Both, Play),
    b("start", &["Enter"], &["Start"], Both, Play),
    b("up", &["ArrowUp", "KeyW"], &["DPadUp"], Both, Play),
    b("down", &["ArrowDown", "KeyS"], &["DPadDown"], Both, Play),
    b("left", &["ArrowLeft", "KeyA"], &["DPadLeft"], Both, Play),
    b("right", &["ArrowRight", "KeyD"], &["DPadRight"], Both, Play),
    // Controller 2 from the keyboard; a second gamepad is controller 2
    // with controller 1's buttons.
    b("a2", &["KeyM"], &[], Both, Play),
    b("b2", &["KeyN"], &[], Both, Play),
    b("select2", &["KeyH"], &[], Both, Play),
    b("start2", &["KeyY"], &[], Both, Play),
    b("up2", &["KeyI"], &[], Both, Play),
    b("down2", &["KeyK"], &[], Both, Play),
    b("left2", &["KeyJ"], &[], Both, Play),
    b("right2", &["KeyL"], &[], Both, Play),
    // The shell.
    b("reset", &["F5"], &[], Both, Play),
    b("settings", &["Escape", "F1"], &["RightThumb"], Both, Play),
    extra(b("rewind", &["Backspace"], &["LeftTrigger2"], Both, Play)),
    b("pause", &["F2"], &[], Both, Play),
    b("frame", &["F3"], &[], Both, Play),
    b("slow", &["F4"], &[], Both, Play),
    b("scale", &["F8"], &[], Web, Play),
    b("widescreen", &["F9"], &[], Web, Play),
    b("dim-flashes", &["F10"], &[], Web, Play),
    b("mute-pulse1", &["Digit5"], &[], Web, Play),
    b("mute-pulse2", &["Digit6"], &[], Web, Play),
    b("mute-triangle", &["Digit7"], &[], Web, Play),
    b("mute-noise", &["Digit8"], &[], Web, Play),
    b("mute-dmc", &["Digit9"], &[], Web, Play),
    shifted("save-1", &["Digit1"]),
    shifted("save-2", &["Digit2"]),
    shifted("save-3", &["Digit3"]),
    shifted("save-4", &["Digit4"]),
    extra(b("load-1", &["Digit1"], &[], Web, Play)),
    extra(b("load-2", &["Digit2"], &[], Web, Play)),
    extra(b("load-3", &["Digit3"], &[], Web, Play)),
    extra(b("load-4", &["Digit4"], &[], Web, Play)),
    // Quick save and quick load to the last slot used, on the bumpers
    // (LB loads, RB saves), behind the "Quick save buttons" switch.
    quick("quick-save", &["RightTrigger"]),
    quick("quick-load", &["LeftTrigger"]),
    // The shell's screens.
    b("nav-up", &["ArrowUp", "KeyW"], &["DPadUp"], Both, Screen),
    b(
        "nav-down",
        &["ArrowDown", "KeyS"],
        &["DPadDown"],
        Both,
        Screen,
    ),
    b(
        "nav-left",
        &["ArrowLeft", "KeyA"],
        &["DPadLeft"],
        Both,
        Screen,
    ),
    b(
        "nav-right",
        &["ArrowRight", "KeyD"],
        &["DPadRight"],
        Both,
        Screen,
    ),
    b(
        "nav-choose",
        &["Enter", "Space", "KeyX"],
        &["South"],
        Both,
        Screen,
    ),
    b(
        "nav-back",
        &["Escape", "Backspace"],
        &["East"],
        Both,
        Screen,
    ),
    b("nav-back", &["KeyZ"], &[], Web, Screen),
    b("nav-open", &["F1"], &["RightThumb"], Both, Screen),
];

/// The browser's standard gamepad layout: The name for each of its
/// buttons, by index.
pub const STANDARD_BUTTONS: [&str; 17] = [
    "South",
    "East",
    "West",
    "North",
    "LeftTrigger",
    "RightTrigger",
    "LeftTrigger2",
    "RightTrigger2",
    "Select",
    "Start",
    "LeftThumb",
    "RightThumb",
    "DPadUp",
    "DPadDown",
    "DPadLeft",
    "DPadRight",
    "Mode",
];

/// A button's index in the standard layout.
pub fn standard_index(button: &str) -> Option<usize> {
    STANDARD_BUTTONS.iter().position(|b| *b == button)
}

/// The bindings a target has.
pub fn of(target: On) -> impl Iterator<Item = &'static Binding> {
    BINDINGS.iter().filter(move |b| b.on.has(target))
}

/// The keys that do `action` in `target`, without Shift.
pub fn keys(action: &str, target: On) -> Vec<&'static str> {
    of(target)
        .filter(|b| b.action == action && !b.shift)
        .flat_map(|b| b.keys.iter().copied())
        .collect()
}

/// The gamepad buttons that do `action` in `target`.
pub fn buttons(action: &str, target: On) -> Vec<&'static str> {
    of(target)
        .filter(|b| b.action == action)
        .flat_map(|b| b.buttons.iter().copied())
        .collect()
}

/// Two bindings that clash: the same key (with Shift alike) or button on
/// two actions of one target at the same time. Empty for the standard.
pub fn clashes() -> Vec<String> {
    let mut out = Vec::new();
    for target in [Web] {
        for when in [Play, Screen] {
            let mut seen: Vec<(String, &str)> = Vec::new();
            for b in of(target).filter(|b| b.when == when) {
                let names = b
                    .keys
                    .iter()
                    .map(|k| format!("{}{k}", if b.shift { "Shift+" } else { "" }))
                    .chain(b.buttons.iter().map(|x| format!("button {x}")));
                for name in names {
                    match seen.iter().find(|(n, _)| *n == name) {
                        Some((_, a)) if *a != b.action => out.push(format!(
                            "{} {}: {name} is on {a} and {}",
                            target.name(),
                            when.name(),
                            b.action
                        )),
                        Some(_) => {}
                        None => seen.push((name, b.action)),
                    }
                }
            }
        }
    }
    out
}
