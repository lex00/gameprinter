//! The standard in-game settings screen (docs/settings.md): the modes
//! (`retro`, `normal` and the template's other presets) and every switch of
//! the model's `profiles.toml`, by group, each with its description and an
//! on/off toggle. Both runtimes draw it with this code: the generated Bevy
//! game (its shell's menu) and the interpreter preview (gp-web's
//! `gp_settings_*` exports), so it looks and works the same in each.
//!
//! The screen is presentation. It never reads or writes the simulation:
//! closing it gives the profile chosen, by name, and the runtime starts the
//! game again from power-on in it, applying the switches to the model
//! before it runs. Its pictures are whole pixels in a built-in 5 by 7 font
//! at the game's own resolution, so the screen needs no font file.

use std::collections::{BTreeMap, BTreeSet};

use crate::shell::{self, SlotLine};

/// The model as written, every switch off (`gp_model::profile::BASE`).
pub const RETRO: &str = "retro";

/// The standard preset beside retro (`gp_model::profile::NORMAL`).
pub const NORMAL: &str = "normal";

/// The keyboard key that opens and closes the screen in every game, by
/// its `KeyboardEvent.code` and Bevy `KeyCode` name. On a Mac F1 is a
/// brightness key unless fn is held, so Escape is the main key.
pub const OPEN_KEY_MAIN: &str = "Escape";

/// An extra keyboard key that does the same.
pub const OPEN_KEY: &str = "F1";

/// The gamepad button that opens and closes it: the right stick's press,
/// Bevy's `RightThumb`, button 11 of the browser's standard layout. No NES
/// button and no shell action is on it.
pub const OPEN_BUTTON: &str = "RightThumb";

/// The same button in the browser's standard gamepad layout.
pub const OPEN_BUTTON_STANDARD: usize = 11;

/// One switch as the screen lists it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Switch {
    pub name: String,
    pub description: String,
}

/// A group of switches, under its heading.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Group {
    pub title: String,
    pub description: String,
    pub switches: Vec<Switch>,
}

/// A preset: a mode by name, with its switches.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Preset {
    pub name: String,
    pub description: String,
    pub switches: Vec<String>,
}

/// What a game's settings are: its groups and presets, from
/// `profiles.toml`. Retro is always there and is never listed.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Catalog {
    pub groups: Vec<Group>,
    pub presets: Vec<Preset>,
}

impl Catalog {
    /// The modes in the order the screen steps through them: retro,
    /// normal, then the other presets by name.
    pub fn modes(&self) -> Vec<&str> {
        let mut out = vec![RETRO];
        if self.presets.iter().any(|p| p.name == NORMAL) {
            out.push(NORMAL);
        }
        let mut rest: Vec<&str> = self
            .presets
            .iter()
            .map(|p| p.name.as_str())
            .filter(|n| *n != NORMAL)
            .collect();
        rest.sort_unstable();
        out.extend(rest);
        out
    }

    fn preset(&self, name: &str) -> Option<&Preset> {
        self.presets.iter().find(|p| p.name == name)
    }

    fn has_switch(&self, name: &str) -> bool {
        self.groups
            .iter()
            .any(|g| g.switches.iter().any(|s| s.name == name))
    }

    /// The switches a profile's name turns on: none for retro, a preset's,
    /// a switch, or names joined by `+`. A name the game hasn't is left
    /// out, so a kept setting from an older build plays what it can.
    pub fn switches_of(&self, profile: &str) -> BTreeSet<String> {
        let mut on = BTreeSet::new();
        if profile == RETRO || profile.is_empty() {
            return on;
        }
        for part in profile.split('+') {
            if let Some(p) = self.preset(part) {
                on.extend(p.switches.iter().cloned());
            } else if self.has_switch(part) {
                on.insert(part.to_owned());
            }
        }
        on
    }

    /// The mode a set of switches is: the one named first when it fits,
    /// else normal, else another preset with exactly these switches, else
    /// retro for none; None for a set no mode names (a custom set).
    pub fn mode_of(&self, on: &BTreeSet<String>, first: Option<&str>) -> Option<String> {
        let fits = |m: &str| -> bool {
            if m == RETRO {
                return on.is_empty();
            }
            self.preset(m)
                .is_some_and(|p| p.switches.iter().cloned().collect::<BTreeSet<_>>() == *on)
        };
        if let Some(f) = first.filter(|f| fits(f)) {
            return Some(f.to_owned());
        }
        self.modes()
            .into_iter()
            .find(|m| fits(m))
            .map(str::to_owned)
    }
}

/// A key the screen reads, from the keyboard or a controller.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Key {
    Up,
    Down,
    Left,
    Right,
    /// Enter, Space, X or the controller's A.
    Choose,
    /// Escape, Backspace or the controller's B: closes, as "Done" does.
    Back,
    /// The settings key or button itself: closes too.
    Open,
}

/// What the player asked of a save slot on the Saves line. The host does
/// it through its storage and tells the screen how it went
/// ([`Screen::say`], [`Screen::set_saves`]); a load that worked closes the
/// screen ([`Screen::close_loaded`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SlotAction {
    Save(usize),
    Load(usize),
    Delete(usize),
    /// Hand the slot to the player as a file
    /// (`shell::export_name`).
    Download(usize),
    /// Load a save from a file the player picks.
    LoadFile,
}

/// What a key did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    /// Nothing changed.
    Nothing,
    /// The screen changed: draw it again.
    Redraw,
    /// The screen closed. `profile` is the setting to keep (a mode's name,
    /// or the switches on joined by `+`), and `restart` says whether the
    /// switches on differ from the ones the game runs in, so the game
    /// starts again from power-on in them. `theme` is the look chosen, by
    /// id, when the screen lists looks: the game draws the next frame in
    /// it, with no restart, since the simulation never reads it.
    Closed {
        profile: String,
        restart: bool,
        theme: Option<String>,
    },
    /// A save slot asked for: the host does it, and the screen stays open.
    Slot(SlotAction),
}

/// A line of the screen.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Row {
    Mode,
    /// The look (theme) the game draws in, when there is more than one.
    Theme,
    /// Under the Theme line, for a published theme: its author and
    /// whether it is downloaded. The cursor passes over it.
    ThemeAbout,
    /// Removes the downloaded theme at the Theme line from the cache.
    ThemeRemove,
    /// The shell's "Console only" switch: no saves, loads or rewind.
    ConsoleOnly,
    /// The shell's "Quick save buttons" switch.
    QuickButtons,
    /// The Saves heading, which the cursor passes over.
    Saves,
    /// Under the heading while "Console only" is on.
    SavesOff,
    /// A save slot: 0 is the autosave ("Resume"), shown once it is kept.
    Slot(usize),
    /// Load a save from a file.
    LoadFile,
    /// The slot chosen, over what can be done with it.
    AskTitle(usize),
    /// One thing to do with the slot chosen.
    Ask(usize, AskItem),
    /// Back to the list from the slot chosen.
    AskBack,
    /// A group's heading, which the cursor passes over.
    Heading(usize),
    /// A switch, by group and place.
    Switch(usize, usize),
    /// Close and play: in the new settings when they changed.
    Done,
    /// Close and keep the settings the game runs in.
    Cancel,
}

/// What can be done with a slot once chosen.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AskItem {
    Save,
    Load,
    Download,
    Delete,
}

