//! MAME's input recordings (`.inp`, made with `mame -record`), read into
//! the controls of each frame, for `gp movie import`.
//!
//! The layout, as MAME 0.276 writes it (src/emu/ioport.cpp, `inp_header`
//! and `record_frame`/`record_port`), and as checked against a recording
//! of Tapper made in the oracle's image:
//!
//! - A 64-byte header: `MAMEINP\0`, the base time (8 bytes), the format's
//!   major and minor version (3 and 0), 2 reserved bytes, the system's
//!   name (12 bytes, NUL padded) and the MAME that made it (32 bytes).
//! - Then a zlib stream holding one record per frame: the emulated time
//!   (a 4-byte second and an 8-byte attosecond count), the speed (4
//!   bytes), then for each input port in MAME's order (by tag) two 4-byte
//!   words, the port's default value and the bits pressed. The port reads
//!   as their exclusive or.
//!
//! Record N holds the ports the game reads during frame N+1, the frame an
//! input log's entry N runs ([`crate::Controls`]). Tapper's board has six
//! ports in that order: one the game never reads, then the SSIO's IP0 to
//! IP4 (coins and starts, player 1, player 2, the DIP switches, unused).
//! Only what an arcade input log can say is taken: coin 1, start 1, tilt
//! and player 1's stick and tap. Anything else pressed (player 2, coin 2,
//! service) or DIP switches off their defaults are refused, in
//! words, rather than dropped.

use std::io::Read as _;

use crate::Controls;

/// What a recording holds.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Inp {
    /// MAME's name for the system it was made on.
    pub system: String,
    /// The MAME that made it, as it names itself (`MAME 0.276 (...)`).
    pub made_by: String,
    /// Each frame's controls, frame 1 first.
    pub frames: Vec<Controls>,
}

const HEADER: usize = 64;
/// Ports in a Tapper recording, and the bytes of one frame's record.
const PORTS: usize = 6;
const RECORD: usize = 16 + PORTS * 8;

/// Reads a recording of a Tapper set. `max_frames` bounds what a crafted
/// file can make it decompress.
pub fn parse(bytes: &[u8], max_frames: usize) -> Result<Inp, String> {
    if bytes.len() < HEADER || &bytes[..8] != b"MAMEINP\0" {
        return Err("not a MAME input recording (no MAMEINP header)".into());
    }
    let (major, minor) = (bytes[16], bytes[17]);
    if major != 3 {
        return Err(format!(
            "a MAME input recording of format {major}.{minor}; this reads format 3 (MAME 0.146 on)"
        ));
    }
    let text = |b: &[u8]| {
        let end = b.iter().position(|&c| c == 0).unwrap_or(b.len());
        String::from_utf8_lossy(&b[..end]).into_owned()
    };
    let system = text(&bytes[20..32]);
    let made_by = text(&bytes[32..64]);
    if crate::romset::set(&system).is_none() {
        return Err(format!(
            "the recording was made on {system}, which isn't a set gp-arcade runs ({})",
            crate::romset::SETS
                .iter()
                .map(|s| s.name)
                .collect::<Vec<_>>()
                .join(", ")
        ));
    }
    let limit = (max_frames * RECORD) as u64 + 1;
    let mut body = Vec::new();
    flate2::read::ZlibDecoder::new(&bytes[HEADER..])
        .take(limit)
        .read_to_end(&mut body)
        .map_err(|e| format!("the recording's frames don't decompress: {e}"))?;
    if body.len() as u64 >= limit {
        return Err(format!("the recording is longer than {max_frames} frames"));
    }
    if body.len() % RECORD != 0 {
        return Err(format!(
            "{} bytes of frames isn't a whole number of {RECORD}-byte records: not a Tapper recording of MAME 0.276's layout ({PORTS} input ports)",
            body.len()
        ));
    }
    let mut frames = Vec::with_capacity(body.len() / RECORD);
    for (n, rec) in body.as_chunks::<RECORD>().0.iter().enumerate() {
        let port = |i: usize| {
            let at = 16 + i * 8;
            let word = |o: usize| u32::from_le_bytes(rec[o..o + 4].try_into().expect("4 bytes"));
            (word(at) ^ word(at + 4)) as u8
        };
        let frame = n + 1;
        let (ip0, ip1, ip2, ip3) = (port(1), port(2), port(3), port(4));
        let held = |v: u8, mask: u8| v & mask == 0;
        if ip0 | 0x25 != 0xff {
            return Err(format!(
                "frame {frame} presses something on IP0 besides coin 1, start 1 and tilt (it reads {ip0:02x}): coin 2, start 2 and service aren't in gp-arcade-input 0"
            ));
        }
        if ip1 | 0x1f != 0xff || ip2 != 0xff {
            return Err(format!(
                "frame {frame} presses player 2's controls or an unknown bit (IP1 {ip1:02x}, IP2 {ip2:02x}): gp-arcade-input 0 has player 1's only"
            ));
        }
        if ip3 != Controls::default().ports()[3] {
            return Err(format!(
                "frame {frame} has the DIP switches at {ip3:02x}, not the defaults gp-arcade runs (fb)"
            ));
        }
        frames.push(Controls {
            coin: held(ip0, 0x01),
            start: held(ip0, 0x04),
            right: held(ip1, 0x01),
            left: held(ip1, 0x02),
            down: held(ip1, 0x04),
            up: held(ip1, 0x08),
            tap: held(ip1, 0x10),
            tilt: held(ip0, 0x20),
        });
    }
    Ok(Inp {
        system,
        made_by,
        frames,
    })
}

