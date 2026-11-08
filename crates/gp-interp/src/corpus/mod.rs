//! The clip corpus (docs/interpreter.md, "The clip corpus"; the research
//! is docs/research/clip-corpus.md): a grid of what the player presses by
//! the situation the press happens in, derived from the game model, with
//! starts found in the run's recordings, timings measured on the original
//! and a measure of which numbers each clip can move.
//!
//! The user asked for it on 2026-10-29: "a tree/matrix of clips that could
//! be generated in any game, or by genre ... the choices should be based on
//! what the game is and what characters can actually do", consistent enough
//! to build an alignment corpus from.
//!
//! | Part | What |
//! |---|---|
//! | [`derive`] | The contexts and cells from the model, its catalog and the original's timings; pruning by the records and by the original's outcomes |
//! | [`scan`] | Starts for each context found in a recording of the original |
//! | [`coverage`] | Which clips move each number and enter each state, the minimal set, the gaps, and the queue's order |
//!
//! A cell is (mechanic, press pattern, context). Its ID is built from those
//! three and never from a frame number the model computes, so refining the
//! model never renames or moves a clip, and adding a mechanic only adds
//! cells. A clip is a cell at one start: `CELL@START`.
//!
//! The corpus is written to a capture run as `corpus/corpus.json`
//! (docs/formats/corpus.md). The same model, catalog, mapping, ROM and
//! recordings give the same file, byte for byte.

pub mod coverage;
pub mod derive;
pub mod scan;

#[cfg(test)]
mod tests;

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

/// The corpus format's version (docs/formats/corpus.md).
pub const CORPUS_VERSION: &str = "0.1.0";

/// The generator's own version: a change to the rules that make cells
/// bumps it, and a regeneration's diff names what was added and removed.
pub const GENERATOR: &str = "1";

/// The corpus file in a run's `corpus/` folder.
pub const CORPUS_FILE: &str = "corpus.json";

/// The clip corpus file's JSON Schema, as `schema/clip-corpus.json` holds
/// it, generated from [`Corpus`].
pub fn schema() -> String {
    let mut schema = schemars::schema_for!(Corpus);
    schema.insert(
        "$id".into(),
        format!("game-printer:clip-corpus@{GENERATOR}").into(),
    );
    let mut text = serde_json::to_string_pretty(&schema).expect("schemas serialize");
    text.push('\n');
    text
}

/// The seed of the `mixed` cells' pseudo-random presses.
pub const SEED: u64 = 1;

/// Hold lengths every game gets, log-spaced, so corpora are comparable
/// across games.
pub const HOLDS: [u64; 7] = [1, 2, 4, 8, 16, 32, 64];

/// Frames with nothing pressed after a cell's last press.
pub const SETTLE: u64 = crate::scenario::SETTLE;

/// The most frames a cell's own presses and settle run (2 seconds); a
/// lead-in adds to it.
pub const MAX_FRAMES: u64 = crate::scenario::MAX_FRAMES;

/// The whole corpus file.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct Corpus {
    pub corpus: Header,
    /// Timings measured on the original once, by name.
    pub anchors: BTreeMap<String, Anchor>,
    pub contexts: Vec<Context>,
    pub cells: Vec<Cell>,
    pub clips: Vec<Clip>,
    pub coverage: coverage::Coverage,
    /// The clip IDs in the queue's order, before any score or mark
    /// ([`coverage::rank`]).
    pub order: Vec<String>,
}

/// What the corpus was built from.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct Header {
    /// [`CORPUS_VERSION`].
    pub format: String,
    /// [`GENERATOR`].
    pub generator: String,
    /// The extensions' versions the records came from: "core 0.4.4,
    /// platformer 0.4.2".
    pub catalog: String,
    /// `blake3:` hashes of the model text, the catalog (its records and
    /// uses as JSON), the state mapping's text and the ROM.
    pub model: String,
    pub catalog_hash: String,
    pub mapping: String,
    pub rom: String,
    /// The recordings and save points the starts were looked for in, in
    /// order.
    pub recordings: Vec<String>,
    /// The model's genre extensions, which choose the probe cells.
    pub genre: Vec<String>,
    pub seed: u64,
    /// The player entity.
    pub player: String,
}

/// A timing measured on the original.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct Anchor {
    pub frames: u64,
    /// The start it was measured from.
    pub from: String,
    /// What was held: `right`, `right+b`.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub held: String,
    /// In words: "the top of a held jump from standing, 32 frames".
    pub says: String,
}

