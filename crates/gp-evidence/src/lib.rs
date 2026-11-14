//! Evidence for Game Printer: where it lives, how it's addressed, and the
//! formats recordings are made of. See docs/formats/recording.md.

pub mod baseline;
pub mod chunk;
pub mod dump;
pub mod events;
pub mod frame_hashes;
pub mod guard;
pub mod hash;
pub mod input_log;
pub mod jsonl;
pub mod manifest;
pub mod model_events;
pub mod paths;
pub mod recordings;
pub mod registry;
pub mod source;
pub mod store;
pub mod trace;

pub use hash::Hash;
pub use input_log::{InputFrame, InputLog};
pub use manifest::RecordingManifest;
pub use paths::Paths;
pub use store::Store;
