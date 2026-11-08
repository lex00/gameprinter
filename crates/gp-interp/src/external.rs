//! External runtimes (docs/interpreter.md, "External runtimes"): another
//! program that can't load the original's state, but can start at a level
//! start the way a player does and log a trace of what happens, one line a
//! frame. Super Mario Bros. Remastered, run under Godot with our probe
//! (scripts/remastered/, docs/remastered.md), is the first.
//!
//! | Part | What |
//! |---|---|
//! | [`Trace`] | The probe's trace: a header line, then one row a frame, JSON lines |
//! | [`TraceMap`] | How the trace's columns read as model paths: a scale and an offset for a number, a table for a name |
//! | [`TraceRuntime`] | The trace as a `gp_runtime::Runtime`: each step takes the next row, so the lockstep measures it as it measures the rebuilt game |
//!
//! A trace's row `f = -1` is where the player stands as it gets control,
//! before any press; row `f = k` is the end of the `k`th frame after it,
//! with that frame's presses. The lockstep compares its start frame before
//! any step, so the runtime stands at row -1 before its first step.
//!
//! The runtime checks each step's presses against the row's own (`in`),
//! so a trace made from other presses than the clip's is refused, not
//! scored.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use gp_runtime::{
    Capabilities, Entry, Event, InputFrame, LoadHow, LoadNote, PatchResult, Rect, Runtime, Until,
    Value,
};

/// The trace format's version, the probe's `probe` header key.
pub const TRACE_VERSION: &str = "1";

/// The buttons in the input log's order (docs/formats/recording.md).
const ORDER: &[u8; 8] = b"RLDUTSBA";

/// One row of a trace. Numbers are in the external program's own units.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Row {
    /// The frame: -1 as the player gets control, then 0, 1, ...
    pub f: i64,
    /// Every other column, by name: numbers, flags and names.
    #[serde(flatten)]
    pub columns: BTreeMap<String, serde_json::Value>,
}

impl Row {
    /// The presses the row was made with, in the input log's order with
    /// `.` for a button not held: `R.....BA`.
    pub fn pressed(&self) -> Option<&str> {
        self.columns.get("in").and_then(|v| v.as_str())
    }
}

/// A whole trace.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Trace {
    /// The probe's header: its version, the program and its version, the
    /// level, the settings it ran with.
    pub header: BTreeMap<String, serde_json::Value>,
    /// Row -1 first, then the frames in order.
    pub rows: Vec<Row>,
}

impl Trace {
    /// Reads a trace's text: the header line, then rows. A row with
    /// `error` (the probe gave up, as when the player never got control)
    /// is an error here.
    pub fn parse(text: &str) -> Result<Self, String> {
        let mut lines = text
            .lines()
            .enumerate()
            .filter(|(_, l)| !l.trim().is_empty());
        let (_, first) = lines.next().ok_or("the trace is empty")?;
        let header: BTreeMap<String, serde_json::Value> =
            serde_json::from_str(first).map_err(|e| format!("trace line 1: {e}"))?;
        match header.get("probe").and_then(|v| v.as_str()) {
            Some(TRACE_VERSION) => {}
            Some(v) => {
                return Err(format!(
                    "trace line 1: probe version {v}; this gp reads {TRACE_VERSION}"
                ));
            }
            None => return Err("trace line 1: no `probe` version, so it isn't a header".into()),
        }
        let mut rows: Vec<Row> = Vec::new();
        for (i, line) in lines {
            let v: serde_json::Value =
                serde_json::from_str(line).map_err(|e| format!("trace line {}: {e}", i + 1))?;
            if let Some(e) = v.get("error").and_then(|e| e.as_str()) {
                return Err(format!("the probe stopped: {e}"));
            }
            let row: Row =
                serde_json::from_value(v).map_err(|e| format!("trace line {}: {e}", i + 1))?;
            let want = rows.last().map_or(-1, |r| r.f + 1);
            if row.f != want {
                return Err(format!(
                    "trace line {}: frame {} where {want} comes next",
                    i + 1,
                    row.f
                ));
            }
            rows.push(row);
        }
        if rows.is_empty() {
            return Err("the trace has a header and no rows".into());
        }
        Ok(Self { header, rows })
    }

