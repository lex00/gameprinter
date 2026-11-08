//! Frame traces: the runtime's full per-frame state, stored in chunks.
//!
//! A chunk is a 56-byte header (`GPTR`, schema u32, first frame u64, frame
//! count u32, the field table's BLAKE3 hash, body length u32) and an
//! LZ4-compressed body: the first frame's state in full (a keyframe), then
//! for each later frame the byte ranges that differ from the frame before.
//! The state is a runtime's canonical state, and the field table
//! ([`FieldTable`], its own object in the store) says how to split it into
//! named fields, so a reader needs no runtime code. See
//! docs/formats/recording.md.

use crate::chunk::{self, ChunkError, Reader, put_varint};
use crate::hash::Hash;

pub const MAGIC: &[u8; 4] = b"GPTR";
/// The trace chunk schema [`TraceWriter`] writes and [`decode_chunk`] reads.
pub const SCHEMA: u32 = 1;
const HEADER: usize = 56;

pub const TABLE_MAGIC: &[u8; 4] = b"GPFT";
/// The field table encoding's own version.
pub const TABLE_SCHEMA: u32 = 1;

/// Two differing bytes separated by at most this many equal ones go in one
/// range: a new range costs at least two bytes of header.
const MERGE_GAP: usize = 2;

/// The largest region field a state may hold, 4 MiB. NES 2.0 headers can
/// declare up to 2 MiB of PRG RAM or CHR RAM; nothing recorded so far has
/// more than 8 KiB.
pub const MAX_REGION: usize = 4 << 20;

/// The longest an unsigned LEB128 integer gets (a u64).
const MAX_VARINT: usize = 10;

/// How many bytes a field takes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Width {
    /// A fixed number of bytes: a little-endian integer or a fixed-size array.
    Fixed(u32),
    /// A u32 byte count, then that many bytes.
    Region,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FieldDef {
    pub name: String,
    pub width: Width,
}

/// The layout of a runtime's state: an 8-byte header (`state_magic`, then
/// `state_version` as a u32) and the fields in order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FieldTable {
    pub state_magic: [u8; 4],
    pub state_version: u32,
    pub fields: Vec<FieldDef>,
}

impl FieldTable {
    /// `GPFT`, the table schema (u32), the state magic and version, the
    /// field count (u32), then per field: name length (u8), name, kind (u8:
    /// 0 fixed, 1 region) and width (u32, 0 for a region).
    pub fn encode(&self) -> Vec<u8> {
        let mut out = Vec::new();
        out.extend_from_slice(TABLE_MAGIC);
        out.extend_from_slice(&TABLE_SCHEMA.to_le_bytes());
        out.extend_from_slice(&self.state_magic);
        out.extend_from_slice(&self.state_version.to_le_bytes());
        out.extend_from_slice(&(self.fields.len() as u32).to_le_bytes());
        for f in &self.fields {
            let name = f.name.as_bytes();
            out.push(u8::try_from(name.len()).expect("field names are short"));
            out.extend_from_slice(name);
            let (kind, width) = match f.width {
                Width::Fixed(n) => (0, n),
                Width::Region => (1, 0),
            };
            out.push(kind);
            out.extend_from_slice(&width.to_le_bytes());
        }
        out
    }

    pub fn hash(&self) -> Hash {
        Hash::of(&self.encode())
    }

    pub fn decode(bytes: &[u8]) -> Result<Self, ChunkError> {
        let mut r = Reader { rest: bytes };
        if r.bytes(4, "field table magic")? != TABLE_MAGIC {
            return Err(ChunkError::Magic("field table"));
        }
        let version = r.u32("field table schema")?;
        if version != TABLE_SCHEMA {
            return Err(ChunkError::Schema {
                what: "field table",
                version,
            });
        }
        let state_magic = r.bytes(4, "state magic")?.try_into().expect("4");
        let state_version = r.u32("state version")?;
        let count = r.u32("field count")?;
        let mut fields = Vec::new();
        for _ in 0..count {
            let n = r.u8("field name length")?;
            let name = std::str::from_utf8(r.bytes(n.into(), "field name")?)
                .map_err(|_| ChunkError::Invalid("a field name isn't UTF-8".into()))?
                .to_owned();
            let width = match (r.u8("field kind")?, r.u32("field width")?) {
                (0, n) => Width::Fixed(n),
                (1, _) => Width::Region,
                (k, _) => return Err(ChunkError::Invalid(format!("field kind {k}"))),
            };
            fields.push(FieldDef { name, width });
        }
        if !r.rest.is_empty() {
            return Err(ChunkError::Trailing(r.rest.len()));
        }
        Ok(Self {
            state_magic,
            state_version,
            fields,
        })
    }

