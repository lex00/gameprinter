//! The recording manifest: the one JSON document that names every part of a
//! recording by hash. See docs/formats/recording.md.

use serde::{Deserialize, Serialize};

use crate::hash::Hash;

/// Recording format version. Bump on any breaking change and add a converter.
pub const RECORDING_SCHEMA: u32 = 1;

/// What was run, and nothing else: the same ROM, input, runtime, power-on
/// state and checkpoint interval give the same manifest, so the same ID.
/// When it was recorded, its name and notes are in the index
/// ([`crate::recordings::IndexEntry`]).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct RecordingManifest {
    pub schema: u32,
    pub source: SourceRef,
    pub runtime: RuntimeRef,
    pub power_on: PowerOn,
    /// Number of frames, which equals the input log's frame count.
    pub frames: u64,
    pub input_log: Hash,
    pub checkpoint_interval: u32,
    #[serde(default)]
    pub checkpoints: Vec<Checkpoint>,
    /// Per-frame state hashes ([`crate::frame_hashes`]), frames 0 to `frames`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub frame_hashes: Option<Hash>,
    /// Full per-frame state ([`crate::trace`]), frames 0 to `frames`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub trace: Option<TraceRef>,
    /// What happened within each frame ([`crate::events`]), frames 0 to `frames`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub events: Option<EventsRef>,
    pub provenance: Provenance,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum SourceRef {
    /// Identified as in [`crate::source::identify_nes`]. The title comes
    /// from the source registry, by hash, and isn't part of the manifest.
    NesRom { sha256: String, mapper: u16 },
    /// A game model, run by a model runtime such as the interpreter
    /// (`gp-interp`). `bundle` is a store object holding the model's
    /// canonical text and the lifted asset files it reads, so a replay needs
    /// nothing else; `name` is the model's `model.name`. Added 2026-10-28
    /// within schema 1: older builds refuse it as an unknown kind.
    Model { name: String, bundle: Hash },
    /// An arcade romset, run by gp-arcade: `identity` is the set's
    /// identity (gp_arcade::romset::identity, the SHA-256 of its chips'
    /// SHA-1s sorted) and `system` MAME's name for the set (`tapper`).
    /// Added 2026-11-12 within schema 1: older builds refuse it as an
    /// unknown kind.
    Romset { identity: String, system: String },
}

