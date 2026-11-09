//! The start cache: the console at a clip's start, kept so a clip deep in
//! a play doesn't replay the input from power-on on every score.

/// Starts kept under a chain hash: the ROM, the power-on settings and the
/// emulator's identity, then each block of [`CHECKPOINT`] frames in turn,
/// so two logs share a start exactly as far as they share frames. Files
/// live under `$GP_HOME/work/snapshots`, and the oldest-used are removed
/// past a size cap ([`cap_bytes`]).
pub(crate) mod cache {
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::sync::atomic::{AtomicUsize, Ordering};

    use anyhow::Result;
    use gp_evidence::input_log::InputFrame;
    use gp_evidence::manifest::PowerOn;
    use sha2::{Digest as _, Sha256};

    /// Frames between checkpoints.
    pub(crate) const CHECKPOINT: usize = 600;

    /// The cache's size cap: `$GP_SNAPSHOT_CAP_MB` megabytes, else 4 GB.
    fn cap_bytes() -> u64 {
        std::env::var("GP_SNAPSHOT_CAP_MB")
            .ok()
            .and_then(|v| v.parse::<u64>().ok())
            .unwrap_or(4096)
            * 1024
            * 1024
    }

    fn dir(home: &Path) -> PathBuf {
        home.join("work").join("snapshots").join("cp")
    }

    /// The chain hash's first link: what makes two runs of the same frames
    /// come out the same.
    fn root(rom: &[u8], power_on: &PowerOn) -> [u8; 32] {
        let mut h = Sha256::new();
        h.update(b"gp-snapshot-cache 1\n");
        h.update(Sha256::digest(rom));
        h.update(serde_json::to_vec(power_on).unwrap_or_default());
        h.update(serde_json::to_vec(&gp_nes::runtime_ref()).unwrap_or_default());
        h.finalize().into()
    }

    fn link(prev: &[u8; 32], frames: &[InputFrame]) -> [u8; 32] {
        let mut h = Sha256::new();
        h.update(prev);
        for f in frames {
            h.update(f.pads);
            h.update([u8::from(f.reset)]);
        }
        h.finalize().into()
    }

    /// The keys of the checkpoints at frames CHECKPOINT, 2*CHECKPOINT, ...
    /// up to `n`.
    fn keys(rom: &[u8], power_on: &PowerOn, frames: &[InputFrame], n: usize) -> Vec<[u8; 32]> {
        let mut k = root(rom, power_on);
        let mut out = Vec::new();
        let mut at = 0;
        while at + CHECKPOINT <= n {
            k = link(&k, &frames[at..at + CHECKPOINT]);
            out.push(k);
            at += CHECKPOINT;
        }
        out
    }

    static WRITES: AtomicUsize = AtomicUsize::new(0);

    /// Removes the least recently used checkpoints and starts until the
    /// cache is under `cap` bytes.
    pub(crate) fn evict(home: &Path, cap: u64) {
        let mut files: Vec<(std::time::SystemTime, u64, PathBuf)> = Vec::new();
        let tops = [dir(home), starts_dir(home)];
        for sub in tops
            .iter()
            .filter_map(|d| fs::read_dir(d).ok())
            .flat_map(|t| t.flatten())
        {
            let Ok(rd) = fs::read_dir(sub.path()) else {
                continue;
            };
            for e in rd.flatten() {
                if let Ok(m) = e.metadata()
                    && m.is_file()
                {
                    files.push((
                        m.modified().unwrap_or(std::time::UNIX_EPOCH),
                        m.len(),
                        e.path(),
                    ));
                }
            }
        }
        let mut total: u64 = files.iter().map(|f| f.1).sum();
        if total <= cap {
            return;
        }
        files.sort();
        for (_, len, p) in files {
            if total <= cap {
                break;
            }
            if fs::remove_file(&p).is_ok() {
                total -= len;
            }
        }
    }

    /// Frames a clip's start snapshot stands before the clip's first frame:
    /// a lockstep's screen measure starts drawing the original 3 frames
    /// before its start, and a resumed run must step those frames itself.
    pub(crate) const START_LEAD: usize = 4;

    /// Whether the start cache is on: `$GP_START_CACHE=off` turns it off,
    /// and a clip replays its start from power-on (to check that both give
    /// the same output).
    pub(crate) fn starts_on() -> bool {
        std::env::var("GP_START_CACHE").map_or(true, |v| v != "off" && v != "0")
    }

    fn starts_dir(home: &Path) -> PathBuf {
        home.join("work").join("snapshots").join("starts")
    }

    fn start_file(home: &Path, key: &[u8; 32], ext: &str) -> PathBuf {
        let h: String = key.iter().map(|b| format!("{b:02x}")).collect();
        starts_dir(home).join(&h[..2]).join(format!("{h}.{ext}"))
    }