    /// The most bytes a state following this table can take: its 8-byte
    /// header, every fixed field, and for each region a u32 length and
    /// [`MAX_REGION`] bytes.
    pub fn max_state_len(&self) -> usize {
        self.fields.iter().fold(8usize, |n, f| {
            n.saturating_add(match f.width {
                Width::Fixed(w) => w as usize,
                Width::Region => 4 + MAX_REGION,
            })
        })
    }

    /// Splits a state into its named fields, in table order.
    pub fn split<'a>(&'a self, state: &'a [u8]) -> Result<Vec<(&'a str, &'a [u8])>, ChunkError> {
        let mut r = Reader { rest: state };
        if r.bytes(4, "state magic")? != self.state_magic {
            return Err(ChunkError::Magic("state"));
        }
        let version = r.u32("state version")?;
        if version != self.state_version {
            return Err(ChunkError::Schema {
                what: "state",
                version,
            });
        }
        let mut out = Vec::with_capacity(self.fields.len());
        for f in &self.fields {
            let n = match f.width {
                Width::Fixed(n) => n as usize,
                Width::Region => {
                    let n = r.u32("region length")? as usize;
                    if n > MAX_REGION {
                        return Err(ChunkError::Invalid(format!(
                            "region {} is {n} bytes, more than {MAX_REGION}",
                            f.name
                        )));
                    }
                    n
                }
            };
            out.push((f.name.as_str(), r.bytes(n, "a field")?));
        }
        if !r.rest.is_empty() {
            return Err(ChunkError::Trailing(r.rest.len()));
        }
        Ok(out)
    }
}

/// What a chunk's header says.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ChunkHeader {
    pub first_frame: u64,
    pub frames: u32,
    /// The hash of the field table the states follow.
    pub fields: Hash,
}

/// Builds trace chunks from one state per frame, starting at frame 0.
pub struct TraceWriter {
    chunk_frames: u32,
    fields: Hash,
    next_frame: u64,
    first_frame: u64,
    frames: u32,
    body: Vec<u8>,
    prev: Vec<u8>,
}

impl TraceWriter {
    pub fn new(chunk_frames: u32, fields: Hash) -> Self {
        assert!(chunk_frames > 0, "a chunk holds at least one frame");
        Self {
            chunk_frames,
            fields,
            next_frame: 0,
            first_frame: 0,
            frames: 0,
            body: Vec::new(),
            prev: Vec::new(),
        }
    }

    /// Adds the next frame's state. Returns a finished chunk once it holds
    /// `chunk_frames` frames. Every state in a chunk must have the same
    /// length, as a runtime's state does for one cartridge.
    pub fn push(&mut self, state: &[u8]) -> Result<Option<Vec<u8>>, ChunkError> {
        if self.frames == 0 {
            self.first_frame = self.next_frame;
            self.body.clear();
            put_varint(&mut self.body, state.len() as u64);
            self.body.extend_from_slice(state);
        } else {
            if state.len() != self.prev.len() {
                return Err(ChunkError::Invalid(format!(
                    "frame {} has {} bytes of state, the frame before it {}",
                    self.next_frame,
                    state.len(),
                    self.prev.len()
                )));
            }
            put_delta(&mut self.body, &self.prev, state);
        }
        self.prev.clear();
        self.prev.extend_from_slice(state);
        self.frames += 1;
        self.next_frame += 1;
        Ok((self.frames == self.chunk_frames).then(|| self.seal()))
    }

