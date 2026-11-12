//! The event stream: what happened inside each frame, at the PPU dot it
//! happened on.
//!
//! Chunks like the frame trace's: a 24-byte header (`GPEV`, schema u32,
//! first frame u64, frame count u32, body length u32) and an LZ4-compressed
//! body holding, for each frame in order, an event count (unsigned LEB128)
//! and that many records: kind (u8), the position's change from the event
//! before (zigzag LEB128), address (u16, little-endian) and value (u8). A
//! position is `scanline * 341 + dot`, and the first event in a frame counts
//! from 0. Storing the change rather than the scanline and dot makes the
//! same writes in consecutive frames the same bytes, which is what LZ4
//! finds: it took the event stream of the Super Mario Bros. baseline run from
//! 246KB to 58KB. See docs/formats/recording.md.
//!
//! An arcade board's frame isn't the NES's 312 lines of 341 dots, so a
//! recording of one counts positions in its own [`Geometry`]: Tapper's is
//! [`Geometry::MCR`], 481 lines of 512 pixels. The bytes are the same
//! format; which geometry a stream uses follows from the recording's
//! source (a romset), and the NES's is the default everywhere.

use crate::chunk::{self, ChunkError, Reader, put_varint};

pub const MAGIC: &[u8; 4] = b"GPEV";
/// The event chunk schema [`EventWriter`] writes and [`decode_chunk`] reads.
pub const SCHEMA: u32 = 1;
const HEADER: usize = 24;
/// The fewest bytes a record takes.
const MIN_RECORD: usize = 5;
/// Dots per scanline, the same in every region.
pub const DOTS: u16 = 341;
/// Scanlines per frame in the longest region (PAL and Dendy have 312, NTSC
/// 262). Every event's scanline is below it.
pub const MAX_SCANLINES: u16 = 312;
/// The most events a frame can hold: one per dot of the longest frame.
/// Events come from CPU accesses and interrupts, and the CPU runs one cycle
/// per three dots or more, so a real frame stays far below it.
pub const MAX_EVENTS_PER_FRAME: usize = MAX_SCANLINES as usize * DOTS as usize;
/// How a frame's positions are counted: `lines` lines of `dots` each.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Geometry {
    pub lines: u16,
    pub dots: u16,
}

impl Geometry {
    /// The NES: [`MAX_SCANLINES`] scanlines of [`DOTS`] dots.
    pub const NES: Self = Self {
        lines: MAX_SCANLINES,
        dots: DOTS,
    };
    /// Midway's MCR boards (Tapper): 480 lines of 512 pixels a frame, from
    /// the start of one VBLANK to the next, and one line more for what a
    /// CPU runs past the frame's end before the scheduler stops it.
    pub const MCR: Self = Self {
        lines: 481,
        dots: 512,
    };

    /// The most events a frame can hold: one per position.
    pub fn max_events(self) -> usize {
        usize::from(self.lines) * usize::from(self.dots)
    }

    fn positions(self) -> i64 {
        i64::from(self.lines) * i64::from(self.dots)
    }
}

/// The most bytes [`EventWriter`] writes for one event: the kind, a
/// position change of at most a frame's dots (3 bytes of zigzag LEB128),
/// the address and the value.
const MAX_RECORD: usize = 1 + 3 + 2 + 1;
/// The same for an arcade frame's larger positions (up to 4 bytes of
/// zigzag LEB128).
const MAX_RECORD_ARCADE: usize = 1 + 4 + 2 + 1;
/// The longest an unsigned LEB128 integer gets (a u64).
const MAX_VARINT: usize = 10;

/// The most body bytes `frames` frames can take: per frame an event count
/// and at most [`MAX_EVENTS_PER_FRAME`] records.
pub fn max_body(frames: u32) -> usize {
    (frames as usize).saturating_mul(MAX_VARINT + MAX_EVENTS_PER_FRAME * MAX_RECORD)
}

/// [`max_body`] in another geometry.
pub fn max_body_in(frames: u32, g: Geometry) -> usize {
    if g == Geometry::NES {
        return max_body(frames);
    }
    (frames as usize).saturating_mul(MAX_VARINT + g.max_events() * MAX_RECORD_ARCADE)
}