impl SourceRef {
    /// The kind's name, as the manifest writes it.
    pub fn kind(&self) -> &'static str {
        match self {
            Self::NesRom { .. } => "nes-rom",
            Self::Model { .. } => "model",
            Self::Romset { .. } => "romset",
        }
    }

    /// The headerless SHA-256 of an NES ROM source.
    pub fn nes_sha256(&self) -> Option<&str> {
        match self {
            Self::NesRom { sha256, .. } => Some(sha256),
            Self::Model { .. } | Self::Romset { .. } => None,
        }
    }

    /// An arcade romset source's identity.
    pub fn romset_identity(&self) -> Option<&str> {
        match self {
            Self::Romset { identity, .. } => Some(identity),
            Self::NesRom { .. } | Self::Model { .. } => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct RuntimeRef {
    /// For example `gp-nes` or `mesen`.
    pub name: String,
    pub version: String,
    /// Source commit for runtimes we build, such as the `tetanes-core` fork.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub commit: Option<String>,
}

/// Everything that changes how the console behaves from power-on. A runtime
/// powers on from this alone, so a replay reproduces it, and refuses a
/// combination it can't reproduce. [`PowerOn::default`] is what `gp record`
/// uses.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PowerOn {
    pub region: Region,
    pub ram_init: RamInit,
    /// Master clock cycles the PPU runs behind the CPU (CPU-PPU clock
    /// alignment, nesdev wiki "PPU frame timing"). `tetanes-core` fixes it at 1.
    pub cpu_ppu_offset: u8,
    /// Whether the PPU ignores writes to some registers until about one frame
    /// after power-on, as real hardware does (nesdev wiki "PPU power up state").
    pub ppu_warmup: bool,
    /// Board revisions for mappers whose revision the ROM header can't name.
    pub mapper_revisions: MapperRevisions,
    pub four_player: FourPlayer,
    pub zapper: bool,
    /// Whether opposing D-pad directions can be held together, as input logs
    /// can say. Off would change logged input before the game sees it.
    pub concurrent_dpad: bool,
    /// Whether the PPU draws pixels. Headless (off) detects sprite 0 hits on
    /// a separate path.
    pub render_video: bool,
    /// Whether the APU mixes audio samples.
    pub mix_audio: bool,
}

impl Default for PowerOn {
    fn default() -> Self {
        Self {
            region: Region::Ntsc,
            ram_init: RamInit::Zeros,
            cpu_ppu_offset: 1,
            ppu_warmup: false,
            mapper_revisions: MapperRevisions::default(),
            four_player: FourPlayer::None,
            zapper: false,
            concurrent_dpad: true,
            render_video: false,
            mix_audio: false,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum Region {
    Ntsc,
    Pal,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum RamInit {
    Zeros,
    Ones,
    Random {
        seed: u64,
    },
    /// FCEUX's default power-on fill of internal RAM: `00 00 00 00 FF FF FF
    /// FF` repeated (FCEUX 2.6.6 `src/fceu.cpp`, `FCEU_MemoryRand`, option
    /// 0). Other RAM (PRG RAM, CHR RAM) powers on as zeros, as FCEUX's does
    /// for a movie. For playing movies made in FCEUX on games that read RAM
    /// before writing it.
    Fceux,
}

impl RamInit {
    /// The byte at internal RAM address `addr` (0 to $7FF) at power-on, for
    /// the fills gp-nes writes itself.
    pub fn fceux_byte(addr: usize) -> u8 {
        if addr & 4 != 0 { 0xFF } else { 0x00 }
    }
}

#[derive(
    Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema,
)]
#[serde(deny_unknown_fields)]
pub struct MapperRevisions {
    /// Mapper 4.
    pub mmc3: Mmc3Revision,
    /// Mapper 71.
    pub bf909: Bf909Revision,
}

#[derive(
    Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema,
)]
#[serde(rename_all = "kebab-case")]
pub enum Mmc3Revision {
    A,
    #[default]
    #[serde(rename = "bc")]
    BC,
    /// Acclaim's clone, which clocks its IRQ counter on the falling edge.
    Acclaim,
}

#[derive(
    Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema,
)]
#[serde(rename_all = "kebab-case")]
pub enum Bf909Revision {
    #[default]
    Bf909x,
    Bf9097,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum FourPlayer {
    /// Two controllers.
    None,
    /// The NES Four Score.
    FourScore,
    /// The Famicom four players adapter.
    Satellite,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct Checkpoint {
    pub frame: u64,
    pub snapshot: Hash,
}

/// A frame trace: chunks of `chunk_frames` frames covering frames 0 to the
/// recording's last, in order.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct TraceRef {
    /// The chunk schema ([`crate::trace::SCHEMA`]).
    pub schema: u32,
    pub chunk_frames: u32,
    /// The field table object ([`crate::trace::FieldTable`]) that names the
    /// state's fields.
    pub fields: Hash,
    pub chunks: Vec<Hash>,
}

/// An event stream, chunked like the trace.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct EventsRef {
    /// The chunk schema ([`crate::events::SCHEMA`]).
    pub schema: u32,
    pub chunk_frames: u32,
    pub chunks: Vec<Hash>,
}

/// Where the input came from. The input log itself is named by hash, so a
/// scripted run carries no name of its own.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum Provenance {
    HumanPlay,
    TasImport { url: String },
    Bot { name: String },
    Scripted,
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum ManifestError {
    #[error("recording schema {0} is not supported (this build reads 0 to {RECORDING_SCHEMA})")]
    Schema(u32),
    #[error("manifest JSON: {0}")]
    Json(String),
    #[error("checkpoint at frame {frame} is out of order or past the end ({frames} frames)")]
    Checkpoint { frame: u64, frames: u64 },
    #[error("checkpoint interval must be at least 1")]
    Interval,
    #[error(
        "{what} has {chunks} chunks of {chunk_frames} frames, which can't cover frames 0 to {frames}"
    )]
    ChunkCount {
        what: &'static str,
        chunks: usize,
        chunk_frames: u32,
        frames: u64,
    },
}