    /// The last, partly filled chunk, if any frames are left in it.
    pub fn finish(mut self) -> Option<Vec<u8>> {
        (self.frames > 0).then(|| self.seal())
    }

    fn seal(&mut self) -> Vec<u8> {
        let packed = chunk::compress(&self.body);
        let mut out = Vec::with_capacity(HEADER + packed.len());
        out.extend_from_slice(MAGIC);
        out.extend_from_slice(&SCHEMA.to_le_bytes());
        out.extend_from_slice(&self.first_frame.to_le_bytes());
        out.extend_from_slice(&self.frames.to_le_bytes());
        out.extend_from_slice(self.fields.as_bytes());
        out.extend_from_slice(&(self.body.len() as u32).to_le_bytes());
        out.extend_from_slice(&packed);
        self.frames = 0;
        out
    }
}

/// The ranges of `cur` that differ from `prev`: a count, then per range the
/// bytes skipped since the end of the previous range, its length, and its
/// bytes. Counts and lengths are unsigned LEB128.
fn put_delta(out: &mut Vec<u8>, prev: &[u8], cur: &[u8]) {
    let mut ranges = Vec::new();
    let mut i = 0;
    while i < cur.len() {
        if cur[i] == prev[i] {
            i += 1;
            continue;
        }
        let start = i;
        let mut end = i + 1;
        let mut j = end;
        while j < cur.len() && j - end <= MERGE_GAP {
            if cur[j] != prev[j] {
                end = j + 1;
            }
            j += 1;
        }
        ranges.push(start..end);
        i = end;
    }
    put_varint(out, ranges.len() as u64);
    let mut last = 0;
    for r in ranges {
        put_varint(out, (r.start - last) as u64);
        put_varint(out, r.len() as u64);
        out.extend_from_slice(&cur[r.clone()]);
        last = r.end;
    }
}

/// Reads a chunk's header without decompressing it.
pub fn header(bytes: &[u8]) -> Result<ChunkHeader, ChunkError> {
    parse_header(bytes).map(|(h, _)| h)
}

/// The header, and the body's uncompressed length.
fn parse_header(bytes: &[u8]) -> Result<(ChunkHeader, usize), ChunkError> {
    let mut r = Reader { rest: bytes };
    if r.bytes(4, "magic")? != MAGIC {
        return Err(ChunkError::Magic("trace"));
    }
    let version = r.u32("schema")?;
    if version != SCHEMA {
        return Err(ChunkError::Schema {
            what: "trace",
            version,
        });
    }
    let header = ChunkHeader {
        first_frame: r.u64("first frame")?,
        frames: r.u32("frame count")?,
        fields: Hash::from_bytes(r.bytes(32, "field table hash")?.try_into().expect("32")),
    };
    if header.frames == 0 {
        return Err(ChunkError::Invalid("a chunk with no frames".into()));
    }
    Ok((header, r.u32("body length")? as usize))
}

/// A decoded chunk: every frame's full state, from `header.first_frame` on.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TraceChunk {
    pub header: ChunkHeader,
    pub states: Vec<Vec<u8>>,
}

/// The most body bytes `frames` states of at most `max_state` bytes can
/// take as [`TraceWriter`] writes them: the keyframe and its length, then
/// per later frame a range count and at most one range per
/// `MERGE_GAP + 2` bytes of state, each with a skip, a length and its bytes.
pub fn max_body(frames: u32, max_state: usize) -> usize {
    let ranges = max_state / (MERGE_GAP + 2) + 1;
    let delta = MAX_VARINT
        .saturating_add(max_state)
        .saturating_add(ranges.saturating_mul(2 * MAX_VARINT));
    MAX_VARINT
        .saturating_add(max_state)
        .saturating_add(delta.saturating_mul(frames.saturating_sub(1) as usize))
}