/// A recording in MAME's layout, from each frame's controls: what MAME
/// would write for them. For tests, and to hand a scripted run to MAME's
/// `-playback`.
pub fn write(system: &str, frames: &[Controls]) -> Vec<u8> {
    use std::io::Write as _;
    let mut out = Vec::with_capacity(HEADER + 64);
    out.extend_from_slice(b"MAMEINP\0");
    out.extend_from_slice(&[0; 8]);
    out.extend_from_slice(&[3, 0, 0, 0]);
    let mut name = [0u8; 12];
    name[..system.len().min(12)].copy_from_slice(&system.as_bytes()[..system.len().min(12)]);
    out.extend_from_slice(&name);
    let mut app = [0u8; 32];
    app[..10].copy_from_slice(b"gp-arcade ");
    out.extend_from_slice(&app);
    let period = crate::attotime::ATTOSECONDS_PER_SECOND / 30;
    let mut body = Vec::with_capacity(frames.len() * RECORD);
    for (n, c) in frames.iter().enumerate() {
        let atto = n as i64 * period;
        let sec = atto / crate::attotime::ATTOSECONDS_PER_SECOND;
        body.extend_from_slice(&(sec as i32).to_le_bytes());
        body.extend_from_slice(&(atto % crate::attotime::ATTOSECONDS_PER_SECOND).to_le_bytes());
        body.extend_from_slice(&(1u32 << 20).to_le_bytes());
        let ports = c.ports();
        let defaults = Controls::default().ports();
        let mut word = |def: u8, now: u8| {
            body.extend_from_slice(&u32::from(def).to_le_bytes());
            body.extend_from_slice(&u32::from(def ^ now).to_le_bytes());
        };
        word(0xff, 0xff);
        for i in 0..5 {
            word(defaults[i], ports[i]);
        }
    }
    let mut z = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::default());
    z.write_all(&body).expect("writing to memory");
    out.extend_from_slice(&z.finish().expect("writing to memory"));
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frames() -> Vec<Controls> {
        let mut f = vec![Controls::default(); 40];
        f[5].coin = true;
        f[9].start = true;
        f[20].tap = true;
        f[21].left = true;
        f[22].up = true;
        f[23].down = true;
        f[24].right = true;
        f
    }

    #[test]
    fn a_written_recording_reads_back() {
        let bytes = write("tapper", &frames());
        let inp = parse(&bytes, 1000).unwrap();
        assert_eq!(inp.system, "tapper");
        assert_eq!(inp.frames, frames());
    }

    #[test]
    fn refusals() {
        assert!(parse(b"not an inp", 10).is_err());
        let good = write("tapper", &frames());
        let mut bad = good.clone();
        bad[16] = 2;
        assert!(parse(&bad, 10_000).unwrap_err().contains("format 2"));
        let other = write("dkong", &frames());
        assert!(parse(&other, 10_000).unwrap_err().contains("dkong"));
        assert!(parse(&good, 10).unwrap_err().contains("longer than 10"));
        // Player 2's start (IP0 bit 3) can't be said in the log.
        let mut body = Vec::new();
        flate2::read::ZlibDecoder::new(&good[HEADER..])
            .read_to_end(&mut body)
            .unwrap();
        body[16 + 8 + 4] ^= 0x08;
        let mut z = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::default());
        std::io::Write::write_all(&mut z, &body).unwrap();
        let mut p2 = good[..HEADER].to_vec();
        p2.extend(z.finish().unwrap());
        assert!(parse(&p2, 10_000).unwrap_err().contains("frame 1"));
    }

    /// With a recording MAME made of tapper on this machine (see
    /// docs/movies.md): it reads, and its first frames are idle.
    #[test]
    fn a_mame_recording_reads_when_present() {
        let Some(home) = std::env::var_os("GP_HOME")
            .map(std::path::PathBuf::from)
            .or_else(|| {
                std::env::var_os("HOME").map(|h| std::path::PathBuf::from(h).join(".gameprinter"))
            })
        else {
            return;
        };
        let path = home.join("work/oracle/port6-short-inp/short.inp");
        let Ok(bytes) = std::fs::read(&path) else {
            eprintln!("skipped: no MAME recording at {}", path.display());
            return;
        };
        let inp = parse(&bytes, 1_000_000).unwrap();
        assert_eq!(inp.system, "tapper");
        assert!(inp.made_by.starts_with("MAME 0.276"));
        assert_eq!(inp.frames[0], Controls::default());
        assert!(inp.frames.iter().any(|c| c.coin));
    }
}
