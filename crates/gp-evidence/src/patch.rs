//! Applying ROM hack patches: IPS and BPS, the two formats SMB Arena and
//! romhacking.net hand out. A patched source (docs/sources.md, "Patched
//! sources") is a base ROM the person owns plus a patch they download from
//! the author; Game Printer applies it in memory and keeps only hashes.
//!
//! IPS has no checksums, so the registry's hashes of the base and the
//! result are the only check. BPS carries CRC32s of the source, the target
//! and the patch itself, and [`apply`] checks all three.

use serde::{Deserialize, Serialize};

/// A patch file's format, found by its magic bytes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PatchFormat {
    Ips,
    Bps,
}

impl PatchFormat {
    /// The format of a patch file, by its first bytes.
    pub fn detect(patch: &[u8]) -> Option<Self> {
        if patch.starts_with(b"PATCH") {
            Some(PatchFormat::Ips)
        } else if patch.starts_with(b"BPS1") {
            Some(PatchFormat::Bps)
        } else {
            None
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            PatchFormat::Ips => "ips",
            PatchFormat::Bps => "bps",
        }
    }
}

/// What applying a patch did, beside the result's bytes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Applied {
    pub format: PatchFormat,
    /// IPS records (or BPS actions) applied.
    pub records: usize,
    /// Bytes of the result that differ from the input, where both have one.
    pub bytes_changed: usize,
    pub input_bytes: usize,
    pub output_bytes: usize,
}

#[derive(Debug, PartialEq, Eq, thiserror::Error)]
pub enum PatchError {
    #[error("not an IPS or BPS patch (it starts with neither \"PATCH\" nor \"BPS1\")")]
    UnknownFormat,
    #[error("the patch ends inside a record at byte {0}")]
    Truncated(usize),
    #[error("the IPS patch has no EOF marker")]
    NoEof,
    #[error(
        "the IPS patch has {0} bytes after its EOF marker, which is neither nothing nor a 3-byte truncation size"
    )]
    TrailingBytes(usize),
    #[error("the BPS patch expects a source of {want} bytes, but the base has {got}")]
    SourceSize { want: u64, got: usize },
    #[error("the BPS patch's {what} CRC32 is {want:08x}, but the {what} has {got:08x}")]
    Crc {
        what: &'static str,
        want: u32,
        got: u32,
    },
    #[error("the BPS patch reads outside the {what} at byte {at}")]
    OutOfRange { what: &'static str, at: usize },
    #[error("the BPS patch writes {wrote} bytes, but its target is {want}")]
    TargetSize { want: u64, wrote: usize },
    #[error("the patch would make an image of {0} bytes, more than any NES ROM")]
    TooLarge(u64),
}

/// The most bytes a result may have: more than any NES image.
const MAX_OUTPUT: u64 = 16 * 1024 * 1024;

/// Applies `patch` to `base`, which is the whole file the patch was made
/// for (for NES IPS patches, the iNES file with its header).
pub fn apply(base: &[u8], patch: &[u8]) -> Result<(Vec<u8>, Applied), PatchError> {
    let format = PatchFormat::detect(patch).ok_or(PatchError::UnknownFormat)?;
    let (out, records) = match format {
        PatchFormat::Ips => apply_ips(base, patch)?,
        PatchFormat::Bps => apply_bps(base, patch)?,
    };
    let bytes_changed = base.iter().zip(&out).filter(|(a, b)| a != b).count();
    let applied = Applied {
        format,
        records,
        bytes_changed,
        input_bytes: base.len(),
        output_bytes: out.len(),
    };
    Ok((out, applied))
}