/// What happened. The numbers are the stored kind codes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(u8)]
pub enum EventKind {
    /// The CPU took the NMI vector; `addr` is the handler's address.
    Nmi = 0,
    /// The CPU took the IRQ vector; `addr` is the handler's address.
    Irq = 1,
    /// The PPU set the sprite 0 hit flag.
    Sprite0Hit = 2,
    /// A write to the cartridge changed which banks are mapped (or the
    /// mirroring); `addr` and `value` are the write's.
    BankSwitch = 3,
    /// A read of $4016 or $4017; `value` is what the CPU read.
    ControllerRead = 4,
    /// A write to $4016, the controller strobe.
    ControllerWrite = 5,
    /// A write to a PPU register, $2000-$2007, or to $4014 (OAM DMA).
    PpuWrite = 6,
    /// A write to an APU register: $4000-$4013, $4015 or $4017.
    ApuWrite = 7,
    /// Arcade: the main CPU read an input port; `addr` is the port and
    /// `value` what it read.
    PortRead = 8,
    /// Arcade: the main CPU wrote the sound board's latch; `addr` is the
    /// latch (0 to 3) and `value` the byte.
    SoundLatch = 9,
    /// Arcade: the main CPU fed the watchdog; `addr` is the port.
    Watchdog = 10,
    /// Arcade: the sound CPU wrote an AY-8910 register; `addr` is the chip
    /// times 256 plus the register, `value` the byte.
    AyWrite = 11,
    /// Arcade: the sound CPU took its interrupt; `addr` is the handler's
    /// address. (The main CPU's interrupts, from its CTC, are `irq`, with
    /// the vector the CTC gave in `value`.)
    SoundIrq = 12,
}

impl EventKind {
    pub const ALL: [Self; 13] = [
        Self::Nmi,
        Self::Irq,
        Self::Sprite0Hit,
        Self::BankSwitch,
        Self::ControllerRead,
        Self::ControllerWrite,
        Self::PpuWrite,
        Self::ApuWrite,
        Self::PortRead,
        Self::SoundLatch,
        Self::Watchdog,
        Self::AyWrite,
        Self::SoundIrq,
    ];

    /// The kinds an NES recording holds.
    pub const NES: [Self; 8] = [
        Self::Nmi,
        Self::Irq,
        Self::Sprite0Hit,
        Self::BankSwitch,
        Self::ControllerRead,
        Self::ControllerWrite,
        Self::PpuWrite,
        Self::ApuWrite,
    ];

    /// The kinds an arcade recording holds.
    pub const ARCADE: [Self; 6] = [
        Self::Irq,
        Self::PortRead,
        Self::SoundLatch,
        Self::Watchdog,
        Self::AyWrite,
        Self::SoundIrq,
    ];

    /// The name commands take and print, such as `ppu-write`.
    pub fn name(self) -> &'static str {
        match self {
            Self::Nmi => "nmi",
            Self::Irq => "irq",
            Self::Sprite0Hit => "sprite0-hit",
            Self::BankSwitch => "bank-switch",
            Self::ControllerRead => "controller-read",
            Self::ControllerWrite => "controller-write",
            Self::PpuWrite => "ppu-write",
            Self::ApuWrite => "apu-write",
            Self::PortRead => "port-read",
            Self::SoundLatch => "sound-latch",
            Self::Watchdog => "watchdog",
            Self::AyWrite => "ay-write",
            Self::SoundIrq => "sound-irq",
        }
    }

    pub fn from_name(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|k| k.name() == name)
    }

    pub fn from_code(code: u8) -> Option<Self> {
        Self::ALL.get(usize::from(code)).copied()
    }
}

/// One event. The frame is the one whose run it happened in: frame N's
/// events happened between the boundary that ends frame N-1 and the one that
/// ends frame N, so frame 0 (power-on) has none.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Event {
    pub frame: u64,
    pub kind: EventKind,
    pub scanline: u16,
    pub dot: u16,
    pub addr: u16,
    pub value: u8,
}