/// The save slots as the host read them for the Saves line.
#[derive(Debug, Clone, PartialEq)]
pub struct Saves {
    /// The autosave (slot 0) and the player's slots, from
    /// `shell::slots::slot_lines`.
    pub lines: Vec<SlotLine>,
    /// The host's clock, in seconds since 1970, for "5 min ago"; None for
    /// a host with none.
    pub now: Option<u64>,
    /// Frames a second, for the play time.
    pub rate: f64,
    /// Whether the host can load a save from a file.
    pub files: bool,
}

impl Saves {
    fn line(&self, slot: usize) -> Option<&SlotLine> {
        self.lines.iter().find(|l| l.slot == slot)
    }
}

/// The screen while it's open.
#[derive(Debug, Clone, PartialEq)]
pub struct Screen {
    pub catalog: Catalog,
    /// The switches the game runs in, as the screen opened.
    pub running: BTreeSet<String>,
    /// The mode the game runs in, as the screen opened (None for a custom
    /// set).
    pub running_mode: Option<String>,
    /// The switches chosen.
    pub on: BTreeSet<String>,
    /// The mode chosen, None for a custom set.
    pub mode: Option<String>,
    /// The row at the cursor, an index of [`Screen::rows`].
    pub cursor: usize,
    /// The looks the game can draw in, by id and title
    /// (`gp_present::looks::choices`): the build's own first.
    pub themes: Vec<(String, String)>,
    /// The look chosen and the one shown as the screen opened, as indexes
    /// of `themes`.
    pub theme: usize,
    pub running_theme: usize,
    /// The published themes among `themes`, by id: who made each and
    /// whether it is downloaded (docs/theme-publishing.md, "In the game").
    pub marks: BTreeMap<String, ThemeMark>,
    /// The themes the player removed from the cache while the screen was
    /// open, by id, for the host to delete and take.
    pub removed: Vec<String>,
    /// The Saves line, when the host lists its slots.
    pub saves: Option<Saves>,
    /// The shell's settings chosen and the ones the game runs in, when the
    /// host offers them.
    pub shell: Option<shell::Settings>,
    pub running_shell: Option<shell::Settings>,
    /// The slot chosen, while the screen asks what to do with it.
    pub asking: Option<usize>,
    /// What the host said of the last thing done, shown under the list
    /// until the cursor moves.
    pub said: Option<String>,
}

/// Where a published theme's files are.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kept {
    /// In the cache: it works offline.
    Downloaded,
    /// On a mirror only: choosing it downloads it.
    Download,
}

/// A published theme on the Theme line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ThemeMark {
    pub author: String,
    pub kept: Kept,
    /// Its package's size in bytes.
    pub size: u64,
}

/// A size in bytes as people read it.
pub fn size_text(n: u64) -> String {
    if n >= 1 << 20 {
        format!("{:.1} MB", n as f64 / f64::from(1u32 << 20))
    } else {
        format!("{} KB", n.div_ceil(1024))
    }
}

impl Screen {
    /// The screen over a game running in `profile` (the kept setting's
    /// name).
    pub fn open(catalog: Catalog, profile: &str) -> Screen {
        let on = catalog.switches_of(profile);
        let first = (!profile.contains('+')).then_some(profile);
        let mode = catalog.mode_of(&on, first);
        Screen {
            catalog,
            running: on.clone(),
            running_mode: mode.clone(),
            on,
            mode,
            cursor: 0,
            themes: Vec::new(),
            theme: 0,
            running_theme: 0,
            marks: BTreeMap::new(),
            removed: Vec::new(),
            saves: None,
            shell: None,
            running_shell: None,
            asking: None,
            said: None,
        }
    }

    /// The screen with the shell's own settings, as the game runs in them.
    pub fn with_shell(mut self, settings: shell::Settings) -> Screen {
        self.shell = Some(settings);
        self.running_shell = Some(settings);
        self
    }

    /// The screen with a Saves line listing these slots.
    pub fn with_saves(mut self, saves: Saves) -> Screen {
        self.set_saves(saves);
        self
    }

    /// Lists the slots again (after a save, a delete, a load), keeping the
    /// cursor on its line.
    pub fn set_saves(&mut self, saves: Saves) {
        let rows = self.rows();
        let at = rows.get(self.cursor).copied();
        self.saves = Some(saves);
        if let Some(row) = at {
            self.cursor_to(row);
        }
    }

    /// Shows a sentence under the list, in place of the cursor's line
    /// explained, until the cursor moves: what the host did.
    pub fn say(&mut self, text: impl Into<String>) {
        self.said = Some(text.into());
    }

    /// Closes the screen after a load from it: the game goes on from the
    /// save, so the switches it runs in are kept (a restart would undo the
    /// load), while the look and the shell's settings chosen stay chosen.
    pub fn close_loaded(&mut self) -> Outcome {
        self.on = self.running.clone();
        self.mode = self.running_mode.clone();
        self.close(true)
    }

    fn cursor_to(&mut self, row: Row) {
        if let Some(i) = self.rows().iter().position(|r| *r == row) {
            self.cursor = i;
        }
    }

    /// The screen with the published themes of its Theme line marked:
    /// each one's author and whether it is downloaded.
    pub fn with_marks(mut self, marks: BTreeMap<String, ThemeMark>) -> Screen {
        self.marks = marks;
        self
    }

    /// The mark of the theme at the Theme line, when it is a published one.
    pub fn mark(&self) -> Option<&ThemeMark> {
        self.theme_id().and_then(|id| self.marks.get(id))
    }

    /// The screen with a Theme line listing `themes` (id, title), at the
    /// look shown, `current`. One look or none shows no line.
    pub fn with_themes(mut self, themes: Vec<(String, String)>, current: &str) -> Screen {
        self.theme = themes.iter().position(|(id, _)| id == current).unwrap_or(0);
        self.running_theme = self.theme;
        self.themes = themes;
        self
    }

    /// The look chosen, by id.
    pub fn theme_id(&self) -> Option<&str> {
        self.themes.get(self.theme).map(|(id, _)| id.as_str())
    }

    /// Whether the choice differs from what the game runs in.
    pub fn changed(&self) -> bool {
        self.on != self.running
            || self.mode != self.running_mode
            || self.theme != self.running_theme
            || self.shell != self.running_shell
    }

    fn console_only(&self) -> bool {
        self.shell.is_some_and(|s| s.console_only)
    }

    fn line(&self, slot: usize) -> Option<&SlotLine> {
        self.saves.as_ref().and_then(|s| s.line(slot))
    }

    fn kept(&self, slot: usize) -> bool {
        self.line(slot)
            .is_some_and(|l| l.save.is_some() || l.damaged)
    }