    /// A header value as text.
    pub fn says(&self, key: &str) -> String {
        match self.header.get(key) {
            Some(serde_json::Value::String(s)) => s.clone(),
            Some(v) => v.to_string(),
            None => String::new(),
        }
    }
}

/// How one model path is read from a trace's column.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MapField {
    /// The model path: `entities.mario[0].x`.
    pub path: String,
    /// The trace's column.
    pub column: String,
    /// A number is `column * scale + offset`, in the model's units.
    #[serde(default = "one")]
    pub scale: f64,
    #[serde(default)]
    pub offset: f64,
    /// Round a number to the nearest whole: for a position the program
    /// keeps finer than it draws, compared with an original that keeps
    /// whole pixels.
    #[serde(default)]
    pub round: bool,
    /// A flag column (true or false).
    #[serde(default)]
    pub flag: bool,
    /// A name column: the column's value (as text) to the model's name.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub values: BTreeMap<String, String>,
    /// Where the numbers come from, in words, for the docs and the report.
    #[serde(default)]
    pub note: String,
}

fn one() -> f64 {
    1.0
}

/// A trace map: what an external program's trace columns are as model
/// paths, as a file (scripts/remastered/smb1r-map.toml).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TraceMap {
    /// The program, as the trace's `game` header names it: `smb1r`.
    pub game: String,
    /// The model it maps to (`model.name`).
    pub model: String,
    #[serde(default)]
    pub description: String,
    #[serde(rename = "field")]
    pub fields: Vec<MapField>,
}

impl TraceMap {
    pub fn parse(text: &str) -> Result<Self, String> {
        let m: Self = toml::from_str(text).map_err(|e| format!("trace map: {e}"))?;
        for f in &m.fields {
            if f.flag && !f.values.is_empty() {
                return Err(format!(
                    "trace map: {} is a flag or a name, not both",
                    f.path
                ));
            }
        }
        Ok(m)
    }

    /// The paths it gives, in its order.
    pub fn paths(&self) -> Vec<String> {
        self.fields.iter().map(|f| f.path.clone()).collect()
    }

    /// One row's values, in the map's order.
    pub fn read(&self, row: &Row) -> Result<Vec<Value>, String> {
        self.fields
            .iter()
            .map(|f| {
                let v = row.columns.get(&f.column).ok_or_else(|| {
                    format!(
                        "trace frame {}: no `{}` column for {}",
                        row.f, f.column, f.path
                    )
                })?;
                convert(f, v).ok_or_else(|| {
                    format!(
                        "trace frame {}: `{}` is {v}, which {} can't be read from",
                        row.f, f.column, f.path
                    )
                })
            })
            .collect()
    }
}

/// A column's value as the model's.
fn convert(f: &MapField, v: &serde_json::Value) -> Option<Value> {
    if f.flag {
        return v.as_bool().map(Value::Flag);
    }
    if !f.values.is_empty() {
        let key = match v {
            serde_json::Value::String(s) => s.clone(),
            other => other.to_string(),
        };
        return f.values.get(&key).map(|n| Value::Text(n.clone()));
    }
    let x = v.as_f64()? * f.scale + f.offset;
    Some(Value::Fixed(fixed(if f.round { x.round() } else { x })))
}

/// A number as a 16.16 fixed-point raw value, to the nearest.
pub fn fixed(x: f64) -> i64 {
    (x * 65536.0).round() as i64
}

/// The presses of an input frame as the trace writes them: the first
/// controller's buttons in the input log's order, `.` for one not held.
/// Start and Select are written as not held, since the probe doesn't press
/// them.
pub fn pressed(input: &InputFrame) -> String {
    let pad = (input.buttons & 0xff) as u8;
    ORDER
        .iter()
        .enumerate()
        .map(|(i, &c)| {
            let bit = 1u8 << (7 - i);
            if c == b'T' || c == b'S' || pad & bit == 0 {
                '.'
            } else {
                c as char
            }
        })
        .collect()
}

/// A trace as a runtime: the lockstep steps it as it steps a model, and
/// it answers queries for its map's paths from the row it stands at.
#[derive(Debug, Clone)]
pub struct TraceRuntime {
    level: String,
    /// Each row's values, in the map's order.
    values: Vec<Vec<Value>>,
    pressed: Vec<Option<String>>,
    paths: Vec<String>,
    /// The row it stands at: 0 is the trace's row -1.
    at: usize,
}