/// How a context's start is reached.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum How {
    /// Found in a recording of the original.
    Recording,
    /// A free play save point the user made.
    SavePoint,
    /// Another context's start plus presses that reach it on the original.
    LeadIn,
}

/// A situation a press happens in, and where it was found.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct Context {
    /// `ground.still`, `ground.edge[1].over`, `enemy.near[goomba]`.
    pub id: String,
    /// "On the ground, standing still".
    pub words: String,
    /// The predicate over the original's mapped measures, in words.
    pub predicate: String,
    /// The mechanics whose records name it.
    pub needs: Vec<String>,
    /// For a context reached by a lead-in: from which context, holding
    /// what, for which anchor's frames.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lead: Option<Lead>,
    pub starts: Vec<Start>,
    /// Why no start the rebuilt game can begin at was found.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub missing: Option<String>,
}

/// A lead-in: the presses that take a start of `from` into this context.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct Lead {
    pub from: String,
    /// Actions held through the lead-in and the cell: `fwd`, `run`.
    pub hold: Vec<String>,
    /// The anchor whose frames the lead-in lasts, less `back`.
    pub anchor: String,
    #[serde(default)]
    pub back: u64,
}

/// One start of a context.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct Start {
    /// `s1`, `s2`, by recording and frame.
    pub id: String,
    pub how: How,
    /// The recording (or save point) the original plays to the frame.
    pub recording: String,
    pub frame: u64,
    /// The model's frame there, as the lockstep counts it.
    pub model_frame: u64,
    pub level: String,
    /// The player faces left: `fwd` is the left button.
    #[serde(default)]
    pub facing_left: bool,
    /// The original's snapshot there, relative to `$GP_HOME`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub snapshot: Option<String>,
    /// The input log the original plays to the frame, relative to
    /// `$GP_HOME`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub log: Option<String>,
    /// Presses before the cell's own, and how many frames they take: a
    /// lead-in's, or nothing pressed until the player stands still.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub prefix: Vec<String>,
    #[serde(default)]
    pub prefix_frames: u64,
    /// Why the rebuilt game can't begin here: kept, with no clips.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cant: Option<String>,
}

/// A press pattern (docs/research/clip-corpus.md, section 2).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct Pattern {
    /// `idle`, `tap`, `hold`, `release`, `taps`, `chord`, `through`,
    /// `early`, `again`, `late`, `doublet`, `turn`, `mixed`, `check`.
    pub name: String,
    /// The action pressed (`fwd` and `back` are directions relative to
    /// the player's facing), or empty.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub action: String,
    /// Actions held with it.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub with: Vec<String>,
    /// A length from [`HOLDS`] or another fixed number.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub frames: Option<u64>,
    /// A length or time measured on the original, by anchor name.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub anchor: Option<String>,
    /// For a press placed at the original's own event in the clip
    /// (`land`, `leave`): the event, and frames from it (negative before).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub event: Option<EventAt>,
}

/// A press placed by the original's own timing in each clip.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct EventAt {
    /// `land`: the player's next landing after leaving the ground;
    /// `leave`: the player's leaving the ground.
    pub event: String,
    pub offset: i64,
    /// The button pressed there and for how many frames.
    pub button: String,
    pub len: u64,
}

/// Where a cell stands.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, schemars::JsonSchema,
)]
#[serde(rename_all = "lowercase")]
pub enum Status {
    /// Fitted from.
    Active,
    /// Removed, with the reason; the next model change can bring it back.
    Pruned,
    /// For a move the model lacks: scored, never fitted; a probe that
    /// differs is a gap report.
    Probe,
    /// A check: a mapping check on the original, or a press that should
    /// do nothing.
    Check,
}

/// A mapping check's result on the original.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct CheckResult {
    pub passed: bool,
    pub says: String,
}

/// One cell of the grid.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct Cell {
    /// `MECHANIC:PATTERN:CONTEXT`: `platformer.jump:jump.hold-16:ground.still`.
    pub id: String,
    pub mechanic: String,
    pub pattern: Pattern,
    pub context: String,
    /// 0 checks, 1 one action from standing, 2 other contexts and
    /// combinations, 3 lead-ins, 4 probes and mixed.
    pub tier: u8,
    /// In plain words: "Jump, held 16 frames, standing still".
    pub name: String,
    /// The presses with `fwd` as right, frames from the cell's first
    /// (after any lead-in); a press placed by an event is added per clip.
    pub presses: Vec<String>,
    /// Frames its presses and settle run.
    pub frames: u64,
    pub status: Status,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    /// Cells that must match first.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub requires: Vec<String>,
    /// The numbers the cell's clips move on the model, measured.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub excites: Vec<String>,
    /// The mechanic states its clips enter on the model.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub enters: Vec<String>,
    /// For a mapping check, its result on the original.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub check: Option<CheckResult>,
}