    /// The lines, in order: the mode, each group's heading and switches,
    /// then Done (and Cancel once something changed).
    pub fn rows(&self) -> Vec<Row> {
        if let Some(n) = self.asking {
            let mut out = vec![Row::AskTitle(n)];
            let line = self.line(n);
            let readable = line.is_some_and(|l| l.save.is_some());
            if n != shell::AUTOSAVE_SLOT {
                out.push(Row::Ask(n, AskItem::Save));
            }
            if readable {
                out.push(Row::Ask(n, AskItem::Load));
            }
            if self.kept(n) {
                out.push(Row::Ask(n, AskItem::Download));
                out.push(Row::Ask(n, AskItem::Delete));
            }
            out.push(Row::AskBack);
            return out;
        }
        let mut out = vec![Row::Mode];
        if self.themes.len() > 1 {
            out.push(Row::Theme);
            if let Some(m) = self.mark() {
                out.push(Row::ThemeAbout);
                if m.kept == Kept::Downloaded {
                    out.push(Row::ThemeRemove);
                }
            }
        }
        if self.shell.is_some() {
            out.push(Row::ConsoleOnly);
            out.push(Row::QuickButtons);
        }
        if let Some(saves) = &self.saves {
            out.push(Row::Saves);
            if self.console_only() {
                out.push(Row::SavesOff);
            } else {
                if self.kept(shell::AUTOSAVE_SLOT) {
                    out.push(Row::Slot(shell::AUTOSAVE_SLOT));
                }
                out.extend((1..=shell::QUICK_SLOTS).map(Row::Slot));
                if saves.files {
                    out.push(Row::LoadFile);
                }
            }
        }
        for (g, group) in self.catalog.groups.iter().enumerate() {
            out.push(Row::Heading(g));
            out.extend((0..group.switches.len()).map(|i| Row::Switch(g, i)));
        }
        out.push(Row::Done);
        if self.changed() {
            out.push(Row::Cancel);
        }
        out
    }

    /// The setting to keep: the mode's name, or the switches joined by `+`.
    pub fn profile(&self) -> String {
        match &self.mode {
            Some(m) => m.clone(),
            None => self.on.iter().cloned().collect::<Vec<_>>().join("+"),
        }
    }

    fn mode_label(&self) -> String {
        self.mode.clone().unwrap_or_else(|| "custom".to_owned())
    }

    /// A row's text.
    pub fn text(&self, row: Row) -> String {
        match row {
            Row::Mode => format!("Mode      < {} >", self.mode_label()),
            Row::Theme => format!(
                "Theme     < {} >",
                self.themes.get(self.theme).map_or("", |(_, t)| t.as_str())
            ),
            Row::ThemeAbout => {
                let m = self.mark();
                let by = m
                    .filter(|m| !m.author.is_empty())
                    .map(|m| format!("by {}, ", m.author))
                    .unwrap_or_default();
                match m {
                    Some(m) if m.kept == Kept::Downloaded => format!("  {by}downloaded"),
                    Some(m) => format!("  {by}download {}", size_text(m.size)),
                    None => String::new(),
                }
            }
            Row::ThemeRemove => "  Remove it from the cache".to_owned(),
            Row::ConsoleOnly => format!("{} Console only", mark(self.console_only())),
            Row::QuickButtons => format!(
                "{} Quick save buttons",
                mark(self.shell.is_some_and(|s| s.quick_buttons))
            ),
            Row::Saves => "Saves".to_owned(),
            Row::SavesOff => "  Off: console only is on".to_owned(),
            Row::Slot(n) => self.slot_text(n),
            Row::LoadFile => "Load from a file".to_owned(),
            Row::AskTitle(n) => self
                .line(n)
                .map_or_else(|| SlotLine::empty(n).label(), SlotLine::label),
            Row::Ask(n, AskItem::Save) if self.kept(n) => "Save here, replacing it".to_owned(),
            Row::Ask(_, AskItem::Save) => "Save here".to_owned(),
            Row::Ask(_, AskItem::Load) => "Load".to_owned(),
            Row::Ask(_, AskItem::Download) => "Download as a file".to_owned(),
            Row::Ask(_, AskItem::Delete) => "Delete".to_owned(),
            Row::AskBack => "Back".to_owned(),
            Row::Heading(g) => self.catalog.groups[g].title.clone(),
            Row::Switch(g, i) => {
                let s = &self.catalog.groups[g].switches[i];
                format!(
                    "{} {}",
                    mark(self.on.contains(&s.name)),
                    s.name.replace('_', " ")
                )
            }
            Row::Done if self.on != self.running => "Done: restart in these settings".to_owned(),
            Row::Done => "Done".to_owned(),
            Row::Cancel => "Cancel the changes".to_owned(),
        }
    }

    /// What the row at the cursor is, in plain words, shown under the list.
    pub fn detail(&self, row: Row) -> String {
        match row {
            Row::Mode => match self.mode.as_deref() {
                Some(RETRO) => {
                    "Retro: the game as the console runs it, frame for frame, every switch off"
                        .to_owned()
                }
                Some(m) => match self.catalog.preset(m) {
                    Some(p) if !p.description.is_empty() => {
                        format!("{}: {}", title_case(m), p.description)
                    }
                    _ if m == NORMAL => {
                        "Normal: no lag and none of the console's quirks".to_owned()
                    }
                    _ => format!("{}: {} switches on", title_case(m), self.on.len()),
                },
                None => format!(
                    "Your own set: {} switches on. Left and right pick a mode",
                    self.on.len()
                ),
            },
            Row::Theme | Row::ThemeAbout | Row::ThemeRemove if self.mark().is_some() => {
                let title = self.themes.get(self.theme).map_or("", |(_, t)| t.as_str());
                let m = self.mark().cloned().unwrap_or(ThemeMark {
                    author: String::new(),
                    kept: Kept::Download,
                    size: 0,
                });
                let by = if m.author.is_empty() {
                    String::new()
                } else {
                    format!(" by {}", m.author)
                };
                match (row, m.kept) {
                    (Row::ThemeRemove, _) => format!(
                        "Removes {title} from this game's cache. It can be downloaded again"
                    ),
                    (_, Kept::Downloaded) => format!(
                        "{title}{by}: downloaded, so it works offline. A theme changes at once, with no restart"
                    ),
                    (_, Kept::Download) => format!(
                        "{title}{by}: not downloaded yet. Done downloads it ({}), checks every file and draws it at once, with no restart",
                        size_text(m.size)
                    ),
                }
            }
            Row::ThemeAbout | Row::ThemeRemove => String::new(),
            Row::ConsoleOnly => "Console only: no saves, loads or rewind, as on the console. The game doesn't start again".to_owned(),
            Row::QuickButtons => "Quick save buttons: on a controller, the right bumper saves and the left bumper loads, to the last slot used (slot 1 at first). Off at first, so a stray press can't replace a save".to_owned(),
            Row::Saves | Row::SavesOff => "Saves are off while Console only is on".to_owned(),
            Row::Slot(n) => self.slot_detail(n),
            Row::LoadFile => "Loads a save from a file you downloaded from this game before".to_owned(),
            Row::AskTitle(_) | Row::AskBack => "Back to the list of saves".to_owned(),
            Row::Ask(n, item) => {
                let name = self.text(Row::AskTitle(n));
                match item {
                    AskItem::Save => format!("Saves the game where it is as {name}"),
                    AskItem::Load => format!("Loads {name} and goes back to the game"),
                    AskItem::Download => format!("Downloads {name} as a file, to keep or to load on another computer"),
                    AskItem::Delete => format!("Deletes {name}. It can't be brought back"),
                }
            }
            Row::Theme => match self.theme_id() {
                Some(crate::looks::ORIGINAL) => {
                    "Original: the game's own pictures and sound, from your ROM. A theme changes at once, with no restart".to_owned()
                }
                _ => "The look: pictures and sound. A theme changes at once, with no restart, since the game never reads it".to_owned(),
            },
            Row::Heading(g) => self.catalog.groups[g].description.clone(),
            Row::Switch(g, i) => self.catalog.groups[g].switches[i].description.clone(),
            Row::Done if self.on != self.running => {
                "The game starts again from power-on in the new settings".to_owned()
            }
            Row::Done => "Back to the game".to_owned(),
            Row::Cancel => "Back to the game in the settings it runs in".to_owned(),
        }
    }

