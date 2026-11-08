//! What frame trace and event stream chunks share: a fixed header followed by
//! an LZ4-compressed body, unsigned LEB128 integers inside the body, and the
//! rule for which frames each chunk covers. See docs/formats/recording.md.

/// Frames per chunk unless a recording asks for another number.
pub const DEFAULT_CHUNK_FRAMES: u32 = 64;

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum ChunkError {
    #[error("not a {0} chunk (wrong magic)")]
    Magic(&'static str),
    #[error("{what} chunk schema {version} is not supported")]
    Schema { what: &'static str, version: u32 },
    #[error("chunk is truncated in {0}")]
    Truncated(&'static str),
    #[error("chunk body doesn't decompress: {0}")]
    Compression(String),
    #[error("chunk body has {0} bytes after its last frame")]
    Trailing(usize),
    #[error("chunk is inconsistent: {0}")]
    Invalid(String),
}

/// The frames chunk `index` covers, for a recording of `frames` frames:
/// chunks cover frames 0 to `frames` in order, `chunk_frames` each, and the
/// last one stops at `frames`.
pub fn chunk_range(index: usize, chunk_frames: u32, frames: u64) -> std::ops::Range<u64> {
    let start = index as u64 * u64::from(chunk_frames);
    start..(start + u64::from(chunk_frames)).min(frames + 1)
}

/// Refuses a chunk whose header doesn't claim exactly `expected`, the frames
/// the manifest gives it. Checked before anything is decompressed, so a
/// damaged header can't make a decoder build more frames than the
/// recording has.
pub(crate) fn expect_frames(
    what: &str,
    first_frame: u64,
    frames: u32,
    expected: &std::ops::Range<u64>,
) -> Result<(), ChunkError> {
    if first_frame != expected.start || expected.end - expected.start != u64::from(frames) {
        return Err(ChunkError::Invalid(format!(
            "the {what} chunk covers {frames} frames from {first_frame}, but the manifest gives it frames {expected:?}"
        )));
    }
    Ok(())
}

/// How many chunks cover frames 0 to `frames`.
pub fn chunk_count(chunk_frames: u32, frames: u64) -> usize {
    (frames + 1).div_ceil(u64::from(chunk_frames)) as usize
}

/// LZ4 block format, no frame and no size prefix: the header carries the
/// uncompressed size.
pub(crate) fn compress(body: &[u8]) -> Vec<u8> {
    lz4_flex::block::compress(body)
}

/// The most bytes an LZ4 block of `compressed` bytes can decompress to. A
/// match's length takes one more byte for every 255 bytes it copies, so no
/// block expands by more than 255 times, plus a few bytes for its last
/// token. Checked before anything is allocated, so a header can't ask for a
/// body its compressed bytes couldn't hold.
pub fn max_decompressed(compressed: usize) -> usize {
    compressed.saturating_mul(255).saturating_add(64)
}

/// Decompresses a body the header says is `len` bytes, refusing a `len`
/// over `limit` (what the manifest allows the chunk) or over what
/// `data` could decompress to, before allocating anything.
pub(crate) fn decompress(data: &[u8], len: usize, limit: usize) -> Result<Vec<u8>, ChunkError> {
    if len > limit {
        return Err(ChunkError::Invalid(format!(
            "the header gives the body {len} bytes, more than the {limit} its frames can need"
        )));
    }
    if len > max_decompressed(data.len()) {
        return Err(ChunkError::Invalid(format!(
            "the header gives the body {len} bytes, more than {} compressed bytes can hold",
            data.len()
        )));
    }
    let out = lz4_flex::block::decompress(data, len)
        .map_err(|e| ChunkError::Compression(e.to_string()))?;
    if out.len() != len {
        return Err(ChunkError::Compression(format!(
            "{} bytes, header says {len}",
            out.len()
        )));
    }
    Ok(out)
}

pub(crate) fn put_varint(out: &mut Vec<u8>, mut v: u64) {
    loop {
        let byte = (v & 0x7f) as u8;
        v >>= 7;
        if v == 0 {
            out.push(byte);
            return;
        }
        out.push(byte | 0x80);
    }
}

/// Reads fields from a chunk header or body in order.
pub(crate) struct Reader<'a> {
    pub rest: &'a [u8],
}

impl<'a> Reader<'a> {
    pub fn bytes(&mut self, n: usize, what: &'static str) -> Result<&'a [u8], ChunkError> {
        let out = self.rest.get(..n).ok_or(ChunkError::Truncated(what))?;
        self.rest = &self.rest[n..];
        Ok(out)
    }

    pub fn u8(&mut self, what: &'static str) -> Result<u8, ChunkError> {
        Ok(self.bytes(1, what)?[0])
    }

    pub fn u16(&mut self, what: &'static str) -> Result<u16, ChunkError> {
        Ok(u16::from_le_bytes(
            self.bytes(2, what)?.try_into().expect("2"),
        ))
    }

    pub fn u32(&mut self, what: &'static str) -> Result<u32, ChunkError> {
        Ok(u32::from_le_bytes(
            self.bytes(4, what)?.try_into().expect("4"),
        ))
    }

    pub fn u64(&mut self, what: &'static str) -> Result<u64, ChunkError> {
        Ok(u64::from_le_bytes(
            self.bytes(8, what)?.try_into().expect("8"),
        ))
    }

    pub fn varint(&mut self, what: &'static str) -> Result<u64, ChunkError> {
        let mut v = 0u64;
        for shift in (0..64).step_by(7) {
            let byte = self.u8(what)?;
            v |= u64::from(byte & 0x7f) << shift;
            if byte & 0x80 == 0 {
                return Ok(v);
            }
        }
        Err(ChunkError::Invalid(format!(
            "{what}: integer longer than 64 bits"
        )))
    }

    /// A varint that must fit in memory, such as a length.
    pub fn len(&mut self, what: &'static str) -> Result<usize, ChunkError> {
        usize::try_from(self.varint(what)?)
            .map_err(|_| ChunkError::Invalid(format!("{what} is too large")))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn varints_round_trip() {
        for v in [
            0,
            1,
            127,
            128,
            300,
            16_383,
            16_384,
            u64::from(u32::MAX),
            u64::MAX,
        ] {
            let mut out = Vec::new();
            put_varint(&mut out, v);
            let mut r = Reader { rest: &out };
            assert_eq!(r.varint("v").unwrap(), v);
            assert!(r.rest.is_empty());
        }
        assert_eq!(
            Reader { rest: &[0x80] }.varint("v"),
            Err(ChunkError::Truncated("v"))
        );
    }

    #[test]
    fn chunks_cover_frames_0_to_the_last() {
        assert_eq!(chunk_count(64, 0), 1);
        assert_eq!(chunk_count(64, 63), 1);
        assert_eq!(chunk_count(64, 64), 2);
        assert_eq!(chunk_count(64, 1800), 29);
        assert_eq!(chunk_range(0, 64, 1800), 0..64);
        assert_eq!(chunk_range(28, 64, 1800), 1792..1801);
    }

    #[test]
    fn compression_round_trips_and_checks_the_length() {
        let body: Vec<u8> = (0..5000u32).map(|i| (i % 7) as u8).collect();
        let packed = compress(&body);
        assert!(packed.len() < body.len() / 4);
        assert_eq!(decompress(&packed, body.len(), usize::MAX).unwrap(), body);
        assert!(decompress(&packed, body.len() + 1, usize::MAX).is_err());
        assert!(decompress(&packed, body.len(), body.len() - 1).is_err());
    }

    #[test]
    fn no_body_decompresses_past_the_bound() {
        // Zeros are the best case for LZ4.
        for n in [0, 1, 100, 65_536, 4 << 20] {
            let body = vec![0u8; n];
            let packed = compress(&body);
            assert!(
                n <= max_decompressed(packed.len()),
                "{n} from {}",
                packed.len()
            );
            assert_eq!(decompress(&packed, n, n).unwrap(), body);
        }
        // A 433-byte chunk can't ask for 1 GB.
        assert!(matches!(
            decompress(&[0; 433], 1 << 30, usize::MAX),
            Err(ChunkError::Invalid(_))
        ));
    }
}