impl TraceRuntime {
    /// The trace read through the map, standing at row -1. `level` is the
    /// model level name it answers `level` with.
    pub fn new(trace: &Trace, map: &TraceMap, level: &str) -> Result<Self, String> {
        let values = trace
            .rows
            .iter()
            .map(|r| map.read(r))
            .collect::<Result<Vec<_>, _>>()?;
        Ok(Self {
            level: level.to_owned(),
            values,
            pressed: trace
                .rows
                .iter()
                .map(|r| r.pressed().map(str::to_owned))
                .collect(),
            paths: map.paths(),
            at: 0,
        })
    }

    /// The values at the row it stands at, in the map's order.
    pub fn values(&self) -> &[Value] {
        &self.values[self.at]
    }
}

const CANT: &str = "an external runtime runs only from its own level start and its trace";

impl Runtime for TraceRuntime {
    fn capabilities(&self) -> Capabilities {
        Capabilities::FORK.union(Capabilities::SNAPSHOT)
    }

    fn step(&mut self, inputs: &[InputFrame]) -> Result<Vec<Event>, String> {
        for input in inputs {
            if self.at + 1 >= self.values.len() {
                return Err(format!(
                    "the trace ends at frame {}; the clip goes on",
                    self.at as i64 - 1
                ));
            }
            self.at += 1;
            let want = pressed(input);
            if let Some(got) = &self.pressed[self.at]
                && *got != want
            {
                return Err(format!(
                    "trace frame {}: made with {got} pressed, the clip presses {want}",
                    self.at - 1
                ));
            }
        }
        Ok(Vec::new())
    }

    fn run_until(&mut self, _: &str, _: &[InputFrame], _: u32) -> Result<Until, String> {
        Err(CANT.into())
    }

    fn frame(&self) -> u64 {
        self.at as u64
    }

    fn snapshot(&self) -> Vec<u8> {
        (self.at as u64).to_le_bytes().to_vec()
    }

    fn restore(&mut self, snap: &[u8]) -> Result<(), String> {
        let b: [u8; 8] = snap.try_into().map_err(|_| "not a trace snapshot")?;
        let at = u64::from_le_bytes(b) as usize;
        if at >= self.values.len() {
            return Err("the snapshot is past the trace's end".into());
        }
        self.at = at;
        Ok(())
    }

    fn fork(&self) -> Self {
        self.clone()
    }

    fn canonical_state(&self) -> Vec<u8> {
        self.snapshot()
    }

    fn state_hash(&self) -> u64 {
        self.at as u64
    }

    /// It can't load a state: it stays at its own start, and says so.
    fn load_state(&mut self, _state: &[u8]) -> Result<Vec<LoadNote>, String> {
        self.at = 0;
        Ok(vec![LoadNote {
            path: "*".into(),
            how: LoadHow::Missing,
            why: CANT.into(),
        }])
    }

    fn query(&self, selector: &str) -> Result<Vec<Entry>, String> {
        if selector == "level" {
            return Ok(vec![Entry {
                path: "level".into(),
                value: Value::Text(self.level.clone()),
            }]);
        }
        let i = self
            .paths
            .iter()
            .position(|p| p == selector)
            .ok_or_else(|| format!("the trace map gives no {selector}"))?;
        Ok(vec![Entry {
            path: selector.to_owned(),
            value: self.values[self.at][i].clone(),
        }])
    }

    fn sample(
        &mut self,
        _: &[String],
        _: &[InputFrame],
        _: u32,
    ) -> Result<Vec<Vec<Entry>>, String> {
        Err(CANT.into())
    }

    fn entities_at(&self, _: Rect) -> Vec<u64> {
        Vec::new()
    }

    fn patch(&mut self, _: &str) -> PatchResult {
        PatchResult::Rejected(CANT.into())
    }

    fn set(&mut self, _: &str, _: Value) -> Result<(), String> {
        Err(CANT.into())
    }

    fn explain(&self, _: &str) -> Vec<String> {
        Vec::new()
    }
}