    fn selectable(row: Row) -> bool {
        !matches!(
            row,
            Row::Heading(_) | Row::ThemeAbout | Row::Saves | Row::SavesOff | Row::AskTitle(_)
        )
    }

    /// A slot's line: its name, then its play time, mode and when it was
    /// saved, or that it is empty.
    fn slot_text(&self, n: usize) -> String {
        let empty = SlotLine::empty(n);
        let line = self.line(n).unwrap_or(&empty);
        let rest = match (&line.save, line.damaged) {
            (_, true) => "can't be read".to_owned(),
            (None, false) => "empty".to_owned(),
            (Some(s), _) if s.refused.is_some() => {
                format!("{}  another version", self.time(s.frame))
            }
            (Some(s), _) => {
                let mut t = format!(
                    "{}  {}",
                    self.time(s.frame),
                    shell::slots::mode_title(&s.profile)
                );
                if let (Some(now), Some(at)) = (self.saves.as_ref().and_then(|x| x.now), s.saved_at)
                {
                    t.push_str("  ");
                    t.push_str(&shell::slots::ago(now, at));
                }
                t
            }
        };
        format!("{:<8}{rest}", line.label())
    }

    fn time(&self, frame: u64) -> String {
        shell::slots::play_time(frame, self.saves.as_ref().map_or(60.0, |s| s.rate))
    }

    /// A slot explained under the list.
    fn slot_detail(&self, n: usize) -> String {
        let empty = SlotLine::empty(n);
        let line = self.line(n).unwrap_or(&empty);
        let name = line.label();
        let resume = n == shell::AUTOSAVE_SLOT;
        match &line.save {
            None if line.damaged => {
                format!("{name} holds no save the game can read. Choose it to delete it")
            }
            None => format!("{name} is empty. Choose it to save the game here"),
            Some(s) => {
                let mut parts = Vec::new();
                if let Some(l) = s.level {
                    parts.push(format!("level {l}"));
                }
                parts.push(format!("{} played", self.time(s.frame)));
                parts.push(format!("in {}", shell::slots::mode_title(&s.profile)));
                if let (Some(now), Some(at)) = (self.saves.as_ref().and_then(|x| x.now), s.saved_at)
                {
                    parts.push(format!("saved {}", shell::slots::ago(now, at)));
                }
                let what = if resume {
                    "Resume: the game as the last level started, saved by itself"
                } else {
                    &name
                };
                match s.refused {
                    Some(why) => format!("{what}: {}. {why}", parts.join(", ")),
                    None if resume => format!("{what}: {}. Choose it to load it", parts.join(", ")),
                    None => format!(
                        "{what}: {}. Choose it to save here, load, download or delete it",
                        parts.join(", ")
                    ),
                }
            }
        }
    }

    fn close(&mut self, keep: bool) -> Outcome {
        self.asking = None;
        if !keep {
            self.shell = self.running_shell;
            let profile = match &self.running_mode {
                Some(m) => m.clone(),
                None => self.running.iter().cloned().collect::<Vec<_>>().join("+"),
            };
            return Outcome::Closed {
                profile,
                restart: false,
                theme: self
                    .themes
                    .get(self.running_theme)
                    .map(|(id, _)| id.clone()),
            };
        }
        Outcome::Closed {
            profile: self.profile(),
            restart: self.on != self.running,
            theme: self.theme_id().map(str::to_owned),
        }
    }

    fn step_mode(&mut self, d: isize) {
        let modes: Vec<String> = self
            .catalog
            .modes()
            .into_iter()
            .map(str::to_owned)
            .collect();
        let at = self
            .mode
            .as_ref()
            .and_then(|m| modes.iter().position(|x| x == m));
        let n = modes.len() as isize;
        let next = match at {
            Some(at) => (at as isize + d).rem_euclid(n) as usize,
            // From a custom set, right goes to the first mode after retro.
            None if d > 0 => 1.min(modes.len() - 1),
            None => 0,
        };
        let m = modes[next].clone();
        self.on = self.catalog.switches_of(&m);
        self.mode = Some(m);
    }

    fn toggle(&mut self, g: usize, i: usize) {
        let name = self.catalog.groups[g].switches[i].name.clone();
        if !self.on.remove(&name) {
            self.on.insert(name);
        }
        self.mode = self.catalog.mode_of(&self.on, None);
    }

    /// Reads a key.
    pub fn key(&mut self, k: Key) -> Outcome {
        let rows = self.rows();
        self.cursor = self.cursor.min(rows.len() - 1);
        let row = rows[self.cursor];
        if let (Some(n), Key::Back) = (self.asking, k) {
            self.asking = None;
            self.cursor_to(Row::Slot(n));
            return Outcome::Redraw;
        }
        match k {
            Key::Up | Key::Down => {
                self.said = None;
                let n = rows.len();
                let mut c = self.cursor;
                loop {
                    c = if k == Key::Up {
                        (c + n - 1) % n
                    } else {
                        (c + 1) % n
                    };
                    if Self::selectable(rows[c]) {
                        break;
                    }
                }
                self.cursor = c;
                Outcome::Redraw
            }
            Key::Back | Key::Open => self.close(true),
            Key::Left | Key::Right | Key::Choose => {
                let d = if k == Key::Left { -1 } else { 1 };
                match row {
                    Row::Mode => self.step_mode(d),
                    Row::Theme => {
                        let n = self.themes.len() as isize;
                        self.theme = (self.theme as isize + d).rem_euclid(n.max(1)) as usize;
                    }
                    Row::Switch(g, i) => self.toggle(g, i),
                    Row::ConsoleOnly => {
                        if let Some(s) = &mut self.shell {
                            s.console_only = !s.console_only;
                        }
                    }
                    Row::QuickButtons => {
                        if let Some(s) = &mut self.shell {
                            s.quick_buttons = !s.quick_buttons;
                        }
                    }
                    Row::Slot(n) if k == Key::Choose => {
                        self.asking = Some(n);
                        self.said = None;
                        self.cursor = 1;
                        return Outcome::Redraw;
                    }
                    Row::LoadFile if k == Key::Choose => {
                        return Outcome::Slot(SlotAction::LoadFile);
                    }
                    Row::Ask(n, item) if k == Key::Choose => {
                        self.asking = None;
                        self.cursor_to(Row::Slot(n));
                        return Outcome::Slot(match item {
                            AskItem::Save => SlotAction::Save(n),
                            AskItem::Load => SlotAction::Load(n),
                            AskItem::Download => SlotAction::Download(n),
                            AskItem::Delete => SlotAction::Delete(n),
                        });
                    }
                    Row::AskBack if k == Key::Choose => {
                        let n = self.asking.take().unwrap_or(1);
                        self.cursor_to(Row::Slot(n));
                        return Outcome::Redraw;
                    }
                    Row::ThemeRemove if k == Key::Choose => {
                        if let Some(id) = self.theme_id().map(str::to_owned)
                            && let Some(m) = self.marks.get_mut(&id)
                        {
                            m.kept = Kept::Download;
                            self.removed.push(id);
                        }
                    }
                    Row::Done if k == Key::Choose => return self.close(true),
                    Row::Cancel if k == Key::Choose => return self.close(false),
                    _ => return Outcome::Nothing,
                }
                // Cancel comes and goes with a change: keep the cursor on
                // a line that is there.
                self.cursor = self.cursor.min(self.rows().len() - 1);
                Outcome::Redraw
            }
        }
    }