fn apply_ips(base: &[u8], patch: &[u8]) -> Result<(Vec<u8>, usize), PatchError> {
    let mut out = base.to_vec();
    let mut at = 5;
    let mut records = 0;
    let take = |at: usize, n: usize| -> Result<&[u8], PatchError> {
        patch
            .get(at..at + n)
            .ok_or(PatchError::Truncated(at.min(patch.len())))
    };
    loop {
        if patch.len() < at + 3 {
            return Err(PatchError::NoEof);
        }
        if &patch[at..at + 3] == b"EOF" {
            at += 3;
            break;
        }
        let h = take(at, 5)?;
        let offset = u64::from(h[0]) << 16 | u64::from(h[1]) << 8 | u64::from(h[2]);
        let size = usize::from(h[3]) << 8 | usize::from(h[4]);
        at += 5;
        let data: Vec<u8> = if size == 0 {
            let r = take(at, 3)?;
            at += 3;
            vec![r[2]; usize::from(r[0]) << 8 | usize::from(r[1])]
        } else {
            let d = take(at, size)?.to_vec();
            at += size;
            d
        };
        let end = offset + data.len() as u64;
        if end > MAX_OUTPUT {
            return Err(PatchError::TooLarge(end));
        }
        let (offset, end) = (offset as usize, end as usize);
        if out.len() < end {
            out.resize(end, 0);
        }
        out[offset..end].copy_from_slice(&data);
        records += 1;
    }
    match patch.len() - at {
        0 => {}
        3 => {
            let t = &patch[at..];
            let size = usize::from(t[0]) << 16 | usize::from(t[1]) << 8 | usize::from(t[2]);
            out.truncate(size);
        }
        n => return Err(PatchError::TrailingBytes(n)),
    }
    Ok((out, records))
}

/// CRC-32 (IEEE 802.3, reflected, as zlib computes it), which BPS uses.
pub fn crc32(bytes: &[u8]) -> u32 {
    let mut crc = 0xFFFF_FFFFu32;
    for &b in bytes {
        crc ^= u32::from(b);
        for _ in 0..8 {
            crc = if crc & 1 != 0 {
                (crc >> 1) ^ 0xEDB8_8320
            } else {
                crc >> 1
            };
        }
    }
    !crc
}