/// A cell at one start: the unit the background queue scores.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct Clip {
    /// `CELL@CONTEXT/START`.
    pub id: String,
    pub cell: String,
    /// `CONTEXT/START`.
    pub start: String,
    /// Every press from the start, lead-in included, directions resolved
    /// by the player's facing there.
    pub presses: Vec<String>,
    pub frames: u64,
    /// `blake3:` of the clip's spec (its ID, start, presses and frames):
    /// what the queue's ledger keys a clip's results on.
    pub spec: String,
    pub status: Status,
    /// The hash of the original's mapped measures on every frame.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub original: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub excites: Vec<String>,
    /// For each number in `excites`, how much it moves the clip: the
    /// mapped measures on frames that change, counted, the larger of the
    /// two perturbations. Two clips that move the same two numbers in a
    /// different ratio tell them apart.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub effect: Vec<u64>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub enters: Vec<String>,
    /// In the minimal set.
    #[serde(default)]
    pub minimal: bool,
}

impl Clip {
    /// How much the clip moves number `p`, when it does.
    pub fn effect_of(&self, p: &str) -> Option<u64> {
        let i = self.excites.iter().position(|e| e == p)?;
        Some(self.effect.get(i).copied().unwrap_or(1))
    }

    /// The spec hash of a clip: its ID, start, presses and frames.
    pub fn spec_of(id: &str, start: &Start, presses: &[String], frames: u64) -> String {
        let v = serde_json::json!({
            "id": id,
            "recording": start.recording,
            "frame": start.frame,
            "model_frame": start.model_frame,
            "presses": presses,
            "frames": frames,
        });
        hash(v.to_string().as_bytes())
    }
}

/// `blake3:` and the hex hash of `bytes`.
pub fn hash(bytes: &[u8]) -> String {
    format!("blake3:{}", gp_evidence::Hash::of(bytes).to_hex())
}

/// The corpus as its file's bytes: pretty JSON with a final newline.
pub fn to_bytes(c: &Corpus) -> Vec<u8> {
    let mut v = serde_json::to_vec_pretty(c).expect("a corpus serializes");
    v.push(b'\n');
    v
}

/// A corpus file read back.
pub fn from_bytes(b: &[u8]) -> Result<Corpus, String> {
    let c: Corpus = serde_json::from_slice(b).map_err(|e| format!("the corpus file: {e}"))?;
    let major = |v: &str| v.split('.').take(2).collect::<Vec<_>>().join(".");
    if major(&c.corpus.format) != major(CORPUS_VERSION) {
        return Err(format!(
            "the corpus file is format {}, this gp reads {CORPUS_VERSION}; build it again",
            c.corpus.format
        ));
    }
    Ok(c)
}

/// A span in the input log's format: `b@from-to`.
pub fn span(button: &str, from: u64, len: u64) -> String {
    format!("{button}@{from}-{}", from + len.max(1) - 1)
}

/// The last frame a span list presses, plus one.
pub fn end_of(presses: &[String]) -> u64 {
    presses
        .iter()
        .filter_map(|p| p.rsplit_once('-').and_then(|(_, t)| t.parse::<u64>().ok()))
        .map(|t| t + 1)
        .max()
        .unwrap_or(0)
}

/// `presses` moved later by `by` frames.
pub fn shift(presses: &[String], by: u64) -> Vec<String> {
    presses
        .iter()
        .filter_map(|p| {
            let (b, r) = p.split_once('@')?;
            let (f, t) = r.split_once('-')?;
            let (f, t) = (f.parse::<u64>().ok()?, t.parse::<u64>().ok()?);
            Some(format!("{b}@{}-{}", f + by, t + by))
        })
        .collect()
}

/// `presses` with left and right swapped, for a player facing left.
pub fn mirror(presses: &[String]) -> Vec<String> {
    presses
        .iter()
        .map(|p| {
            let (b, r) = p.split_once('@').unwrap_or((p, ""));
            let bs: Vec<&str> = b
                .split('+')
                .map(|x| match x {
                    "left" => "right",
                    "right" => "left",
                    o => o,
                })
                .collect();
            format!("{}@{r}", bs.join("+"))
        })
        .collect()
}