impl RecordingManifest {
    pub fn validate(&self) -> Result<(), ManifestError> {
        if self.schema != RECORDING_SCHEMA {
            return Err(ManifestError::Schema(self.schema));
        }
        if self.checkpoint_interval == 0 {
            return Err(ManifestError::Interval);
        }
        let mut last = None;
        for c in &self.checkpoints {
            if c.frame > self.frames || last.is_some_and(|l| c.frame <= l) {
                return Err(ManifestError::Checkpoint {
                    frame: c.frame,
                    frames: self.frames,
                });
            }
            last = Some(c.frame);
        }
        let chunked = [
            self.trace
                .as_ref()
                .map(|t| ("trace", t.chunk_frames, t.chunks.len())),
            self.events
                .as_ref()
                .map(|e| ("events", e.chunk_frames, e.chunks.len())),
        ];
        for (what, chunk_frames, chunks) in chunked.into_iter().flatten() {
            if chunk_frames == 0 || chunks != crate::chunk::chunk_count(chunk_frames, self.frames) {
                return Err(ManifestError::ChunkCount {
                    what,
                    chunks,
                    chunk_frames,
                    frames: self.frames,
                });
            }
        }
        Ok(())
    }

    /// The manifest's JSON Schema, as `schema/recording.json` holds it,
    /// for the current schema ([`RECORDING_SCHEMA`]).
    pub fn json_schema() -> String {
        let mut schema = schemars::schema_for!(RecordingManifest);
        schema.insert(
            "$id".into(),
            format!("game-printer:recording@{RECORDING_SCHEMA}").into(),
        );
        let mut text = serde_json::to_string_pretty(&schema).expect("schemas serialize");
        text.push('\n');
        text
    }

    pub fn to_json(&self) -> String {
        serde_json::to_string_pretty(self).expect("manifests always serialize")
    }

    /// Reads a stored manifest of any supported schema, converting older ones
    /// to this one, and validates it. A converted manifest has a new hash,
    /// but the recording keeps its ID, which is the hash of the stored bytes.
    pub fn from_json(bytes: &[u8]) -> Result<Self, ManifestError> {
        #[derive(Deserialize)]
        struct Schema {
            schema: u32,
        }
        let json = |e: serde_json::Error| ManifestError::Json(e.to_string());
        let manifest: Self = match serde_json::from_slice::<Schema>(bytes)
            .map_err(json)?
            .schema
        {
            0 => serde_json::from_slice::<v0::Manifest>(bytes)
                .map_err(json)?
                .into(),
            RECORDING_SCHEMA => serde_json::from_slice(bytes).map_err(json)?,
            other => return Err(ManifestError::Schema(other)),
        };
        manifest.validate()?;
        Ok(manifest)
    }
}

/// Schema 0, as written up to 2026-10-27. It also held the creation time,
/// name, notes, golden flag and registry title, so the same run got a new ID
/// each time; those now live in the index, which already had the time and
/// name.
mod v0 {
    use serde::Deserialize;

    use super::{
        Checkpoint, FourPlayer, Hash, MapperRevisions, RamInit, RecordingManifest, Region,
        RuntimeRef,
    };

    #[derive(Deserialize)]
    pub(super) struct Manifest {
        source: SourceRef,
        runtime: RuntimeRef,
        power_on: PowerOn,
        frames: u64,
        input_log: Hash,
        checkpoint_interval: u32,
        #[serde(default)]
        checkpoints: Vec<Checkpoint>,
        #[serde(default)]
        frame_hashes: Option<Hash>,
        // Schema 0 also had `trace` and `events` fields, drafts that nothing
        // ever wrote; they're ignored.
        provenance: Provenance,
    }

    #[derive(Deserialize)]
    #[serde(tag = "kind", rename_all = "kebab-case")]
    enum SourceRef {
        NesRom { sha256: String, mapper: u16 },
    }

    #[derive(Deserialize)]
    struct PowerOn {
        region: Region,
        ram_init: RamInit,
    }

    #[derive(Deserialize)]
    #[serde(tag = "kind", rename_all = "kebab-case")]
    enum Provenance {
        HumanPlay,
        TasImport { url: String },
        Bot { name: String },
        Scripted {},
    }