fn apply_bps(base: &[u8], patch: &[u8]) -> Result<(Vec<u8>, usize), PatchError> {
    if patch.len() < 4 + 12 {
        return Err(PatchError::Truncated(patch.len()));
    }
    let body_end = patch.len() - 12;
    let crc_at = |i: usize| {
        let b = &patch[body_end + 4 * i..body_end + 4 * i + 4];
        u32::from_le_bytes([b[0], b[1], b[2], b[3]])
    };
    let (want_source, want_target, want_patch) = (crc_at(0), crc_at(1), crc_at(2));
    let got_patch = crc32(&patch[..patch.len() - 4]);
    if got_patch != want_patch {
        return Err(PatchError::Crc {
            what: "patch",
            want: want_patch,
            got: got_patch,
        });
    }
    let mut at = 4;
    let number = |at: &mut usize| -> Result<u64, PatchError> {
        let mut data = 0u64;
        let mut shift = 1u64;
        loop {
            let x = *patch
                .get(*at)
                .filter(|_| *at < body_end)
                .ok_or(PatchError::Truncated(*at))?;
            *at += 1;
            data = data
                .checked_add(u64::from(x & 0x7F).saturating_mul(shift))
                .ok_or(PatchError::TooLarge(u64::MAX))?;
            if x & 0x80 != 0 {
                break;
            }
            shift = shift.saturating_mul(128);
            data = data
                .checked_add(shift)
                .ok_or(PatchError::TooLarge(u64::MAX))?;
        }
        Ok(data)
    };
    let source_size = number(&mut at)?;
    let target_size = number(&mut at)?;
    let metadata = number(&mut at)?;
    if source_size != base.len() as u64 {
        return Err(PatchError::SourceSize {
            want: source_size,
            got: base.len(),
        });
    }
    let got_source = crc32(base);
    if got_source != want_source {
        return Err(PatchError::Crc {
            what: "source",
            want: want_source,
            got: got_source,
        });
    }
    if target_size > MAX_OUTPUT {
        return Err(PatchError::TooLarge(target_size));
    }
    at = at
        .checked_add(metadata as usize)
        .filter(|&a| a <= body_end)
        .ok_or(PatchError::Truncated(body_end))?;
    let mut out: Vec<u8> = Vec::with_capacity(target_size as usize);
    let (mut source_rel, mut target_rel) = (0i64, 0i64);
    let mut records = 0;
    while at < body_end {
        let data = number(&mut at)?;
        let command = data & 3;
        let length = (data >> 2) as usize + 1;
        if out.len() + length > target_size as usize {
            return Err(PatchError::TargetSize {
                want: target_size,
                wrote: out.len() + length,
            });
        }
        match command {
            // SourceRead: the source's bytes at the same position.
            0 => {
                let start = out.len();
                let src = base
                    .get(start..start + length)
                    .ok_or(PatchError::OutOfRange {
                        what: "source",
                        at: start,
                    })?;
                out.extend_from_slice(src);
            }
            // TargetRead: bytes from the patch.
            1 => {
                let d = patch
                    .get(at..at + length)
                    .filter(|_| at + length <= body_end)
                    .ok_or(PatchError::Truncated(at))?;
                out.extend_from_slice(d);
                at += length;
            }
            // SourceCopy and TargetCopy: from a signed relative offset.
            _ => {
                let rel = number(&mut at)?;
                let delta = (rel >> 1) as i64 * if rel & 1 != 0 { -1 } else { 1 };
                if command == 2 {
                    source_rel += delta;
                    let s = usize::try_from(source_rel).map_err(|_| PatchError::OutOfRange {
                        what: "source",
                        at: 0,
                    })?;
                    let src = base.get(s..s + length).ok_or(PatchError::OutOfRange {
                        what: "source",
                        at: s,
                    })?;
                    out.extend_from_slice(src);
                    source_rel += length as i64;
                } else {
                    target_rel += delta;
                    let from = usize::try_from(target_rel).map_err(|_| PatchError::OutOfRange {
                        what: "target",
                        at: 0,
                    })?;
                    // Byte by byte: a TargetCopy may read what it writes.
                    for t in from..from + length {
                        let b = *out.get(t).ok_or(PatchError::OutOfRange {
                            what: "target",
                            at: t,
                        })?;
                        out.push(b);
                    }
                    target_rel += length as i64;
                }
            }
        }
        records += 1;
    }
    if out.len() as u64 != target_size {
        return Err(PatchError::TargetSize {
            want: target_size,
            wrote: out.len(),
        });
    }
    let got_target = crc32(&out);
    if got_target != want_target {
        return Err(PatchError::Crc {
            what: "target",
            want: want_target,
            got: got_target,
        });
    }
    Ok((out, records))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A made-up 64-byte "ROM": no game data anywhere in these tests.
    fn base() -> Vec<u8> {
        (0..64u8).map(|i| i.wrapping_mul(7)).collect()
    }

    fn ips(records: &[(u32, &[u8])], rle: &[(u32, u16, u8)], truncate: Option<u32>) -> Vec<u8> {
        let mut p = b"PATCH".to_vec();
        for (off, data) in records {
            p.extend_from_slice(&off.to_be_bytes()[1..]);
            p.extend_from_slice(&(data.len() as u16).to_be_bytes());
            p.extend_from_slice(data);
        }
        for (off, n, v) in rle {
            p.extend_from_slice(&off.to_be_bytes()[1..]);
            p.extend_from_slice(&[0, 0]);
            p.extend_from_slice(&n.to_be_bytes());
            p.push(*v);
        }
        p.extend_from_slice(b"EOF");
        if let Some(t) = truncate {
            p.extend_from_slice(&t.to_be_bytes()[1..]);
        }
        p
    }

    #[test]
    fn ips_records_rle_and_growth() {
        let p = ips(&[(4, &[0xAA, 0xBB]), (66, &[0x11])], &[(10, 3, 0xEE)], None);
        let (out, applied) = apply(&base(), &p).unwrap();
        assert_eq!(out.len(), 67);
        assert_eq!(&out[4..6], &[0xAA, 0xBB]);
        assert_eq!(&out[10..13], &[0xEE; 3]);
        assert_eq!(&out[64..67], &[0, 0, 0x11]);
        assert_eq!(out[0..4], base()[0..4]);
        assert_eq!(applied.format, PatchFormat::Ips);
        assert_eq!(applied.records, 3);
    }

    #[test]
    fn ips_truncation() {
        let p = ips(&[(1, &[9])], &[], Some(32));
        let (out, _) = apply(&base(), &p).unwrap();
        assert_eq!(out.len(), 32);
        assert_eq!(out[1], 9);
    }

    #[test]
    fn ips_refuses_broken_patches() {
        let good = ips(&[(4, &[1, 2, 3])], &[], None);
        assert_eq!(
            apply(&base(), &good[..good.len() - 3]),
            Err(PatchError::NoEof)
        );
        assert!(matches!(
            apply(&base(), &good[..9]),
            Err(PatchError::NoEof | PatchError::Truncated(_))
        ));
        let mut trailing = good.clone();
        trailing.push(0);
        assert_eq!(apply(&base(), &trailing), Err(PatchError::TrailingBytes(1)));
        assert_eq!(apply(&base(), b"NOTAPATCH"), Err(PatchError::UnknownFormat));
    }

    fn bps_number(mut n: u64, out: &mut Vec<u8>) {
        loop {
            let x = (n & 0x7F) as u8;
            n >>= 7;
            if n == 0 {
                out.push(0x80 | x);
                break;
            }
            out.push(x);
            n -= 1;
        }
    }

    /// A BPS patch from `base` to `target`, made of the four actions.
    fn bps(base: &[u8], target: &[u8]) -> Vec<u8> {
        let mut p = b"BPS1".to_vec();
        bps_number(base.len() as u64, &mut p);
        bps_number(target.len() as u64, &mut p);
        bps_number(0, &mut p);
        // SourceRead 8, TargetRead 4, SourceCopy 8 from 40, TargetCopy the
        // first 4 again, then SourceRead the rest.
        bps_number((8 - 1) << 2, &mut p);
        bps_number(((4 - 1) << 2) | 1, &mut p);
        p.extend_from_slice(&target[8..12]);
        bps_number(((8 - 1) << 2) | 2, &mut p);
        bps_number(40 << 1, &mut p);
        bps_number(((4 - 1) << 2) | 3, &mut p);
        bps_number(0, &mut p);
        let rest = target.len() - 24;
        // SourceRead copies from the same position in the source.
        bps_number(((rest as u64) - 1) << 2, &mut p);
        p.extend_from_slice(&crc32(base).to_le_bytes());
        p.extend_from_slice(&crc32(target).to_le_bytes());
        let c = crc32(&p);
        p.extend_from_slice(&c.to_le_bytes());
        p
    }

    #[test]
    fn bps_applies_every_action_and_checks_crcs() {
        let b = base();
        let mut t = b.clone();
        t[8..12].copy_from_slice(&[1, 2, 3, 4]);
        t.copy_within(40..48, 12);
        let first4: Vec<u8> = t[0..4].to_vec();
        t[20..24].copy_from_slice(&first4);
        let p = bps(&b, &t);
        let (out, applied) = apply(&b, &p).unwrap();
        assert_eq!(out, t);
        assert_eq!(applied.format, PatchFormat::Bps);
        assert_eq!(applied.records, 5);

        let mut other = b.clone();
        other[0] ^= 1;
        assert!(matches!(
            apply(&other, &p),
            Err(PatchError::Crc { what: "source", .. })
        ));
        let mut bad = p.clone();
        bad[6] ^= 1;
        assert!(matches!(
            apply(&b, &bad),
            Err(PatchError::Crc { what: "patch", .. })
        ));
    }

    #[test]
    fn crc32_matches_the_standard_check_value() {
        assert_eq!(crc32(b"123456789"), 0xCBF4_3926);
    }
}
