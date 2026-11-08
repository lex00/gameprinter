//! The event stream of a model run: event chunk schema 2.
//!
//! The header is the NES stream's (`GPEV`, schema u32, first frame u64,
//! frame count u32, body length u32) and the body is one LZ4 block. A model
//! event has a name and an optional entity, not a position in a frame, so
//! the body is a name table (a count, then each name as a length and UTF-8
//! bytes, all unsigned LEB128 counts) and, for each frame in order, an event
//! count and that many records: the name's index in the table and the
//! entity's index plus one (0 for none), both unsigned LEB128. Events keep
//! the order the systems raised them in. Readers of schema 1 refuse these
//! chunks, since the schema number differs.

use crate::chunk::{self, ChunkError, Reader, put_varint};

pub use crate::events::MAGIC;
/// The event chunk schema for model runs.
pub const SCHEMA: u32 = 2;
const HEADER: usize = 24;
/// The most events a frame of a model can hold.
pub const MAX_EVENTS_PER_FRAME: usize = 1 << 16;
/// The most names a chunk's table can hold.
const MAX_NAMES: usize = 1 << 12;

/// One event of a model run: the frame it happened in, its name, such as
/// `player.died`, and the entity (an index into the model's bodies).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModelEvent {
    pub frame: u64,
    pub kind: String,
    pub entity: Option<u64>,
}

/// Builds model event chunks from each frame's events, starting at frame 0.
pub struct ModelEventWriter {
    chunk_frames: u32,
    next_frame: u64,
    first_frame: u64,
    frames: Vec<Vec<ModelEvent>>,
}

impl ModelEventWriter {
    pub fn new(chunk_frames: u32) -> Self {
        assert!(chunk_frames > 0, "a chunk holds at least one frame");
        Self {
            chunk_frames,
            next_frame: 0,
            first_frame: 0,
            frames: Vec::new(),
        }
    }

    /// Adds the next frame's events (their `frame` is ignored). Returns a
    /// finished chunk once it holds `chunk_frames` frames.
    pub fn push(&mut self, events: &[ModelEvent]) -> Option<Vec<u8>> {
        if self.frames.is_empty() {
            self.first_frame = self.next_frame;
        }
        self.frames.push(events.to_vec());
        self.next_frame += 1;
        (self.frames.len() as u32 == self.chunk_frames).then(|| self.seal())
    }

    /// The last, partly filled chunk, if any frames are left in it.
    pub fn finish(mut self) -> Option<Vec<u8>> {
        (!self.frames.is_empty()).then(|| self.seal())
    }

    fn seal(&mut self) -> Vec<u8> {
        let mut names: Vec<&str> = Vec::new();
        for e in self.frames.iter().flatten() {
            if !names.contains(&e.kind.as_str()) {
                names.push(&e.kind);
            }
        }
        let mut body = Vec::new();
        put_varint(&mut body, names.len() as u64);
        for n in &names {
            put_varint(&mut body, n.len() as u64);
            body.extend_from_slice(n.as_bytes());
        }
        for events in &self.frames {
            put_varint(&mut body, events.len() as u64);
            for e in events {
                let at = names.iter().position(|n| *n == e.kind).expect("listed");
                put_varint(&mut body, at as u64);
                put_varint(&mut body, e.entity.map_or(0, |i| i + 1));
            }
        }
        let packed = chunk::compress(&body);
        let mut out = Vec::with_capacity(HEADER + packed.len());
        out.extend_from_slice(MAGIC);
        out.extend_from_slice(&SCHEMA.to_le_bytes());
        out.extend_from_slice(&self.first_frame.to_le_bytes());
        out.extend_from_slice(&(self.frames.len() as u32).to_le_bytes());
        out.extend_from_slice(&(body.len() as u32).to_le_bytes());
        out.extend_from_slice(&packed);
        self.frames.clear();
        out
    }
}

/// A decoded chunk: frames `first_frame` to `first_frame + frames - 1`, and
/// their events in order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModelEventChunk {
    pub first_frame: u64,
    pub frames: u32,
    pub events: Vec<ModelEvent>,
}