    impl From<Manifest> for RecordingManifest {
        fn from(m: Manifest) -> Self {
            let SourceRef::NesRom { sha256, mapper } = m.source;
            Self {
                schema: super::RECORDING_SCHEMA,
                source: super::SourceRef::NesRom { sha256, mapper },
                runtime: m.runtime,
                // Schema 0 recorded only these two; gp-nes fixed the rest at
                // these values (written out, so a later change to
                // `PowerOn::default` can't reach old recordings).
                power_on: super::PowerOn {
                    region: m.power_on.region,
                    ram_init: m.power_on.ram_init,
                    cpu_ppu_offset: 1,
                    ppu_warmup: false,
                    mapper_revisions: MapperRevisions {
                        mmc3: super::Mmc3Revision::BC,
                        bf909: super::Bf909Revision::Bf909x,
                    },
                    four_player: FourPlayer::None,
                    zapper: false,
                    concurrent_dpad: true,
                    render_video: false,
                    mix_audio: false,
                },
                frames: m.frames,
                input_log: m.input_log,
                checkpoint_interval: m.checkpoint_interval,
                checkpoints: m.checkpoints,
                frame_hashes: m.frame_hashes,
                trace: None,
                events: None,
                provenance: match m.provenance {
                    Provenance::HumanPlay => super::Provenance::HumanPlay,
                    Provenance::TasImport { url } => super::Provenance::TasImport { url },
                    Provenance::Bot { name } => super::Provenance::Bot { name },
                    Provenance::Scripted {} => super::Provenance::Scripted,
                },
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> RecordingManifest {
        RecordingManifest {
            schema: RECORDING_SCHEMA,
            source: SourceRef::NesRom {
                sha256: "00".repeat(32),
                mapper: 0,
            },
            runtime: RuntimeRef {
                name: "gp-nes".into(),
                version: "0.1.0".into(),
                commit: None,
            },
            power_on: PowerOn::default(),
            frames: 120,
            input_log: Hash::of(b"log"),
            checkpoint_interval: 60,
            frame_hashes: Some(Hash::of(b"hashes")),
            checkpoints: vec![
                Checkpoint {
                    frame: 0,
                    snapshot: Hash::of(b"s0"),
                },
                Checkpoint {
                    frame: 60,
                    snapshot: Hash::of(b"s60"),
                },
            ],
            trace: Some(TraceRef {
                schema: 1,
                chunk_frames: 64,
                fields: Hash::of(b"fields"),
                chunks: vec![Hash::of(b"c0"), Hash::of(b"c1")],
            }),
            events: Some(EventsRef {
                schema: 1,
                chunk_frames: 121,
                chunks: vec![Hash::of(b"e0")],
            }),
            provenance: Provenance::Scripted,
        }
    }

    #[test]
    fn json_round_trip() {
        let m = sample();
        let back: RecordingManifest = serde_json::from_str(&m.to_json()).unwrap();
        assert_eq!(back, m);
        assert!(m.to_json().contains("\"kind\": \"nes-rom\""));
    }

    #[test]
    fn valid_sample() {
        sample().validate().unwrap();
    }

    #[test]
    fn rejects_bad_checkpoints() {
        let mut m = sample();
        m.checkpoints.swap(0, 1);
        assert!(matches!(
            m.validate(),
            Err(ManifestError::Checkpoint { .. })
        ));
        let mut m = sample();
        m.checkpoints[1].frame = 500;
        assert!(matches!(
            m.validate(),
            Err(ManifestError::Checkpoint { .. })
        ));
    }

    #[test]
    fn chunks_must_cover_frames_exactly() {
        // 120 frames are frames 0 to 120: two chunks of 64, or one of 121.
        let mut m = sample();
        m.trace.as_mut().unwrap().chunks.pop();
        assert!(matches!(
            m.validate(),
            Err(ManifestError::ChunkCount { what: "trace", .. })
        ));
        let mut m = sample();
        m.trace.as_mut().unwrap().chunks.push(Hash::of(b"c2"));
        assert!(matches!(
            m.validate(),
            Err(ManifestError::ChunkCount { what: "trace", .. })
        ));
        let mut m = sample();
        m.events.as_mut().unwrap().chunk_frames = 120;
        assert!(matches!(
            m.validate(),
            Err(ManifestError::ChunkCount { what: "events", .. })
        ));
        let mut m = sample();
        m.trace.as_mut().unwrap().chunk_frames = 0;
        assert!(m.validate().is_err());
    }

    #[test]
    fn the_fceux_ram_init_round_trips() {
        let json = serde_json::to_string(&RamInit::Fceux).unwrap();
        assert_eq!(json, r#"{"kind":"fceux"}"#);
        assert_eq!(
            serde_json::from_str::<RamInit>(&json).unwrap(),
            RamInit::Fceux
        );
        assert_eq!(RamInit::fceux_byte(3), 0);
        assert_eq!(RamInit::fceux_byte(4), 0xff);
    }

    #[test]
    fn rejects_unknown_schema() {
        let mut m = sample();
        m.schema = 7;
        assert_eq!(m.validate(), Err(ManifestError::Schema(7)));
        assert_eq!(
            RecordingManifest::from_json(m.to_json().as_bytes()),
            Err(ManifestError::Schema(7))
        );
    }

    #[test]
    fn from_json_reads_the_current_schema() {
        let m = sample();
        assert_eq!(RecordingManifest::from_json(m.to_json().as_bytes()), Ok(m));
        assert!(matches!(
            RecordingManifest::from_json(b"{\"schema\": 1}"),
            Err(ManifestError::Json(_))
        ));
    }

    #[test]
    fn power_on_round_trips_every_field() {
        let p = PowerOn {
            region: Region::Pal,
            ram_init: RamInit::Random { seed: 9 },
            cpu_ppu_offset: 2,
            ppu_warmup: true,
            mapper_revisions: MapperRevisions {
                mmc3: Mmc3Revision::Acclaim,
                bf909: Bf909Revision::Bf9097,
            },
            four_player: FourPlayer::FourScore,
            zapper: true,
            concurrent_dpad: false,
            render_video: true,
            mix_audio: true,
        };
        let json = serde_json::to_string(&p).unwrap();
        assert_eq!(serde_json::from_str::<PowerOn>(&json).unwrap(), p);
        assert!(json.contains("\"mmc3\":\"acclaim\""), "{json}");
        assert!(json.contains("\"four_player\":\"four-score\""), "{json}");
        // A field this build doesn't know could change power-on behaviour, so
        // it's an error rather than ignored.
        let extra = json.replace('{', "{\"dendy\":true,");
        assert!(serde_json::from_str::<PowerOn>(&extra).is_err());
    }

    /// A manifest exactly as schema 0 wrote it.
    const SCHEMA_0: &str = r#"{
  "schema": 0,
  "source": {
    "kind": "nes-rom",
    "sha256": "2d3f6c8a1b0e4f5d6c7b8a9e0f1d2c3b4a5968778695a4b3c2d1e0f9a8b7c6d5",
    "name": "Super Mario Bros. (NTSC, PRG0)",
    "mapper": 0
  },
  "runtime": {
    "name": "gp-nes",
    "version": "0.1.0 (tetanes-core 0.17.0 + game-printer patches)"
  },
  "power_on": {
    "region": "ntsc",
    "ram_init": {
      "kind": "zeros"
    }
  },
  "frames": 60,
  "input_log": "af1349b9f5f9a1a6a0404dea36dcc9499bcb25c9adc112b7cc9a93cae41f3262",
  "checkpoint_interval": 60,
  "checkpoints": [
    {
      "frame": 0,
      "snapshot": "af1349b9f5f9a1a6a0404dea36dcc9499bcb25c9adc112b7cc9a93cae41f3262"
    },
    {
      "frame": 60,
      "snapshot": "af1349b9f5f9a1a6a0404dea36dcc9499bcb25c9adc112b7cc9a93cae41f3262"
    }
  ],
  "frame_hashes": "af1349b9f5f9a1a6a0404dea36dcc9499bcb25c9adc112b7cc9a93cae41f3262",
  "provenance": {
    "kind": "scripted",
    "name": "smb-run-right"
  },
  "golden": false,
  "created_at": "2026-10-27T09:12:44Z"
}"#;

    #[test]
    fn schema_0_manifests_load_as_the_current_schema() {
        let m = RecordingManifest::from_json(SCHEMA_0.as_bytes()).unwrap();
        assert_eq!(m.schema, RECORDING_SCHEMA);
        assert_eq!(m.power_on, PowerOn::default());
        assert_eq!(m.provenance, Provenance::Scripted);
        assert_eq!(m.frames, 60);
        assert_eq!(m.checkpoints.len(), 2);
        let SourceRef::NesRom { sha256, mapper } = &m.source else {
            panic!("a ROM source")
        };
        assert!(sha256.starts_with("2d3f6c8a") && *mapper == 0);
        // Converted manifests are current ones, so they also write as schema 1.
        assert!(m.to_json().contains("\"schema\": 1"));
        assert!(!m.to_json().contains("created_at"));
    }
}
