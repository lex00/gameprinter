//! Sound as model data (docs/sound.md): music and sound effects captured
//! from any NES game, held as notes and instruments, played by a
//! deterministic APU synthesizer.
//!
//! - [`log`]: the APU register log a replay writes (docs/formats/apu-log.md).
//! - [`apu`]: a model of the APU that both capture and the synthesizer run.
//! - [`capture`]: log -> notes, instruments, effects, tempo and loop.
//! - [`song`]: the general song and effect representation
//!   (docs/formats/songs.md).
//! - [`player`]: songs and effects -> register writes, a frame at a time.
//! - [`synth`]: register writes -> samples.
//! - [`fidelity`]: a song's render against its log's.
//! - [`drivers`]: decoders for known sound drivers' data, and the registry.
//! - [`banks`]: sound bank mappings (docs/formats/sound-banks.md) and the
//!   banks found on disk.
//! - [`edit`]: deterministic edits on a song set, for agents.
//!
//! All of it is presentation: the simulation never reads it.

// Tuples of plain numbers (cycle, register, value) are this crate's
// vocabulary; naming each combination would hide them.
#![allow(clippy::type_complexity)]

pub mod apu;
pub mod banks;
pub mod capture;
pub mod compare;
pub mod drivers;
pub mod edit;
pub mod fidelity;
pub mod log;
pub mod player;
pub mod song;
pub mod synth;

#[cfg(test)]
mod tests;

use apu::ChannelState;
use song::SongSet;

/// A song's agreement with the frames of the log it came from: the
/// song's notes and its states (played by [`player`]) against
/// `reference`, the log's states over the song's frames, with `mask`
/// leaving out what effects had. With `reference_audio` (the log's render
/// over the same frames) it adds the loudness correlation of the renders.
pub fn agreement(
    set: &SongSet,
    name: &str,
    reference: &[[ChannelState; 5]],
    reference_audio: Option<&[i16]>,
    mask: Option<&[Vec<bool>; 5]>,
) -> Option<fidelity::Agreement> {
    let (timeline, frames) = player::song_timeline(set, name)?;
    let song = set.song(name)?;
    let cand = log::states_of(set.region, &timeline, frames, &set.samples);
    let intended: Vec<capture::RawNote> = song
        .tracks
        .iter()
        .flat_map(|t| {
            t.placed(song.tempo)
                .into_iter()
                .map(move |p| capture::RawNote {
                    channel: t.channel,
                    start: p.start,
                    period: p.note.period(t.channel, set.region),
                    volume: vec![1; p.note.len as usize],
                    duty: Vec::new(),
                    pitch: Vec::new(),
                    sample: None,
                })
        })
        .collect();
    let mut a = fidelity::compare(set.region, reference, &cand, mask, Some(intended));
    // Loudness over fewer than 30 frames says little.
    if let Some(ra) = reference_audio.filter(|_| frames >= 30) {
        let n = frames.min(reference.len() as u64);
        let end = set.region.frame_start(n);
        let block = (synth::SAMPLE_RATE as f64 / set.region.frame_rate()).round() as usize;
        let ca = synth::render(set.region, &timeline, end, &set.samples, synth::UNITY);
        a.loudness =
            fidelity::correlation(&synth::envelope(ra, block), &synth::envelope(&ca, block));
    }
    Some(a)
}

/// Samples of frames `from..to` (from a render starting at frame 0).
pub fn frames_of(region: apu::Region, audio: &[i16], from: u64, to: u64) -> &[i16] {
    let at = |f: u64| {
        ((region.frame_start(f) as u128 * u128::from(synth::SAMPLE_RATE))
            / u128::from(region.cpu_hz())) as usize
    };
    let a = at(from).min(audio.len());
    let b = at(to).min(audio.len()).max(a);
    &audio[a..b]
}