/// The most body bytes `frames` frames can take.
fn max_body(frames: u32) -> usize {
    // Names (a table of at most MAX_NAMES names of up to 255 bytes) and, per
    // frame, a count and records of at most 2 varints of 10 bytes.
    MAX_NAMES * 266 + (frames as usize).saturating_mul(10 + MAX_EVENTS_PER_FRAME * 20)
}

/// Decodes a chunk the manifest says covers `expected`: a header claiming
/// other frames is refused before the body is decompressed.
pub fn decode_chunk_covering(
    bytes: &[u8],
    expected: std::ops::Range<u64>,
) -> Result<ModelEventChunk, ChunkError> {
    let mut r = Reader { rest: bytes };
    if r.bytes(4, "magic")? != MAGIC {
        return Err(ChunkError::Magic("event"));
    }
    let version = r.u32("schema")?;
    if version != SCHEMA {
        return Err(ChunkError::Schema {
            what: "model event",
            version,
        });
    }
    let first_frame = r.u64("first frame")?;
    let frames = r.u32("frame count")?;
    if frames == 0 {
        return Err(ChunkError::Invalid("a chunk with no frames".into()));
    }
    chunk::expect_frames("event", first_frame, frames, &expected)?;
    let len = r.u32("body length")? as usize;
    let body = chunk::decompress(r.rest, len, max_body(frames))?;
    let mut r = Reader { rest: &body };
    let n = r.len("name count")?;
    if n > MAX_NAMES {
        return Err(ChunkError::Invalid(format!("{n} event names")));
    }
    let mut names = Vec::with_capacity(n);
    for _ in 0..n {
        let l = r.len("name length")?;
        let text = std::str::from_utf8(r.bytes(l, "event name")?)
            .map_err(|_| ChunkError::Invalid("an event name isn't UTF-8".into()))?;
        names.push(text.to_owned());
    }
    let mut events = Vec::new();
    for frame in first_frame..first_frame + u64::from(frames) {
        let count = r.len("event count")?;
        if count > MAX_EVENTS_PER_FRAME || count.saturating_mul(2) > r.rest.len() {
            return Err(ChunkError::Invalid(format!(
                "frame {frame} claims {count} events"
            )));
        }
        for _ in 0..count {
            let at = r.len("event name index")?;
            let kind = names
                .get(at)
                .ok_or_else(|| ChunkError::Invalid(format!("event name {at} isn't in the table")))?
                .clone();
            let entity = r.varint("entity")?.checked_sub(1);
            events.push(ModelEvent {
                frame,
                kind,
                entity,
            });
        }
    }
    if !r.rest.is_empty() {
        return Err(ChunkError::Trailing(r.rest.len()));
    }
    Ok(ModelEventChunk {
        first_frame,
        frames,
        events,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ev(frame: u64, kind: &str, entity: Option<u64>) -> ModelEvent {
        ModelEvent {
            frame,
            kind: kind.into(),
            entity,
        }
    }

    #[test]
    fn chunks_round_trip() {
        let mut w = ModelEventWriter::new(3);
        let mut all = Vec::new();
        let mut chunks = Vec::new();
        for f in 0..7u64 {
            let events: Vec<ModelEvent> = (0..f % 3)
                .map(|i| {
                    ev(
                        f,
                        if i == 0 { "frame" } else { "player.died" },
                        Some(i).filter(|i| *i > 0),
                    )
                })
                .collect();
            all.extend(events.clone());
            chunks.extend(w.push(&events));
        }
        chunks.extend(w.finish());
        assert_eq!(chunks.len(), 3);
        let mut back = Vec::new();
        for (i, c) in chunks.iter().enumerate() {
            let range = chunk::chunk_range(i, 3, 6);
            back.extend(decode_chunk_covering(c, range).unwrap().events);
        }
        assert_eq!(back, all);
    }

    #[test]
    fn refuses_the_nes_schema_and_wrong_frames() {
        let mut w = ModelEventWriter::new(1);
        let c = w.push(&[ev(0, "a", None)]).unwrap();
        assert!(decode_chunk_covering(&c, 1..2).is_err());
        assert!(crate::events::decode_chunk(&c).is_err());
        let mut bad = c.clone();
        bad[4] = 1;
        assert!(matches!(
            decode_chunk_covering(&bad, 0..1),
            Err(ChunkError::Schema { .. })
        ));
        assert!(decode_chunk_covering(&c[..12], 0..1).is_err());
    }
}