    /// The screen as text, for checks and readers: a line each, the
    /// cursor's marked `> `, then `| ` and what is shown under the list.
    pub fn lines(&self) -> Vec<String> {
        let rows = self.rows();
        let cursor = self.cursor.min(rows.len() - 1);
        let mut out: Vec<String> = rows
            .iter()
            .enumerate()
            .map(|(i, r)| format!("{}{}", if i == cursor { "> " } else { "  " }, self.text(*r)))
            .collect();
        out.push(format!(
            "| {}",
            self.said
                .clone()
                .unwrap_or_else(|| self.detail(rows[cursor]))
        ));
        out
    }

    /// The screen as RGBA pixels, `w` by `h`, over the game: a dark
    /// backdrop a little see-through, the title, the lines with the cursor,
    /// the cursor's line explained under them, and the keys.
    pub fn draw(&self, w: usize, h: usize) -> Vec<u8> {
        let mut c = Canvas::new(w, h, BACKDROP);
        c.text(8, 6, "SETTINGS", GOLD);
        let cols = w.saturating_sub(16) / 6;
        let rows = self.rows();
        let cursor = self.cursor.min(rows.len() - 1);
        let about = wrap(
            &self
                .said
                .clone()
                .unwrap_or_else(|| self.detail(rows[cursor])),
            cols,
        );
        let hint_y = h.saturating_sub(10);
        let about_top = hint_y.saturating_sub(4 + about.len() * 9);
        let top = 18;
        let fits = (about_top.saturating_sub(top + 2) / 9).max(1);
        let first = cursor
            .saturating_sub(fits - 1)
            .min(rows.len().saturating_sub(fits));
        for (k, row) in rows.iter().enumerate().skip(first).take(fits) {
            let y = top + (k - first) * 9;
            let text = self.text(*row);
            match row {
                Row::Heading(_) | Row::Saves | Row::AskTitle(_) => {
                    c.text(8, y, &clip(&text, cols), GOLD)
                }
                Row::SavesOff => c.text(16, y, &clip(&text, cols.saturating_sub(2)), DIM),
                Row::ThemeAbout => c.text(16, y, &clip(&text, cols.saturating_sub(2)), DIM),
                _ => {
                    let here = k == cursor;
                    if here {
                        c.text(8, y, ">", GOLD);
                    }
                    c.text(
                        16,
                        y,
                        &clip(&text, cols.saturating_sub(2)),
                        if here { WHITE } else { DIM },
                    );
                }
            }
        }
        for (i, line) in about.iter().enumerate() {
            c.text(8, about_top + i * 9, line, WHITE);
        }
        c.rect(8, about_top.saturating_sub(3), w.saturating_sub(16), 1, DIM);
        let hint = "Arrows move, Enter toggles, Esc closes";
        c.text(8, hint_y, &clip(hint, cols), DIM);
        c.px
    }
}

const BACKDROP: [u8; 4] = [0, 0, 0, 225];
const WHITE: [u8; 4] = [255, 255, 255, 255];
const DIM: [u8; 4] = [150, 150, 150, 255];
const GOLD: [u8; 4] = [255, 210, 80, 255];

fn mark(on: bool) -> &'static str {
    if on { "[X]" } else { "[ ]" }
}

fn title_case(s: &str) -> String {
    let mut c = s.chars();
    match c.next() {
        Some(f) => f.to_uppercase().chain(c).collect(),
        None => String::new(),
    }
}

fn clip(s: &str, n: usize) -> String {
    s.chars().take(n).collect()
}

/// Text in lines of at most `cols` letters, broken at spaces.
pub fn wrap(text: &str, cols: usize) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for word in text.split_whitespace() {
        match out.last_mut() {
            Some(l) if l.chars().count() + 1 + word.chars().count() <= cols => {
                l.push(' ');
                l.push_str(word);
            }
            _ => out.push(clip(word, cols)),
        }
    }
    out
}

/// RGBA pixels to draw on.
pub struct Canvas {
    pub w: usize,
    pub h: usize,
    pub px: Vec<u8>,
}

impl Canvas {
    pub fn new(w: usize, h: usize, fill: [u8; 4]) -> Self {
        Canvas {
            w,
            h,
            px: fill.repeat(w * h),
        }
    }

    pub fn rect(&mut self, x: usize, y: usize, w: usize, h: usize, c: [u8; 4]) {
        for yy in y..(y + h).min(self.h) {
            for xx in x..(x + w).min(self.w) {
                let i = (yy * self.w + xx) * 4;
                self.px[i..i + 4].copy_from_slice(&c);
            }
        }
    }

    /// Text in the 5 by 7 font, six pixels a letter, upper case.
    pub fn text(&mut self, x: usize, y: usize, s: &str, c: [u8; 4]) {
        for (n, ch) in s.chars().enumerate() {
            let rows = glyph(ch.to_ascii_uppercase());
            for (r, bits) in rows.iter().enumerate() {
                for col in 0..5 {
                    if bits & (0x10 >> col) != 0 {
                        self.rect(x + n * 6 + col, y + r, 1, 1, c);
                    }
                }
            }
        }
    }
}

/// RGBA pixels as a PNG file, uncompressed (stored deflate blocks), for a
/// picture of the screen from a build with no image library
/// (`headless --settings-picture`).
pub fn png(w: usize, h: usize, rgba: &[u8]) -> Vec<u8> {
    fn crc(bytes: &[u8]) -> u32 {
        let mut c = 0xFFFF_FFFFu32;
        for b in bytes {
            c ^= u32::from(*b);
            for _ in 0..8 {
                c = if c & 1 != 0 {
                    0xEDB8_8320 ^ (c >> 1)
                } else {
                    c >> 1
                };
            }
        }
        !c
    }
    fn chunk(out: &mut Vec<u8>, kind: &[u8; 4], data: &[u8]) {
        out.extend_from_slice(&(data.len() as u32).to_be_bytes());
        let mut body = kind.to_vec();
        body.extend_from_slice(data);
        out.extend_from_slice(&body);
        out.extend_from_slice(&crc(&body).to_be_bytes());
    }
    let mut raw = Vec::with_capacity(h * (w * 4 + 1));
    for row in rgba.chunks(w * 4).take(h) {
        raw.push(0);
        raw.extend_from_slice(row);
    }
    let (mut a, mut b) = (1u32, 0u32);
    for x in &raw {
        a = (a + u32::from(*x)) % 65521;
        b = (b + a) % 65521;
    }
    let mut z = vec![0x78, 0x01];
    let blocks: Vec<&[u8]> = raw.chunks(65535).collect();
    for (i, block) in blocks.iter().enumerate() {
        z.push(u8::from(i + 1 == blocks.len()));
        let n = block.len() as u16;
        z.extend_from_slice(&n.to_le_bytes());
        z.extend_from_slice(&(!n).to_le_bytes());
        z.extend_from_slice(block);
    }
    z.extend_from_slice(&((b << 16) | a).to_be_bytes());
    let mut out = vec![0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A];
    let mut ihdr = Vec::new();
    ihdr.extend_from_slice(&(w as u32).to_be_bytes());
    ihdr.extend_from_slice(&(h as u32).to_be_bytes());
    ihdr.extend_from_slice(&[8, 6, 0, 0, 0]);
    chunk(&mut out, b"IHDR", &ihdr);
    chunk(&mut out, b"IDAT", &z);
    chunk(&mut out, b"IEND", &[]);
    out
}