    /// The key of the state after the first `n` frames of `frames` from
    /// power-on with `power_on`: the checkpoints' chain to the last one at
    /// or before `n` ([`keys`]), then the frames after it and `n`. Two
    /// clips whose input agrees up to `n` share it, whatever follows.
    pub(crate) fn start_key(
        rom: &[u8],
        power_on: &PowerOn,
        frames: &[InputFrame],
        n: usize,
    ) -> [u8; 32] {
        let n = n.min(frames.len());
        let base = keys(rom, power_on, frames, n)
            .last()
            .copied()
            .unwrap_or_else(|| root(rom, power_on));
        let at = n / CHECKPOINT * CHECKPOINT;
        let mut h = Sha256::new();
        h.update(b"gp-start 1\n");
        h.update(base);
        h.update((n as u64).to_le_bytes());
        h.update(link(&base, &frames[at..n]));
        h.finalize().into()
    }

    /// A console's state as a start keeps it: the snapshot, its frame and
    /// the buttons held.
    #[derive(Clone)]
    pub(crate) struct Saved {
        pub snap: Vec<u8>,
        pub frame: u64,
        pub held: [u8; 4],
    }

    impl Saved {
        pub(crate) fn of(nes: &gp_nes::Nes) -> Result<Self> {
            Ok(Self {
                snap: nes.snapshot()?,
                frame: nes.frame(),
                held: nes.held(),
            })
        }

        /// A console powered on and restored to it, video off as after
        /// power-on.
        pub(crate) fn nes(&self, rom: &[u8], power_on: PowerOn) -> Result<gp_nes::Nes> {
            let mut nes = gp_nes::Nes::power_on(rom, power_on)?;
            nes.restore(&self.snap, self.frame, self.held)?;
            nes.set_video(false);
            Ok(nes)
        }
    }

    const START_MAGIC: &[u8; 8] = b"gpstart1";

    /// The start under `key`, restored on `rom` and checked: the canonical
    /// state's hash must be the one written with it, else the file is
    /// removed and `None` given.
    pub(crate) fn read_start(
        home: &Path,
        key: &[u8; 32],
        rom: &[u8],
        power_on: PowerOn,
    ) -> Option<Saved> {
        let f = start_file(home, key, "start");
        let b = fs::read(&f).ok()?;
        let bad = || {
            let _ = fs::remove_file(&f);
            None
        };
        if b.len() < 8 + 8 + 4 + 64 || &b[..8] != START_MAGIC {
            return bad();
        }
        let frame = u64::from_le_bytes(b[8..16].try_into().ok()?);
        let held: [u8; 4] = b[16..20].try_into().ok()?;
        let want = String::from_utf8(b[20..84].to_vec()).ok()?;
        let saved = Saved {
            snap: b[84..].to_vec(),
            frame,
            held,
        };
        match saved
            .nes(rom, power_on)
            .and_then(|n| Ok(n.state_hash()?.to_hex()))
        {
            Ok(h) if h == want => {
                if let Ok(h) = fs::File::options().append(true).open(&f) {
                    let _ = h.set_modified(std::time::SystemTime::now());
                }
                Some(saved)
            }
            _ => bad(),
        }
    }

    /// Keeps `nes`'s state as the start under `key`, with its state hash.
    pub(crate) fn write_start(home: &Path, key: &[u8; 32], nes: &gp_nes::Nes) -> Result<Saved> {
        let saved = Saved::of(nes)?;
        let f = start_file(home, key, "start");
        let mut b = START_MAGIC.to_vec();
        b.extend_from_slice(&saved.frame.to_le_bytes());
        b.extend_from_slice(&saved.held);
        b.extend_from_slice(nes.state_hash()?.to_hex().as_bytes());
        b.extend_from_slice(&saved.snap);
        put_file(home, &f, &b);
        Ok(saved)
    }

    fn put_file(home: &Path, f: &Path, b: &[u8]) {
        let Some(parent) = f.parent() else { return };
        if fs::create_dir_all(parent).is_err() {
            return;
        }
        let tmp = f.with_extension(format!("tmp{}", std::process::id()));
        if fs::write(&tmp, b).is_ok() {
            let _ = fs::rename(&tmp, f);
        }
        if WRITES.fetch_add(1, Ordering::Relaxed).is_multiple_of(64) {
            evict(home, cap_bytes());
        }
    }

    /// What a tool keeps beside a start (a lockstep's counts there, say),
    /// under a tag naming what it depends on besides the input.
    fn note_file(home: &Path, key: &[u8; 32], tag: &str) -> PathBuf {
        let t = Sha256::digest(tag.as_bytes());
        let t: String = t[..8].iter().map(|b| format!("{b:02x}")).collect();
        start_file(home, key, &format!("{t}.json"))
    }

    pub(crate) fn read_note(home: &Path, key: &[u8; 32], tag: &str) -> Option<Vec<u8>> {
        fs::read(note_file(home, key, tag)).ok()
    }

