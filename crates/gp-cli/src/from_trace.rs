//! `gp query` and `gp run-until` answered from a recording's frame trace,
//! without a ROM or a console, when the trace holds what they need: the
//! frames asked for are recorded, no selector reads `bus[...]` (which goes
//! through the mapper), the trace follows this runtime's field table, and
//! this runtime made the recording. A stale recording replays as before, so
//! a runtime that now reaches other states still says so.
//! Each state is checked against its frame hash before it's read. Anything
//! else returns `None`, and the command restores a checkpoint and replays
//! as before. docs/runtime-protocol.md has the measured difference.

use anyhow::Result;
use gp_nes::protocol::{holds_in_state, read_state, reads_console};
use gp_runtime::select::{Condition, Selector};
use serde_json::json;

use crate::Outcome;
use crate::open::Opened;
use crate::recordings::{Row, entry_json, entry_line};

/// Whether the recording has a trace this runtime can read states from in
/// place of replaying: one this runtime made, in its field table.
fn readable_trace(opened: &Opened) -> Result<bool> {
    if opened.manifest.trace.is_none() || !gp_nes::recording::made_by_this_runtime(&opened.manifest)
    {
        return Ok(false);
    }
    let table = gp_nes::recording::field_table(&opened.store, &opened.manifest)?;
    Ok(table == gp_nes::canonical::field_table())
}

fn parse_all(selectors: &[String]) -> Result<Vec<Selector>> {
    selectors
        .iter()
        .map(|s| Selector::parse(s).map_err(anyhow::Error::msg))
        .collect()
}

/// `gp query` from the trace, or `None` if it needs replay.
pub(crate) fn query(opened: &Opened, frame: u64, selectors: &[String]) -> Result<Option<Outcome>> {
    let parsed = parse_all(selectors)?;
    if frame > opened.manifest.frames
        || parsed.iter().any(reads_console)
        || !readable_trace(opened)?
    {
        return Ok(None);
    }
    let Some(state) = gp_nes::recording::trace_state(&opened.store, &opened.manifest, frame)?
    else {
        return Ok(None);
    };
    let mut entries = Vec::new();
    for s in &parsed {
        entries.extend(read_state(&state, frame, s).map_err(anyhow::Error::msg)?);
    }
    Ok(Some(Outcome::ok(
        json!({
            "id": opened.entry.id,
            "frame": frame,
            "read_from": "trace",
            "checkpoint": null,
            "replayed": 0,
            "stale": false,
            "matches_recording": true,
            "entries": entries.iter().map(entry_json).collect::<Vec<_>>(),
        }),
        entries
            .iter()
            .map(entry_line)
            .collect::<Vec<_>>()
            .join("\n"),
    )))
}

/// `gp query --frames` from the trace: every frame's entries, or `None` if
/// it needs replay.
pub(crate) fn query_frames(
    opened: &Opened,
    frames: std::ops::Range<u64>,
    selectors: &[String],
) -> Result<Option<Vec<Row>>> {
    let parsed = parse_all(selectors)?;
    if frames.end > opened.manifest.frames + 1
        || parsed.iter().any(reads_console)
        || !readable_trace(opened)?
    {
        return Ok(None);
    }
    let mut rows = Vec::new();
    let mut failed = None;
    gp_nes::recording::scan_trace(
        &opened.store,
        &opened.manifest,
        frames.start..=frames.end - 1,
        |frame, state| {
            let mut entries = Vec::new();
            for s in &parsed {
                match read_state(state, frame, s) {
                    Ok(e) => entries.extend(e),
                    Err(e) => {
                        failed = Some(e);
                        return false;
                    }
                }
            }
            rows.push(Row { frame, entries });
            true
        },
    )?;
    if let Some(e) = failed {
        return Err(anyhow::Error::msg(e));
    }
    // A trace short of a frame replays instead.
    Ok((rows.len() as u64 == frames.end - frames.start).then_some(rows))
}

/// `gp run-until` from the trace, or `None` if it needs replay: when the
/// run could go past the recording's last frame, or a selector reads `bus`.
pub(crate) fn run_until(
    opened: &Opened,
    condition: &str,
    from: u64,
    max_frames: Option<u32>,
    show: &[String],
) -> Result<Option<Outcome>> {
    let cond = Condition::parse(condition).map_err(anyhow::Error::msg)?;
    let show_parsed = parse_all(show)?;
    let frames = opened.manifest.frames;
    if from > frames
        || cond.selectors().into_iter().any(reads_console)
        || show_parsed.iter().any(reads_console)
        || !readable_trace(opened)?
    {
        return Ok(None);
    }
    let last = match max_frames {
        Some(max) => from.saturating_add(max.into()),
        None => frames,
    };
    if last > frames {
        return Ok(None);
    }
    let mut stopped: Option<(u64, bool, Vec<u8>)> = None;
    let mut failed = None;
    gp_nes::recording::scan_trace(
        &opened.store,
        &opened.manifest,
        from..=last,
        |frame, state| match holds_in_state(state, frame, &cond) {
            Ok(true) => {
                stopped = Some((frame, true, state.to_vec()));
                false
            }
            Ok(false) if frame == last => {
                stopped = Some((frame, false, state.to_vec()));
                false
            }
            Ok(false) => true,
            Err(e) => {
                failed = Some(e);
                false
            }
        },
    )?;
    if let Some(e) = failed {
        return Err(anyhow::Error::msg(e));
    }
    let (at, met, state) = stopped.expect("the scan reaches the last frame or stops earlier");
    let ran = at - from;
    let mut entries = Vec::new();
    for s in &show_parsed {
        entries.extend(read_state(&state, at, s).map_err(anyhow::Error::msg)?);
    }
    let mut text = vec![if met {
        format!("{condition}: holds at frame {at} ({ran} frames from frame {from})")
    } else {
        format!("{condition}: doesn't hold by frame {at} ({ran} frames from frame {from})")
    }];
    text.extend(entries.iter().map(entry_line));
    Ok(Some(Outcome {
        json: json!({
            "id": opened.entry.id,
            "condition": condition,
            "met": met,
            "frame": at,
            "from": from,
            "frames_run": ran,
            "read_from": "trace",
            "matches_recording": true,
            "entries": entries.iter().map(entry_json).collect::<Vec<_>>(),
        }),
        text: text.join("\n"),
        exit: u8::from(!met),
    }))
}