/// Builds event chunks from each frame's events, starting at frame 0.
pub struct EventWriter {
    geometry: Geometry,
    chunk_frames: u32,
    next_frame: u64,
    first_frame: u64,
    frames: u32,
    body: Vec<u8>,
}

impl EventWriter {
    pub fn new(chunk_frames: u32) -> Self {
        Self::with_geometry(chunk_frames, Geometry::NES)
    }

    /// A writer whose positions count in `geometry`.
    pub fn with_geometry(chunk_frames: u32, geometry: Geometry) -> Self {
        assert!(chunk_frames > 0, "a chunk holds at least one frame");
        Self {
            geometry,
            chunk_frames,
            next_frame: 0,
            first_frame: 0,
            frames: 0,
            body: Vec::new(),
        }
    }

    /// Adds the next frame's events, in the order they happened (their
    /// `frame` is ignored). Returns a finished chunk once it holds
    /// `chunk_frames` frames.
    pub fn push(&mut self, events: &[Event]) -> Option<Vec<u8>> {
        if self.frames == 0 {
            self.first_frame = self.next_frame;
            self.body.clear();
        }
        put_varint(&mut self.body, events.len() as u64);
        let mut last = 0i64;
        for e in events {
            let g = self.geometry;
            assert!(e.dot < g.dots, "a line has {} dots", g.dots);
            assert!(
                e.scanline < g.lines,
                "a frame has at most {} lines",
                g.lines
            );
            let at = i64::from(e.scanline) * i64::from(g.dots) + i64::from(e.dot);
            let change = at - last;
            last = at;
            self.body.push(e.kind as u8);
            put_varint(&mut self.body, ((change << 1) ^ (change >> 63)) as u64);
            self.body.extend_from_slice(&e.addr.to_le_bytes());
            self.body.push(e.value);
        }
        self.frames += 1;
        self.next_frame += 1;
        (self.frames == self.chunk_frames).then(|| self.seal())
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
        out.extend_from_slice(&(self.body.len() as u32).to_le_bytes());
        out.extend_from_slice(&packed);
        self.frames = 0;
        out
    }
}

/// A decoded chunk: frames `first_frame` to `first_frame + frames - 1`, and
/// their events in order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EventChunk {
    pub first_frame: u64,
    pub frames: u32,
    pub events: Vec<Event>,
}

/// The header's first frame, frame count and body length, and the
/// compressed body after it.
fn parse_header(bytes: &[u8]) -> Result<(u64, u32, usize, &[u8]), ChunkError> {
    let mut r = Reader { rest: bytes };
    if r.bytes(4, "magic")? != MAGIC {
        return Err(ChunkError::Magic("event"));
    }
    let version = r.u32("schema")?;
    if version != SCHEMA {
        return Err(ChunkError::Schema {
            what: "event",
            version,
        });
    }
    let first_frame = r.u64("first frame")?;
    let frames = r.u32("frame count")?;
    if frames == 0 {
        return Err(ChunkError::Invalid("a chunk with no frames".into()));
    }
    let len = r.u32("body length")? as usize;
    Ok((first_frame, frames, len, r.rest))
}

/// [`decode_chunk`] for a chunk the manifest says covers `expected`: a
/// header claiming other frames is refused before the body is decoded, and
/// a body longer than those frames can need ([`max_body`]) before it's
/// decompressed.
pub fn decode_chunk_covering(
    bytes: &[u8],
    expected: std::ops::Range<u64>,
) -> Result<EventChunk, ChunkError> {
    decode_chunk_covering_in(bytes, expected, Geometry::NES)
}

/// [`decode_chunk_covering`] for a stream counted in `geometry`.
pub fn decode_chunk_covering_in(
    bytes: &[u8],
    expected: std::ops::Range<u64>,
    geometry: Geometry,
) -> Result<EventChunk, ChunkError> {
    let (first_frame, frames, _, _) = parse_header(bytes)?;
    chunk::expect_frames("event", first_frame, frames, &expected)?;
    decode(bytes, max_body_in(frames, geometry), geometry)
}