/// The presses a probe needs for a lockstep: the model steps once on each
/// frame the original reads its controller, with that frame's input, so
/// the probe's frame `k` takes the input of the `k`th compared frame after
/// the start. `frames` are the lockstep's compared frames (model, original)
/// and `inputs` the clip's, from the start's original frame.
pub fn probe_presses(
    frames: &[(u64, u64)],
    start_original: u64,
    inputs: &[InputFrame],
) -> Vec<String> {
    frames
        .iter()
        .skip(1)
        .filter_map(|&(_, of)| {
            let k = of.checked_sub(start_original + 1)?;
            inputs.get(usize::try_from(k).ok()?).map(pressed)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    const MAP: &str = r#"
game = "smb1r"
model = "smb1"

[[field]]
path = "entities.mario[0].x"
column = "x"
offset = 248

[[field]]
path = "entities.mario[0].vx"
column = "vx"
scale = 0.016666666666666666

[[field]]
path = "entities.mario[0].y"
column = "y"
offset = 176
round = true

[[field]]
path = "entities.mario[0].on_ground"
column = "ground"
flag = true

[[field]]
path = "entities.mario[0].facing"
column = "facing"
values = { "1" = "right", "-1" = "left" }
"#;

    fn trace() -> String {
        [
            r#"{"probe":"1","game":"smb1r","level":"1-1","physics":"classic"}"#,
            r#"{"f":-1,"x":-208.0,"vx":0.0,"y":-0.076,"ground":true,"facing":1,"in":"........"}"#,
            r#"{"f":0,"x":-207.9,"vx":6.0,"y":-0.6,"ground":true,"facing":1,"in":"R......."}"#,
            r#"{"f":1,"x":-207.7,"vx":12.0,"y":-4.2,"ground":false,"facing":-1,"in":"R......A"}"#,
        ]
        .join("\n")
    }

    fn frame(buttons: u32) -> InputFrame {
        InputFrame {
            buttons,
            ..InputFrame::default()
        }
    }

    #[test]
    fn a_trace_reads_its_header_and_rows_in_order() {
        let t = Trace::parse(&trace()).unwrap();
        assert_eq!(t.says("physics"), "classic");
        assert_eq!(t.rows.len(), 3);
        assert_eq!(t.rows[0].f, -1);
        assert_eq!(t.rows[2].pressed(), Some("R......A"));
    }

    #[test]
    fn a_trace_with_a_gap_or_an_error_or_another_version_is_refused() {
        let gap = trace().replace(r#""f":1,"#, r#""f":2,"#);
        assert!(Trace::parse(&gap).unwrap_err().contains("frame 2 where 1"));
        let err = format!(
            "{}\n{}",
            r#"{"probe":"1"}"#, r#"{"error":"no control after 1200 frames"}"#
        );
        assert!(Trace::parse(&err).unwrap_err().contains("no control"));
        let v2 = trace().replace(r#""probe":"1""#, r#""probe":"2""#);
        assert!(Trace::parse(&v2).unwrap_err().contains("version 2"));
    }

    #[test]
    fn the_map_reads_numbers_flags_and_names_in_the_models_units() {
        let t = Trace::parse(&trace()).unwrap();
        let m = TraceMap::parse(MAP).unwrap();
        let v = m.read(&t.rows[0]).unwrap();
        // -208 from Remastered's origin is NES x 40.
        assert_eq!(v[0], Value::Fixed(40 << 16));
        assert_eq!(v[1], Value::Fixed(0));
        // 0.076 px above the ground draws on the ground's row.
        assert_eq!(v[2], Value::Fixed(176 << 16));
        assert_eq!(v[3], Value::Flag(true));
        assert_eq!(v[4], Value::Text("right".into()));
        let v = m.read(&t.rows[2]).unwrap();
        // 12 px a second is a fifth of a pixel a frame at 60 frames a second.
        assert_eq!(v[1], Value::Fixed(fixed(0.2)));
        assert_eq!(v[2], Value::Fixed(172 << 16));
        assert_eq!(v[4], Value::Text("left".into()));
        // 40.3 px, to the nearest 65536th.
        assert_eq!(v[0], Value::Fixed(fixed(40.3)));
    }

    #[test]
    fn a_missing_column_says_which() {
        let t = Trace::parse(&trace().replace(r#","ground":true"#, "")).unwrap();
        let m = TraceMap::parse(MAP).unwrap();
        assert!(
            m.read(&t.rows[0])
                .unwrap_err()
                .contains("no `ground` column")
        );
    }

    #[test]
    fn presses_are_written_in_the_logs_order_without_start_or_select() {
        use gp_evidence::input_log::button;
        let b = u32::from(button::RIGHT | button::B | button::A | button::START);
        assert_eq!(pressed(&frame(b)), "R.....BA");
        assert_eq!(pressed(&frame(0)), "........");
    }

    #[test]
    fn the_runtime_steps_through_rows_and_checks_the_presses() {
        use gp_evidence::input_log::button;
        let t = Trace::parse(&trace()).unwrap();
        let m = TraceMap::parse(MAP).unwrap();
        let mut r = TraceRuntime::new(&t, &m, "w1_1").unwrap();
        assert_eq!(r.frame(), 0);
        let x = |r: &TraceRuntime| r.query("entities.mario[0].x").unwrap()[0].value.clone();
        assert_eq!(x(&r), Value::Fixed(40 << 16));
        r.step(&[frame(u32::from(button::RIGHT))]).unwrap();
        assert_eq!(x(&r), Value::Fixed(fixed(40.1)));
        let mut wrong = r.fork();
        assert!(
            wrong
                .step(&[frame(0)])
                .unwrap_err()
                .contains("made with R......A")
        );
        r.step(&[frame(u32::from(button::RIGHT | button::A))])
            .unwrap();
        assert!(r.step(&[frame(0)]).unwrap_err().contains("trace ends"));
        assert_eq!(
            r.query("level").unwrap()[0].value,
            Value::Text("w1_1".into())
        );
        let notes = r.load_state(&[]).unwrap();
        assert_eq!(notes[0].how, LoadHow::Missing);
        assert_eq!(r.frame(), 0);
    }

    #[test]
    fn a_probe_takes_the_input_of_each_frame_the_model_steps_on() {
        use gp_evidence::input_log::button;
        let inputs: Vec<InputFrame> = (0..5u32)
            .map(|i| {
                frame(if i % 2 == 0 {
                    u32::from(button::RIGHT)
                } else {
                    0
                })
            })
            .collect();
        // Start at original frame 100; the model stepped on 101, 102 and 104
        // (103 was a lag frame).
        let frames = [(0, 100), (1, 101), (2, 102), (3, 104)];
        assert_eq!(
            probe_presses(&frames, 100, &inputs),
            vec!["R.......", "........", "........"]
        );
    }

    /// A made-up original: one byte of RAM, its x, one more each frame
    /// Right is held, and every frame polled.
    #[derive(Clone)]
    struct Walker {
        x: u8,
        frame: u64,
    }

    impl Runtime for Walker {
        fn capabilities(&self) -> Capabilities {
            Capabilities::empty()
        }
        fn step(&mut self, inputs: &[InputFrame]) -> Result<Vec<Event>, String> {
            let mut out = Vec::new();
            for i in inputs {
                self.frame += 1;
                if i.buttons & u32::from(gp_evidence::input_log::button::RIGHT) != 0 {
                    self.x += 1;
                }
                out.push(Event {
                    frame: self.frame,
                    kind: crate::runtime::FRAME_KIND.into(),
                    entity: None,
                    data: vec![Entry {
                        path: "polled".into(),
                        value: Value::Flag(true),
                    }],
                });
            }
            Ok(out)
        }
        fn run_until(&mut self, _: &str, _: &[InputFrame], _: u32) -> Result<Until, String> {
            Err("no".into())
        }
        fn frame(&self) -> u64 {
            self.frame
        }
        fn snapshot(&self) -> Vec<u8> {
            vec![self.x]
        }
        fn restore(&mut self, s: &[u8]) -> Result<(), String> {
            self.x = s[0];
            Ok(())
        }
        fn fork(&self) -> Self {
            self.clone()
        }
        fn canonical_state(&self) -> Vec<u8> {
            vec![self.x]
        }
        fn state_hash(&self) -> u64 {
            u64::from(self.x)
        }
        fn load_state(&mut self, _: &[u8]) -> Result<Vec<LoadNote>, String> {
            Err("no".into())
        }
        fn query(&self, sel: &str) -> Result<Vec<Entry>, String> {
            match sel {
                "ram[0x0040]" => Ok(vec![Entry {
                    path: sel.into(),
                    value: Value::Int(i64::from(self.x)),
                }]),
                _ => Err(format!("{sel}?")),
            }
        }
        fn sample(
            &mut self,
            _: &[String],
            _: &[InputFrame],
            _: u32,
        ) -> Result<Vec<Vec<Entry>>, String> {
            Err("no".into())
        }
        fn entities_at(&self, _: Rect) -> Vec<u64> {
            Vec::new()
        }
        fn patch(&mut self, _: &str) -> PatchResult {
            PatchResult::Rejected("no".into())
        }
        fn set(&mut self, _: &str, _: Value) -> Result<(), String> {
            Err("no".into())
        }
        fn explain(&self, _: &str) -> Vec<String> {
            Vec::new()
        }
    }

    #[test]
    fn the_lockstep_scores_a_trace_as_it_scores_a_model() {
        use crate::lockstep::{Mode, Options, Start, lockstep_from};
        use crate::scenario::{Scenario, Source, outcome};
        let mapping = crate::mapping::Mapping::parse(
            "schema = 1\nmodel = \"tiny\"\nsource = \"tiny\"\nalign = \"polled\"\n\n[[field]]\npath = \"entities.hero[0].x\"\nram = [0x40]\n",
        )
        .unwrap();
        let map = TraceMap::parse(
            "game = \"made-up\"\nmodel = \"tiny\"\n\n[[field]]\npath = \"entities.hero[0].x\"\ncolumn = \"x\"\noffset = 10\n",
        )
        .unwrap();
        // The rebuilt side keeps up for two frames, then stops a pixel short.
        let text = [
            r#"{"probe":"1","game":"made-up"}"#,
            r#"{"f":-1,"x":-10,"in":"........"}"#,
            r#"{"f":0,"x":-9,"in":"R......."}"#,
            r#"{"f":1,"x":-8,"in":"R......."}"#,
            r#"{"f":2,"x":-8,"in":"R......."}"#,
        ]
        .join("\n");
        let trace = Trace::parse(&text).unwrap();
        let mut rebuilt = TraceRuntime::new(&trace, &map, "one").unwrap();
        let s = Scenario {
            id: "walk".into(),
            name: "Right, held 3 frames".into(),
            tier: 1,
            mechanic: String::new(),
            params: Vec::new(),
            frames: 3,
            presses: vec!["right@0-2".into()],
            log: None,
            requires: Vec::new(),
            start: None,
            source: Source::Generated,
        };
        let inputs = s.inputs().unwrap();
        let start = Start {
            frame: 0,
            original_frame: 0,
            notes: Vec::new(),
            state: rebuilt.canonical_state(),
        };
        let opts = Options {
            mode: Mode::Free,
            values: true,
            ..Options::default()
        };
        let mut original = Walker { x: 0, frame: 0 };
        // Any spec: the lockstep reads it only to resync, and this run is free.
        let spec = crate::tests::load().spec().clone();
        let ls = lockstep_from(
            &mut original,
            &mut rebuilt,
            &spec,
            &mapping,
            &start,
            &inputs,
            &opts,
        )
        .unwrap();
        assert_eq!(ls.frames.len(), 4);
        assert_eq!(probe_presses(&ls.frames, 0, &inputs), vec!["R......."; 3]);
        let catalog = crate::mechanics::Catalog {
            catalog: "",
            model: "tiny".into(),
            mechanics: Vec::new(),
            opaque: Vec::new(),
            problems: Vec::new(),
            debt: Vec::new(),
            counts: BTreeMap::new(),
            paths: Vec::new(),
        };
        let r = outcome(&s, &start, &ls, &catalog);
        // Three frames of four together; wrong from the third press on.
        assert_eq!(r.headline.together_permille, 750);
        assert_eq!(r.headline.exact, 0);
        assert!(!r.exact);
        let first = r.first.unwrap();
        assert_eq!(first.frame, 3);
        assert_eq!((first.original.as_str(), first.model.as_str()), ("3", "2"));
    }
}