/// Decodes a chunk the manifest says covers `expected`, whose states follow
/// `table`. Everything is checked against the manifest and the table before
/// it's allocated: the header's frames, its field table, the body length
/// ([`max_body`], and what the compressed bytes can hold), and every
/// frame's state, which must split into the table's fields.
pub fn decode_chunk_covering(
    bytes: &[u8],
    expected: std::ops::Range<u64>,
    table: &FieldTable,
) -> Result<TraceChunk, ChunkError> {
    let mut states = Vec::new();
    let header = decode_each(bytes, &expected, table, |_, state| {
        states.push(state.to_vec());
    })?;
    Ok(TraceChunk { header, states })
}

/// The state at `frame` from a chunk checked as in
/// [`decode_chunk_covering`], keeping only that one state.
pub fn decode_state(
    bytes: &[u8],
    expected: std::ops::Range<u64>,
    table: &FieldTable,
    frame: u64,
) -> Result<Vec<u8>, ChunkError> {
    if !expected.contains(&frame) {
        return Err(ChunkError::Invalid(format!(
            "frame {frame} isn't in the chunk for frames {expected:?}"
        )));
    }
    let mut out = None;
    decode_each(bytes, &expected, table, |f, state| {
        if f == frame {
            out = Some(state.to_vec());
        }
    })?;
    Ok(out.expect("the chunk covers the frame"))
}

/// Decodes a chunk, checked as in [`decode_chunk_covering`], and calls
/// `each` with every frame's number and state in order.
pub fn decode_each(
    bytes: &[u8],
    expected: &std::ops::Range<u64>,
    table: &FieldTable,
    mut each: impl FnMut(u64, &[u8]),
) -> Result<ChunkHeader, ChunkError> {
    let (header, len) = parse_header(bytes)?;
    chunk::expect_frames("trace", header.first_frame, header.frames, expected)?;
    if header.fields != table.hash() {
        return Err(ChunkError::Invalid(format!(
            "the chunk follows field table {}, not the manifest's {}",
            header.fields,
            table.hash()
        )));
    }
    let max_state = table.max_state_len();
    let body = chunk::decompress(&bytes[HEADER..], len, max_body(header.frames, max_state))?;
    let mut r = Reader { rest: &body };
    let n = r.len("keyframe length")?;
    if n > max_state {
        return Err(ChunkError::Invalid(format!(
            "a {n}-byte keyframe, more than the field table's {max_state}"
        )));
    }
    let mut state = r.bytes(n, "keyframe")?.to_vec();
    table.split(&state)?;
    each(header.first_frame, &state);
    for frame in header.first_frame + 1..expected.end {
        apply_delta(&mut r, &mut state)?;
        table.split(&state)?;
        each(frame, &state);
    }
    if !r.rest.is_empty() {
        return Err(ChunkError::Trailing(r.rest.len()));
    }
    Ok(header)
}

/// Applies one frame's ranges to the state before it.
fn apply_delta(r: &mut Reader<'_>, state: &mut [u8]) -> Result<(), ChunkError> {
    let ranges = r.len("range count")?;
    let mut at = 0usize;
    for _ in 0..ranges {
        at = at
            .checked_add(r.len("range offset")?)
            .ok_or_else(|| ChunkError::Invalid("range offset overflows".into()))?;
        let len = r.len("range length")?;
        let bytes = r.bytes(len, "range bytes")?;
        let dst = at
            .checked_add(len)
            .and_then(|end| state.get_mut(at..end))
            .ok_or_else(|| ChunkError::Invalid("a range runs past the state".into()))?;
        dst.copy_from_slice(bytes);
        at += len;
    }
    Ok(())
}