/// Decodes a chunk on its own, bounded by its header's frame count.
/// Readers of a stored recording use [`decode_chunk_covering`].
pub fn decode_chunk(bytes: &[u8]) -> Result<EventChunk, ChunkError> {
    let (_, frames, _, _) = parse_header(bytes)?;
    decode(bytes, max_body(frames), Geometry::NES)
}

fn decode(bytes: &[u8], limit: usize, g: Geometry) -> Result<EventChunk, ChunkError> {
    let (first_frame, frames, len, rest) = parse_header(bytes)?;
    let end = first_frame
        .checked_add(u64::from(frames))
        .ok_or_else(|| ChunkError::Invalid(format!("frames from {first_frame} run past u64")))?;
    let body = chunk::decompress(rest, len, limit)?;
    let mut r = Reader { rest: &body };
    let mut events = Vec::new();
    let positions = g.positions();
    let most = g.max_events();
    for frame in first_frame..end {
        let n = r.len("event count")?;
        if n > most {
            return Err(ChunkError::Invalid(format!(
                "frame {frame} has {n} events, more than its {most} dots"
            )));
        }
        if n.saturating_mul(MIN_RECORD) > r.rest.len() {
            return Err(ChunkError::Truncated("events"));
        }
        let mut at = 0i64;
        for _ in 0..n {
            let code = r.u8("kind")?;
            let kind = EventKind::from_code(code)
                .ok_or_else(|| ChunkError::Invalid(format!("event kind {code}")))?;
            let zigzag = r.varint("position")?;
            let change = (zigzag >> 1) as i64 ^ -((zigzag & 1) as i64);
            at = at
                .checked_add(change)
                .filter(|at| (0..positions).contains(at))
                .ok_or_else(|| {
                    ChunkError::Invalid(format!(
                        "an event in frame {frame} is outside the frame's {} lines",
                        g.lines
                    ))
                })?;
            let (scanline, dot) = (at / i64::from(g.dots), at % i64::from(g.dots));
            let scanline = scanline as u16;
            events.push(Event {
                frame,
                kind,
                scanline,
                dot: dot as u16,
                addr: r.u16("address")?,
                value: r.u8("value")?,
            });
        }
    }
    if !r.rest.is_empty() {
        return Err(ChunkError::Trailing(r.rest.len()));
    }
    Ok(EventChunk {
        first_frame,
        frames,
        events,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ev(frame: u64, kind: EventKind, addr: u16, value: u8) -> Event {
        Event {
            frame,
            kind,
            scanline: ((frame * 97 + u64::from(addr)) % 262) as u16,
            dot: (u64::from(addr) * 31 % 341) as u16,
            addr,
            value,
        }
    }

    #[test]
    fn chunks_round_trip() {
        let mut w = EventWriter::new(3);
        let mut all = Vec::new();
        let mut chunks = Vec::new();
        for f in 0..7u64 {
            let events: Vec<Event> = (0..f)
                .map(|i| {
                    ev(
                        f,
                        EventKind::ALL[(i % 8) as usize],
                        0x2000 + i as u16,
                        i as u8,
                    )
                })
                .collect();
            all.extend(events.clone());
            chunks.extend(w.push(&events));
        }
        chunks.extend(w.finish());
        assert_eq!(chunks.len(), 3);
        let decoded: Vec<_> = chunks.iter().map(|c| decode_chunk(c).unwrap()).collect();
        assert_eq!(
            decoded
                .iter()
                .map(|c| (c.first_frame, c.frames))
                .collect::<Vec<_>>(),
            [(0, 3), (3, 3), (6, 1)]
        );
        let back: Vec<Event> = decoded.into_iter().flat_map(|c| c.events).collect();
        assert_eq!(back, all);
    }

    #[test]
    fn kinds_have_stable_codes_and_names() {
        for (i, k) in EventKind::ALL.into_iter().enumerate() {
            assert_eq!(k as usize, i);
            assert_eq!(EventKind::from_code(i as u8), Some(k));
            assert_eq!(EventKind::from_name(k.name()), Some(k));
        }
        assert_eq!(EventKind::from_code(13), None);
        for k in EventKind::NES.into_iter().chain(EventKind::ARCADE) {
            assert!(EventKind::ALL.contains(&k));
        }
        assert_eq!(EventKind::from_name("ppu"), None);
    }

    #[test]
    fn rejects_bad_chunks() {
        let mut w = EventWriter::new(1);
        let c = w.push(&[ev(0, EventKind::Nmi, 0x8082, 0)]).unwrap();
        let mut bad = c.clone();
        bad[0] = b'X';
        assert_eq!(decode_chunk(&bad), Err(ChunkError::Magic("event")));
        let mut bad = c.clone();
        bad[4] = 2;
        assert!(matches!(decode_chunk(&bad), Err(ChunkError::Schema { .. })));
        assert_eq!(
            decode_chunk(&c[..10]),
            Err(ChunkError::Truncated("first frame"))
        );
        assert_eq!(decode_chunk_covering(&c, 0..1).unwrap().events.len(), 1);
        assert!(decode_chunk_covering(&c, 1..2).is_err());
        // A first frame near u64::MAX is an error, not an overflow.
        let mut bad = c.clone();
        bad[8..16].copy_from_slice(&u64::MAX.to_le_bytes());
        assert!(matches!(decode_chunk(&bad), Err(ChunkError::Invalid(_))));
        // A body length the frame can't need is refused before decompressing.
        let mut bad = c.clone();
        bad[20..24].copy_from_slice(&u32::MAX.to_le_bytes());
        assert!(matches!(
            decode_chunk_covering(&bad, 0..1),
            Err(ChunkError::Invalid(_))
        ));
    }

    /// An arcade stream counts 481 lines of 512 dots; the same bytes read
    /// with the NES's geometry would put its late events past the frame.
    #[test]
    fn an_arcade_stream_round_trips_in_its_geometry() {
        let g = Geometry::MCR;
        let mut w = EventWriter::with_geometry(2, g);
        let events = vec![
            Event {
                frame: 0,
                kind: EventKind::PortRead,
                scanline: 3,
                dot: 500,
                addr: 1,
                value: 0xef,
            },
            Event {
                frame: 0,
                kind: EventKind::AyWrite,
                scanline: 480,
                dot: 511,
                addr: 0x0107,
                value: 0x38,
            },
        ];
        assert!(w.push(&events).is_none());
        let c = w.push(&[]).unwrap();
        let back = decode_chunk_covering_in(&c, 0..2, g).unwrap();
        assert_eq!(back.events, events);
        assert!(decode_chunk_covering(&c, 0..2).is_err());
    }

    /// Positions outside a frame, or past i64, are errors rather than
    /// overflows or wrong scanlines.
    #[test]
    fn refuses_positions_outside_the_frame() {
        let chunk = |zigzags: &[u64]| {
            let mut body = Vec::new();
            put_varint(&mut body, zigzags.len() as u64);
            for z in zigzags {
                body.push(0);
                put_varint(&mut body, *z);
                body.extend_from_slice(&[0, 0, 0]);
            }
            let mut out = MAGIC.to_vec();
            out.extend_from_slice(&SCHEMA.to_le_bytes());
            out.extend_from_slice(&0u64.to_le_bytes());
            out.extend_from_slice(&1u32.to_le_bytes());
            out.extend_from_slice(&(body.len() as u32).to_le_bytes());
            out.extend_from_slice(&chunk::compress(&body));
            out
        };
        let last = (u64::from(MAX_SCANLINES) * u64::from(DOTS) - 1) * 2;
        let ok = decode_chunk(&chunk(&[last])).unwrap();
        assert_eq!((ok.events[0].scanline, ok.events[0].dot), (311, 340));
        for bad in [
            vec![last + 2],
            vec![1],
            vec![u64::MAX - 1, u64::MAX - 1],
            vec![u64::MAX, u64::MAX],
        ] {
            assert!(
                matches!(decode_chunk(&chunk(&bad)), Err(ChunkError::Invalid(_))),
                "{bad:?}"
            );
        }
    }
}