    pub(crate) fn write_note(home: &Path, key: &[u8; 32], tag: &str, b: &[u8]) {
        put_file(home, &note_file(home, key, tag), b);
    }
}

#[cfg(test)]
mod cache_tests {
    use std::fs;

    use super::cache;
    use gp_evidence::input_log::InputFrame;

    /// A 16KB NROM whose reset counts a RAM byte up forever, so its state
    /// moves every frame.
    fn counting_rom() -> Vec<u8> {
        let mut rom = b"NES\x1a\x01\x01\x00\x00".to_vec();
        rom.resize(16, 0);
        let mut prg = vec![0u8; 0x4000];
        // C000: INC $10; C002: LDA $4016; C005: JMP $C000
        prg[..8].copy_from_slice(&[0xE6, 0x10, 0xAD, 0x16, 0x40, 0x4C, 0x00, 0xC0]);
        prg[0x10] = 0x40;
        prg[0x3FFA..].copy_from_slice(&[0x10, 0xC0, 0x00, 0xC0, 0x10, 0xC0]);
        rom.extend(prg);
        rom.extend(vec![0u8; 0x2000]);
        rom
    }

    fn frames(n: usize, pad: u8) -> Vec<InputFrame> {
        (0..n)
            .map(|i| InputFrame {
                pads: [if i % 7 == 0 { pad } else { 0 }, 0, 0, 0],
                ..InputFrame::default()
            })
            .collect()
    }

    /// The start key is the input up to the start and nothing after it:
    /// clips with the same start presses share it, a different frame
    /// before the start or another start frame doesn't, on either side of
    /// a checkpoint.
    #[test]
    fn a_start_key_is_its_input_to_the_start() {
        let rom = counting_rom();
        let p = gp_evidence::manifest::PowerOn::default();
        let a = frames(1500, 1);
        let mut b = a.clone();
        for f in &mut b[900..] {
            f.pads[0] ^= 0x80;
        }
        for n in [10, 599, 600, 601, 899, 900] {
            assert_eq!(
                cache::start_key(&rom, &p, &a, n),
                cache::start_key(&rom, &p, &b, n),
                "{n}"
            );
        }
        assert_ne!(
            cache::start_key(&rom, &p, &a, 901),
            cache::start_key(&rom, &p, &b, 901)
        );
        assert_ne!(
            cache::start_key(&rom, &p, &a, 700),
            cache::start_key(&rom, &p, &a, 701)
        );
        let mut c = a.clone();
        c[3].pads[0] ^= 2;
        assert_ne!(
            cache::start_key(&rom, &p, &a, 700),
            cache::start_key(&rom, &p, &c, 700)
        );
        let mut other = rom.clone();
        other[20] ^= 1;
        assert_ne!(
            cache::start_key(&rom, &p, &a, 700),
            cache::start_key(&other, &p, &a, 700)
        );
    }

    /// A kept start is restored as the console it was, and a start whose
    /// state hash doesn't match is refused and removed.
    #[test]
    fn a_kept_start_is_the_console_it_was() {
        let rom = counting_rom();
        let p = gp_evidence::manifest::PowerOn::default();
        let home = tempfile::tempdir().unwrap();
        let log = frames(800, 1);
        for n in [5, 640] {
            let mut plain = gp_nes::Nes::power_on(&rom, p).unwrap();
            for f in &log[..n] {
                plain.step(*f).unwrap();
            }
            let key = cache::start_key(&rom, &p, &log, n);
            cache::write_start(home.path(), &key, &plain).unwrap();
            let saved = cache::read_start(home.path(), &key, &rom, p).unwrap();
            let mut c = saved.nes(&rom, p).unwrap();
            let mut q = plain.clone();
            assert_eq!(c.state_hash().unwrap(), q.state_hash().unwrap());
            assert_eq!((c.frame(), c.held()), (q.frame(), q.held()));
            for f in &log[n..n + 40] {
                c.step(*f).unwrap();
                q.step(*f).unwrap();
                assert_eq!(c.state_hash().unwrap(), q.state_hash().unwrap());
            }
        }
        // A kept start with a wrong hash.
        let key = cache::start_key(&rom, &p, &log, 5);
        let h: String = key.iter().map(|b| format!("{b:02x}")).collect();
        let f = home
            .path()
            .join("work/snapshots/starts")
            .join(&h[..2])
            .join(format!("{h}.start"));
        let mut b = fs::read(&f).unwrap();
        b[20] = if b[20] == b'0' { b'1' } else { b'0' };
        fs::write(&f, &b).unwrap();
        assert!(cache::read_start(home.path(), &key, &rom, p).is_none());
        assert!(!f.exists());
        let plain = gp_nes::Nes::power_on(&rom, p).unwrap();
        cache::write_start(home.path(), &key, &plain).unwrap();
        cache::evict(home.path(), 0);
        assert!(!f.exists());
    }
}