/// A letter's seven rows, five bits each, the left column the highest:
/// the font of every shell menu.
pub fn glyph(c: char) -> [u8; 7] {
    match c {
        'A' => [0x0E, 0x11, 0x11, 0x1F, 0x11, 0x11, 0x11],
        'B' => [0x1E, 0x11, 0x11, 0x1E, 0x11, 0x11, 0x1E],
        'C' => [0x0E, 0x11, 0x10, 0x10, 0x10, 0x11, 0x0E],
        'D' => [0x1E, 0x11, 0x11, 0x11, 0x11, 0x11, 0x1E],
        'E' => [0x1F, 0x10, 0x10, 0x1E, 0x10, 0x10, 0x1F],
        'F' => [0x1F, 0x10, 0x10, 0x1E, 0x10, 0x10, 0x10],
        'G' => [0x0E, 0x11, 0x10, 0x17, 0x11, 0x11, 0x0F],
        'H' => [0x11, 0x11, 0x11, 0x1F, 0x11, 0x11, 0x11],
        'I' => [0x0E, 0x04, 0x04, 0x04, 0x04, 0x04, 0x0E],
        'J' => [0x07, 0x02, 0x02, 0x02, 0x02, 0x12, 0x0C],
        'K' => [0x11, 0x12, 0x14, 0x18, 0x14, 0x12, 0x11],
        'L' => [0x10, 0x10, 0x10, 0x10, 0x10, 0x10, 0x1F],
        'M' => [0x11, 0x1B, 0x15, 0x15, 0x11, 0x11, 0x11],
        'N' => [0x11, 0x11, 0x19, 0x15, 0x13, 0x11, 0x11],
        'O' => [0x0E, 0x11, 0x11, 0x11, 0x11, 0x11, 0x0E],
        'P' => [0x1E, 0x11, 0x11, 0x1E, 0x10, 0x10, 0x10],
        'Q' => [0x0E, 0x11, 0x11, 0x11, 0x15, 0x12, 0x0D],
        'R' => [0x1E, 0x11, 0x11, 0x1E, 0x14, 0x12, 0x11],
        'S' => [0x0F, 0x10, 0x10, 0x0E, 0x01, 0x01, 0x1E],
        'T' => [0x1F, 0x04, 0x04, 0x04, 0x04, 0x04, 0x04],
        'U' => [0x11, 0x11, 0x11, 0x11, 0x11, 0x11, 0x0E],
        'V' => [0x11, 0x11, 0x11, 0x11, 0x11, 0x0A, 0x04],
        'W' => [0x11, 0x11, 0x11, 0x15, 0x15, 0x15, 0x0A],
        'X' => [0x11, 0x11, 0x0A, 0x04, 0x0A, 0x11, 0x11],
        'Y' => [0x11, 0x11, 0x11, 0x0A, 0x04, 0x04, 0x04],
        'Z' => [0x1F, 0x01, 0x02, 0x04, 0x08, 0x10, 0x1F],
        '0' => [0x0E, 0x11, 0x13, 0x15, 0x19, 0x11, 0x0E],
        '1' => [0x04, 0x0C, 0x04, 0x04, 0x04, 0x04, 0x0E],
        '2' => [0x0E, 0x11, 0x01, 0x02, 0x04, 0x08, 0x1F],
        '3' => [0x1F, 0x02, 0x04, 0x02, 0x01, 0x11, 0x0E],
        '4' => [0x02, 0x06, 0x0A, 0x12, 0x1F, 0x02, 0x02],
        '5' => [0x1F, 0x10, 0x1E, 0x01, 0x01, 0x11, 0x0E],
        '6' => [0x06, 0x08, 0x10, 0x1E, 0x11, 0x11, 0x0E],
        '7' => [0x1F, 0x01, 0x02, 0x04, 0x08, 0x08, 0x08],
        '8' => [0x0E, 0x11, 0x11, 0x0E, 0x11, 0x11, 0x0E],
        '9' => [0x0E, 0x11, 0x11, 0x0F, 0x01, 0x02, 0x0C],
        ' ' => [0; 7],
        ':' => [0x00, 0x0C, 0x0C, 0x00, 0x0C, 0x0C, 0x00],
        '.' => [0x00, 0x00, 0x00, 0x00, 0x00, 0x0C, 0x0C],
        ',' => [0x00, 0x00, 0x00, 0x00, 0x0C, 0x04, 0x08],
        '-' => [0x00, 0x00, 0x00, 0x1F, 0x00, 0x00, 0x00],
        '_' => [0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x1F],
        '/' => [0x00, 0x01, 0x02, 0x04, 0x08, 0x10, 0x00],
        '>' => [0x08, 0x04, 0x02, 0x01, 0x02, 0x04, 0x08],
        '^' => [0x04, 0x0A, 0x11, 0x00, 0x00, 0x00, 0x00],
        '<' => [0x02, 0x04, 0x08, 0x10, 0x08, 0x04, 0x02],
        '(' => [0x02, 0x04, 0x08, 0x08, 0x08, 0x04, 0x02],
        ')' => [0x08, 0x04, 0x02, 0x02, 0x02, 0x04, 0x08],
        '[' => [0x0E, 0x08, 0x08, 0x08, 0x08, 0x08, 0x0E],
        ']' => [0x0E, 0x02, 0x02, 0x02, 0x02, 0x02, 0x0E],
        '\'' => [0x0C, 0x04, 0x08, 0x00, 0x00, 0x00, 0x00],
        '!' => [0x04, 0x04, 0x04, 0x04, 0x04, 0x00, 0x04],
        '+' => [0x00, 0x04, 0x04, 0x1F, 0x04, 0x04, 0x00],
        '=' => [0x00, 0x00, 0x1F, 0x00, 0x1F, 0x00, 0x00],
        _ => [0x0E, 0x11, 0x01, 0x02, 0x04, 0x00, 0x04],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A made-up game: a timing group and a fixes group, normal of all
    /// three switches and an assist preset of one.
    fn catalog() -> Catalog {
        let sw = |n: &str| Switch {
            name: n.to_owned(),
            description: format!("{n} on"),
        };
        Catalog {
            groups: vec![
                Group {
                    title: "Timing".into(),
                    description: "Frame timing".into(),
                    switches: vec![sw("no_lag")],
                },
                Group {
                    title: "Fixes".into(),
                    description: String::new(),
                    switches: vec![sw("smooth_walk"), sw("more_lives")],
                },
            ],
            presets: vec![
                Preset {
                    name: "assist".into(),
                    description: "Easier".into(),
                    switches: vec!["more_lives".into()],
                },
                Preset {
                    name: NORMAL.into(),
                    description: "No lag".into(),
                    switches: vec!["no_lag".into(), "smooth_walk".into()],
                },
            ],
        }
    }

    /// The PNG's known parts: the signature, the header's size, and the
    /// zlib stream's checksum of a tiny picture.
    #[test]
    fn a_png_has_its_header_and_size() {
        let px = [255u8, 0, 0, 255].repeat(4);
        let f = png(2, 2, &px);
        assert_eq!(&f[..8], &[0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A]);
        assert_eq!(&f[12..16], b"IHDR");
        assert_eq!(u32::from_be_bytes(f[16..20].try_into().unwrap()), 2);
        assert_eq!(&f[f.len() - 8..f.len() - 4], b"IEND");
    }

    #[test]
    fn modes_are_retro_normal_then_the_rest() {
        assert_eq!(catalog().modes(), vec![RETRO, NORMAL, "assist"]);
    }

    #[test]
    fn a_mode_sets_its_switches_and_closing_restarts_only_on_a_change() {
        let mut s = Screen::open(catalog(), RETRO);
        assert_eq!(s.mode.as_deref(), Some(RETRO));
        assert!(s.on.is_empty());
        // Right on the mode line: normal, with its switches.
        assert_eq!(s.key(Key::Right), Outcome::Redraw);
        assert_eq!(s.mode.as_deref(), Some(NORMAL));
        assert_eq!(s.on.len(), 2);
        assert!(s.rows().contains(&Row::Cancel));
        assert_eq!(
            s.key(Key::Open),
            Outcome::Closed {
                profile: NORMAL.into(),
                restart: true,
                theme: None
            }
        );
        // Opened in normal and closed at once: nothing to restart.
        let mut s = Screen::open(catalog(), NORMAL);
        assert_eq!(s.mode.as_deref(), Some(NORMAL));
        assert_eq!(
            s.key(Key::Back),
            Outcome::Closed {
                profile: NORMAL.into(),
                restart: false,
                theme: None
            }
        );
    }

    #[test]
    fn a_toggle_makes_a_custom_set_and_cancel_keeps_the_running_one() {
        let mut s = Screen::open(catalog(), NORMAL);
        // Down passes over the timing heading to its switch.
        s.key(Key::Down);
        assert_eq!(s.rows()[s.cursor], Row::Switch(0, 0));
        s.key(Key::Choose);
        assert!(!s.on.contains("no_lag"));
        assert_eq!(s.mode, None);
        assert_eq!(s.profile(), "smooth_walk");
        assert!(s.text(Row::Mode).contains("custom"));
        // Back on: normal again, by its switches.
        s.key(Key::Choose);
        assert_eq!(s.mode.as_deref(), Some(NORMAL));
        s.key(Key::Choose);
        // Off again: a change, so Cancel is the last line.
        let last = s.rows().len() - 1;
        s.cursor = last;
        assert_eq!(s.rows()[s.cursor], Row::Cancel);
        assert_eq!(
            s.key(Key::Choose),
            Outcome::Closed {
                profile: NORMAL.into(),
                restart: false,
                theme: None
            }
        );
    }

    #[test]
    fn a_kept_name_the_game_lacks_plays_what_it_can() {
        let c = catalog();
        assert_eq!(c.switches_of("smooth_walk+gone").len(), 1);
        let s = Screen::open(c, "assist+no_lag");
        assert_eq!(s.mode, None);
        assert_eq!(s.profile(), "more_lives+no_lag");
    }

    #[test]
    fn an_empty_normal_is_its_own_mode() {
        let mut c = catalog();
        c.presets[1].switches.clear();
        let mut s = Screen::open(c, RETRO);
        s.key(Key::Right);
        assert_eq!(s.mode.as_deref(), Some(NORMAL));
        // The same switches as retro: kept by name, no restart.
        assert_eq!(
            s.key(Key::Open),
            Outcome::Closed {
                profile: NORMAL.into(),
                restart: false,
                theme: None
            }
        );
    }

    #[test]
    fn it_draws_at_the_games_size_with_every_line_in_reach() {
        let mut s = Screen::open(catalog(), RETRO);
        let px = s.draw(256, 224);
        assert_eq!(px.len(), 256 * 224 * 4);
        // The title's gold is there, and the backdrop lets the game through.
        assert!(px.chunks(4).any(|p| p == GOLD));
        assert_eq!(&px[..4], &BACKDROP);
        // Every row scrolls into view on a short screen.
        for _ in 0..s.rows().len() {
            s.key(Key::Down);
            let px = s.draw(256, 64);
            assert!(px.chunks(4).any(|p| p == WHITE));
        }
    }

    #[test]
    fn a_theme_line_lists_the_looks_and_changes_with_no_restart() {
        let themes = vec![
            ("original".to_owned(), "Original".to_owned()),
            ("paper".to_owned(), "Paper".to_owned()),
        ];
        let s = Screen::open(catalog(), RETRO);
        assert!(!s.rows().contains(&Row::Theme), "no looks, no line");
        let mut s = Screen::open(catalog(), RETRO).with_themes(themes, "original");
        assert_eq!(s.rows()[1], Row::Theme);
        assert_eq!(s.text(Row::Theme), "Theme     < Original >");
        assert_eq!(s.key(Key::Down), Outcome::Redraw);
        assert_eq!(s.key(Key::Right), Outcome::Redraw);
        assert_eq!(s.text(Row::Theme), "Theme     < Paper >");
        assert!(s.changed());
        assert_eq!(
            s.key(Key::Open),
            Outcome::Closed {
                profile: RETRO.into(),
                restart: false,
                theme: Some("paper".into()),
            }
        );
        let px = s.draw(256, 224);
        assert_eq!(px.len(), 256 * 224 * 4);
    }

    #[test]
    fn published_themes_are_marked_and_one_can_be_removed_from_the_cache() {
        let themes = vec![
            ("paper".to_owned(), "Paper".to_owned()),
            ("night".to_owned(), "Night Shift".to_owned()),
            ("dawn".to_owned(), "Dawn".to_owned()),
        ];
        let mut marks = BTreeMap::new();
        marks.insert(
            "night".to_owned(),
            ThemeMark {
                author: "Ann".into(),
                kept: Kept::Download,
                size: 2_229_863,
            },
        );
        marks.insert(
            "dawn".to_owned(),
            ThemeMark {
                author: String::new(),
                kept: Kept::Downloaded,
                size: 4000,
            },
        );
        let mut s = Screen::open(catalog(), RETRO)
            .with_themes(themes, "paper")
            .with_marks(marks);
        // The build's own look has no mark: no line under it.
        assert_eq!(s.rows()[2], Row::Heading(0));
        s.key(Key::Down);
        s.key(Key::Right);
        assert_eq!(s.text(Row::Theme), "Theme     < Night Shift >");
        assert_eq!(&s.rows()[1..3], &[Row::Theme, Row::ThemeAbout]);
        assert_eq!(s.text(Row::ThemeAbout), "  by Ann, download 2.1 MB");
        assert!(s.detail(Row::Theme).contains("not downloaded yet"));
        // Down passes over the line under it, to the next one.
        s.key(Key::Right);
        assert_eq!(s.text(Row::ThemeAbout), "  downloaded");
        assert_eq!(s.rows()[3], Row::ThemeRemove);
        s.key(Key::Down);
        assert_eq!(s.rows()[s.cursor], Row::ThemeRemove);
        assert_eq!(s.key(Key::Choose), Outcome::Redraw);
        assert_eq!(s.removed, ["dawn"]);
        assert_eq!(s.text(Row::ThemeAbout), "  download 4 KB");
        assert!(!s.rows().contains(&Row::ThemeRemove));
        let Outcome::Closed { theme, restart, .. } = s.key(Key::Open) else {
            panic!("not closed");
        };
        assert_eq!((theme.as_deref(), restart), (Some("dawn"), false));
        assert_eq!(s.draw(256, 224).len(), 256 * 224 * 4);
    }

    fn saves(kept: &[usize]) -> Saves {
        use crate::shell::{SlotLine, SlotSave};
        let lines = (0..=4)
            .map(|slot| SlotLine {
                slot,
                save: kept.contains(&slot).then(|| SlotSave {
                    frame: 60 * 221,
                    profile: NORMAL.into(),
                    level: Some(2),
                    saved_at: Some(1000),
                    refused: None,
                }),
                damaged: false,
            })
            .collect();
        Saves {
            lines,
            now: Some(1300),
            rate: 60.0,
            files: true,
        }
    }

    #[test]
    fn the_saves_line_lists_four_slots_and_resume_once_kept() {
        let s = Screen::open(catalog(), RETRO).with_saves(saves(&[2]));
        let rows = s.rows();
        assert_eq!(
            &rows[1..8],
            &[
                Row::Saves,
                Row::Slot(1),
                Row::Slot(2),
                Row::Slot(3),
                Row::Slot(4),
                Row::LoadFile,
                Row::Heading(0)
            ]
        );
        assert_eq!(s.text(Row::Slot(1)), "Slot 1  empty");
        assert_eq!(s.text(Row::Slot(2)), "Slot 2  3:41  Normal  5 min ago");
        assert!(
            s.detail(Row::Slot(2))
                .starts_with("Slot 2: level 2, 3:41 played, in Normal, saved 5 min ago")
        );
        let s = Screen::open(catalog(), RETRO).with_saves(saves(&[0, 2]));
        assert_eq!(s.rows()[2], Row::Slot(0));
        assert_eq!(s.text(Row::Slot(0)), "Resume  3:41  Normal  5 min ago");
        // A host with no clock shows no age.
        let mut x = saves(&[2]);
        x.now = None;
        let s = Screen::open(catalog(), RETRO).with_saves(x);
        assert_eq!(s.text(Row::Slot(2)), "Slot 2  3:41  Normal");
        assert!(s.draw(256, 224).len() == 256 * 224 * 4);
    }

    #[test]
    fn choosing_a_slot_asks_what_to_do_and_hands_it_to_the_host() {
        let mut s = Screen::open(catalog(), RETRO).with_saves(saves(&[2]));
        s.key(Key::Down);
        s.key(Key::Down);
        assert_eq!(s.rows()[s.cursor], Row::Slot(2));
        assert_eq!(s.key(Key::Choose), Outcome::Redraw);
        assert_eq!(
            s.rows(),
            [
                Row::AskTitle(2),
                Row::Ask(2, AskItem::Save),
                Row::Ask(2, AskItem::Load),
                Row::Ask(2, AskItem::Download),
                Row::Ask(2, AskItem::Delete),
                Row::AskBack
            ]
        );
        assert_eq!(
            s.text(Row::Ask(2, AskItem::Save)),
            "Save here, replacing it"
        );
        s.key(Key::Down);
        assert_eq!(s.key(Key::Choose), Outcome::Slot(SlotAction::Load(2)));
        // Back on the list, at the slot.
        assert_eq!(s.rows()[s.cursor], Row::Slot(2));
        // Back from the ask returns to the list, not out of the screen.
        s.key(Key::Choose);
        assert_eq!(s.key(Key::Back), Outcome::Redraw);
        assert_eq!(s.asking, None);
        // An empty slot can only be saved to.
        s.key(Key::Up);
        s.key(Key::Choose);
        assert_eq!(
            s.rows(),
            [Row::AskTitle(1), Row::Ask(1, AskItem::Save), Row::AskBack]
        );
        assert_eq!(s.key(Key::Choose), Outcome::Slot(SlotAction::Save(1)));
        s.set_saves(saves(&[1, 2]));
        s.say("Saved to slot 1");
        assert_eq!(s.rows()[s.cursor], Row::Slot(1));
        assert_eq!(s.said.as_deref(), Some("Saved to slot 1"));
        let lines = s.lines();
        assert!(lines.iter().any(|l| l.starts_with("> Slot 1  3:41")));
        assert_eq!(lines.last().unwrap(), "| Saved to slot 1");
        s.key(Key::Down);
        assert_eq!(s.said, None);
        // Resume can be loaded, not saved to.
        let mut s = Screen::open(catalog(), RETRO).with_saves(saves(&[0]));
        s.key(Key::Down);
        s.key(Key::Choose);
        assert_eq!(s.rows()[1], Row::Ask(0, AskItem::Load));
        // Load from a file.
        let mut s = Screen::open(catalog(), RETRO).with_saves(saves(&[]));
        while s.rows()[s.cursor] != Row::LoadFile {
            s.key(Key::Down);
        }
        assert_eq!(s.key(Key::Choose), Outcome::Slot(SlotAction::LoadFile));
    }

    #[test]
    fn a_load_closes_the_screen_keeping_the_game_s_switches() {
        let mut s = Screen::open(catalog(), RETRO).with_saves(saves(&[1]));
        s.key(Key::Right);
        assert_eq!(s.mode.as_deref(), Some(NORMAL));
        assert_eq!(
            s.close_loaded(),
            Outcome::Closed {
                profile: RETRO.into(),
                restart: false,
                theme: None
            }
        );
    }

    #[test]
    fn console_only_hides_the_saves_and_cancel_puts_the_shell_back() {
        let shell = crate::shell::Settings::default();
        let mut s = Screen::open(catalog(), RETRO)
            .with_shell(shell)
            .with_saves(saves(&[1]));
        assert_eq!(
            &s.rows()[1..4],
            &[Row::ConsoleOnly, Row::QuickButtons, Row::Saves]
        );
        assert_eq!(s.text(Row::ConsoleOnly), "[ ] Console only");
        s.key(Key::Down);
        s.key(Key::Choose);
        assert_eq!(s.text(Row::ConsoleOnly), "[X] Console only");
        assert_eq!(&s.rows()[3..5], &[Row::Saves, Row::SavesOff]);
        assert!(!s.rows().contains(&Row::Slot(1)));
        s.key(Key::Down);
        s.key(Key::Right);
        assert_eq!(
            s.shell,
            Some(crate::shell::Settings {
                console_only: true,
                quick_buttons: true
            })
        );
        assert!(s.changed());
        // Cancel: the shell's settings as they were.
        let last = s.rows().len() - 1;
        s.cursor = last;
        assert_eq!(s.rows()[last], Row::Cancel);
        assert!(matches!(
            s.key(Key::Choose),
            Outcome::Closed { restart: false, .. }
        ));
        assert_eq!(s.shell, Some(shell));
        // A close keeps the choice, with no restart.
        let mut s = Screen::open(catalog(), RETRO).with_shell(shell);
        s.key(Key::Down);
        s.key(Key::Choose);
        assert!(matches!(
            s.key(Key::Back),
            Outcome::Closed { restart: false, .. }
        ));
        assert!(s.shell.unwrap().console_only);
    }
}