/// Decodes a chunk on its own, for tests of the writer. Without a field
/// table nothing bounds the state, so the states it builds can take the
/// square of the body's size; readers use [`decode_chunk_covering`].
#[cfg(test)]
fn decode_chunk(bytes: &[u8]) -> Result<TraceChunk, ChunkError> {
    let (header, len) = parse_header(bytes)?;
    let body = chunk::decompress(&bytes[HEADER..], len, usize::MAX)?;
    let mut r = Reader { rest: &body };
    let n = r.len("keyframe length")?;
    let mut state = r.bytes(n, "keyframe")?.to_vec();
    // Every later frame takes at least its range count's byte.
    if header.frames as usize - 1 > r.rest.len() {
        return Err(ChunkError::Truncated("frames"));
    }
    let mut states = Vec::with_capacity(header.frames as usize);
    states.push(state.clone());
    for _ in 1..header.frames {
        apply_delta(&mut r, &mut state)?;
        states.push(state.clone());
    }
    if !r.rest.is_empty() {
        return Err(ChunkError::Trailing(r.rest.len()));
    }
    Ok(TraceChunk { header, states })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn table() -> FieldTable {
        FieldTable {
            state_magic: *b"TEST",
            state_version: 3,
            fields: vec![
                FieldDef {
                    name: "a".into(),
                    width: Width::Fixed(2),
                },
                FieldDef {
                    name: "mem".into(),
                    width: Width::Region,
                },
            ],
        }
    }

    fn state(a: u16, mem: &[u8]) -> Vec<u8> {
        let mut s = b"TEST".to_vec();
        s.extend_from_slice(&3u32.to_le_bytes());
        s.extend_from_slice(&a.to_le_bytes());
        s.extend_from_slice(&(mem.len() as u32).to_le_bytes());
        s.extend_from_slice(mem);
        s
    }

    #[test]
    fn field_table_round_trips_and_splits_states() {
        let t = table();
        assert_eq!(FieldTable::decode(&t.encode()).unwrap(), t);
        let s = state(0x1234, &[9, 8, 7]);
        assert_eq!(
            t.split(&s).unwrap(),
            vec![("a", &[0x34, 0x12][..]), ("mem", &[9, 8, 7][..])]
        );
        assert_eq!(
            t.split(&s[..s.len() - 1]),
            Err(ChunkError::Truncated("a field"))
        );
        let mut other = s.clone();
        other[4] = 4;
        assert!(matches!(t.split(&other), Err(ChunkError::Schema { .. })));
        // A region may hold MAX_REGION bytes and no more.
        let big = vec![0u8; MAX_REGION];
        assert!(t.split(&state(1, &big)).is_ok());
        let bigger = vec![0u8; MAX_REGION + 1];
        assert!(matches!(
            t.split(&state(1, &bigger)),
            Err(ChunkError::Invalid(_))
        ));
    }

    #[test]
    fn chunks_round_trip_every_frame() {
        let fields = table().hash();
        let mut w = TraceWriter::new(4, fields);
        let mut states = Vec::new();
        let mut chunks = Vec::new();
        for f in 0..10u16 {
            let mut mem = vec![0u8; 100];
            mem[usize::from(f) * 3] = f as u8 + 1;
            mem[99] = (f % 3) as u8;
            let s = state(f * 7, &mem);
            states.push(s.clone());
            if let Some(c) = w.push(&s).unwrap() {
                chunks.push(c);
            }
        }
        chunks.extend(w.finish());
        assert_eq!(chunks.len(), 3);
        let decoded: Vec<_> = chunks.iter().map(|c| decode_chunk(c).unwrap()).collect();
        assert_eq!(
            decoded
                .iter()
                .map(|d| d.header.first_frame)
                .collect::<Vec<_>>(),
            [0, 4, 8]
        );
        assert_eq!(
            decoded.iter().map(|d| d.header.frames).collect::<Vec<_>>(),
            [4, 4, 2]
        );
        assert!(decoded.iter().all(|d| d.header.fields == fields));
        let all: Vec<_> = decoded.into_iter().flat_map(|d| d.states).collect();
        assert_eq!(all, states);
    }

    #[test]
    fn deltas_hold_only_what_changed() {
        let prev = vec![0u8; 1000];
        let mut cur = prev.clone();
        cur[10] = 1;
        cur[12] = 1; // two bytes apart: merged into one range
        cur[500] = 2;
        let mut out = Vec::new();
        put_delta(&mut out, &prev, &cur);
        // count, then (skip 10, len 3, 3 bytes), then (skip 487 in two bytes, len 1, 1 byte)
        assert_eq!(out.len(), 1 + (1 + 1 + 3) + (2 + 1 + 1));
    }

    #[test]
    fn rejects_bad_chunks() {
        let mut w = TraceWriter::new(2, Hash::of(b"t"));
        w.push(&[1, 2, 3]).unwrap();
        assert!(matches!(w.push(&[1, 2]), Err(ChunkError::Invalid(_))));
        let mut w = TraceWriter::new(2, Hash::of(b"t"));
        w.push(&[1, 2, 3]).unwrap();
        let c = w.push(&[1, 2, 4]).unwrap().unwrap();
        assert_eq!(
            decode_chunk(&c[..20]),
            Err(ChunkError::Truncated("field table hash"))
        );
        let mut bad = c.clone();
        bad[0] = b'X';
        assert_eq!(decode_chunk(&bad), Err(ChunkError::Magic("trace")));
        let mut bad = c.clone();
        bad[4] = 9;
        assert!(matches!(decode_chunk(&bad), Err(ChunkError::Schema { .. })));
        let mut bad = c.clone();
        bad.truncate(bad.len() - 1);
        assert!(decode_chunk(&bad).is_err());
        // The covering decode checks the states against the field table too.
        let t = table();
        let mut w = TraceWriter::new(2, t.hash());
        w.push(&state(1, &[1, 2, 3])).unwrap();
        let c = w.push(&state(2, &[1, 2, 4])).unwrap().unwrap();
        assert_eq!(decode_chunk_covering(&c, 0..2, &t).unwrap().states.len(), 2);
        assert_eq!(decode_state(&c, 0..2, &t, 1).unwrap(), state(2, &[1, 2, 4]));
        assert!(decode_state(&c, 0..2, &t, 2).is_err());
        assert!(decode_chunk_covering(&c, 0..3, &t).is_err());
        assert!(decode_chunk_covering(&c, 2..4, &t).is_err());
        let mut other = t.clone();
        other.state_version = 4;
        assert!(decode_chunk_covering(&c, 0..2, &other).is_err());
        // States that don't follow the table are refused.
        let mut w = TraceWriter::new(2, t.hash());
        w.push(&[1, 2, 3]).unwrap();
        let c = w.push(&[1, 2, 4]).unwrap().unwrap();
        assert!(decode_chunk_covering(&c, 0..2, &t).is_err());
    }

    /// A tiny chunk whose header claims far more frames than its slot in
    /// the manifest is refused before it's decoded: decoding it would build
    /// every claimed frame.
    #[test]
    fn refuses_a_header_claiming_more_frames_than_expected() {
        let t = table();
        let mut w = TraceWriter::new(2, t.hash());
        w.push(&state(0, &[0; 4096])).unwrap();
        let c = w.push(&state(0, &[0; 4096])).unwrap().unwrap();
        let mut forged = c.clone();
        forged[16..20].copy_from_slice(&80_000u32.to_le_bytes());
        let err = decode_chunk_covering(&forged, 0..2, &t).unwrap_err();
        assert!(matches!(err, ChunkError::Invalid(_)), "{err}");
        // The same header on its own is refused before it's decompressed:
        // 80,000 frames need a range count each.
        assert!(decode_chunk(&forged).is_err());
    }

    /// A body length the frames can't need, or that the compressed bytes
    /// can't hold, is refused before anything is allocated for it.
    #[test]
    fn refuses_a_body_longer_than_the_frames_need() {
        let t = table();
        let mut w = TraceWriter::new(2, t.hash());
        w.push(&state(0, &[0; 16])).unwrap();
        let c = w.push(&state(0, &[0; 16])).unwrap().unwrap();
        for len in [u32::MAX, 1 << 30, max_body(2, t.max_state_len()) as u32 + 1] {
            let mut forged = c.clone();
            forged[52..56].copy_from_slice(&len.to_le_bytes());
            let err = decode_chunk_covering(&forged, 0..2, &t).unwrap_err();
            assert!(matches!(err, ChunkError::Invalid(_)), "{len}: {err}");
        }
        assert_eq!(t.max_state_len(), 8 + 2 + 4 + MAX_REGION);
    }
}
