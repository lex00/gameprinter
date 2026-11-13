//! What blind discovery looks for past x and the jump (Wave T, from
//! modelling Tapper by hand): which of two bytes that move alike is the
//! one the game copies from, a player who changes lanes instead of
//! jumping, a button held for a while (a pour) instead of pressed, and the
//! score, for progress where x doesn't go anywhere. Each runs from a
//! savestate and puts the machine back. None names a game or an address.
//!
//! | What | How it is found |
//! |---|---|
//! | Copy direction | the board's access log over a walk: a write to one byte of the value just read from the other is a copy from that one; with none, the byte written first in the frame |
//! | Lanes | a stick way x didn't use, pressed and let go four times (one way twice, the other twice): a byte that jumps once a press by one amount and holds between (the lane's y), and a small counter that changes on the same frames (the lane) |
//! | y's direction | the player's sprites move against the y byte on a lane change: y counts up the screen |
//! | Held action | a button held 12 frames against 12 frames of nothing: a byte that counts by one on 3 or more frames in a row while it is held and never without it (the timer), and a byte that changes on the frame the count ends (done; one that goes back when the button is let go, and a flag, first) |
//! | Score | one decimal digit a byte, 3 or more bytes in a row, read as a number: it rises in play (pours and sends in every lane) and never falls |

use anyhow::Result;
use gp_evidence::input_log::InputFrame;

use super::machine::{Machine, Pads};

/// A player who moves between lanes.
#[derive(Clone, Debug)]
pub(crate) struct Lanes {
    /// The lane's y: one amount a lane.
    pub y: usize,
    /// The lane's number, changing on the frame y does.
    pub lane: Option<usize>,
    /// y's step for one lane, as a press of `up` moved it.
    pub step: i64,
    /// The y byte counts up the screen (the sprites move the other way).
    pub y_up: bool,
    /// The stick ways that change lanes.
    pub up: u8,
    pub down: u8,
}

/// A button that does something after being held.
#[derive(Clone, Debug)]
pub(crate) struct Held {
    pub button: u8,
    /// The byte that counts while it is held.
    pub timer: usize,
    /// The byte that changes when the count ends.
    pub done: Option<usize>,
    /// Frames from the press to the frame `done` changes (or the count
    /// ends), counting the press's.
    pub frames: usize,
}

fn frame(pad: u8) -> InputFrame {
    InputFrame {
        polled: true,
        pads: [pad, 0, 0, 0],
        reset: false,
    }
}

/// Each frame's work RAM over `pads`, from the savestate; put back after.
fn ram_run(
    m: &mut Machine,
    at: &(Vec<u8>, u64, [u8; 4]),
    pads: &[(u8, usize)],
) -> Result<Vec<Vec<u8>>> {
    m.restore(&at.0, at.1, at.2)?;
    let mut out = vec![m.wram().to_vec()];
    for &(p, n) in pads {
        for _ in 0..n {
            m.step(frame(p))?;
            out.push(m.wram().to_vec());
        }
    }
    m.restore(&at.0, at.1, at.2)?;
    Ok(out)
}

/// Of bytes that moved alike under the pad, the one the game copies from:
/// on the board's access log over `pads`, count for each pair the writes
/// to one of the value most recently read from the other (within the 8
/// accesses before); the byte others are copied from most, and copied to
/// least, wins. With no copy either way, the one the frame writes first.
/// None on a machine with no access log (the NES), or with no answer.
pub(crate) fn copy_source(
    m: &mut Machine,
    at: &(Vec<u8>, u64, [u8; 4]),
    pads: &[(u8, usize)],
    cands: &[usize],
) -> Result<Option<usize>> {
    let Machine::Arcade(_) = m else {
        return Ok(None);
    };
    m.restore(&at.0, at.1, at.2)?;
    let base = m.ram_base();
    let Machine::Arcade(t) = m else {
        return Ok(None);
    };
    t.set_accesses(true);
    let mut copies = vec![vec![0usize; cands.len()]; cands.len()];
    let mut first = vec![0usize; cands.len()];
    for &(p, n) in pads {
        for _ in 0..n {
            t.step(gp_arcade::Controls::from_bits(u32::from(p)));
            let acc: Vec<_> = t
                .take_accesses()
                .into_iter()
                .filter(|a| {
                    a.cpu == 0
                        && matches!(
                            a.kind,
                            gp_arcade::mcr::AccessKind::Read | gp_arcade::mcr::AccessKind::Write
                        )
                })
                .collect();
            let idx = |addr: u16| cands.iter().position(|&c| c + base == usize::from(addr));
            let mut seen_first = false;
            for (i, a) in acc.iter().enumerate() {
                if a.kind != gp_arcade::mcr::AccessKind::Write {
                    continue;
                }
                let Some(to) = idx(a.addr) else { continue };
                if !seen_first {
                    first[to] += 1;
                    seen_first = true;
                }
                for b in acc[i.saturating_sub(8)..i].iter().rev() {
                    if b.kind == gp_arcade::mcr::AccessKind::Read
                        && let Some(from) = idx(b.addr)
                    {
                        if from != to && b.value == a.value {
                            copies[from][to] += 1;
                        }
                        break;
                    }
                }
            }
        }
    }
    t.set_accesses(false);
    m.restore(&at.0, at.1, at.2)?;
    let net = |k: usize| -> i64 {
        (0..cands.len())
            .map(|j| copies[k][j] as i64 - copies[j][k] as i64)
            .sum()
    };
    if std::env::var_os("GP_BLIND_DEBUG").is_some() {
        for (k, c) in cands.iter().enumerate() {
            eprintln!(
                "copy: ${:04X} copied from {} more than to, written first in {} frames",
                c + base,
                net(k),
                first[k]
            );
        }
    }
    let best = (0..cands.len()).max_by_key(|&k| (net(k), first[k]));
    Ok(best.and_then(|k| {
        let key = (net(k), first[k]);
        let tied = (0..cands.len())
            .filter(|&j| (net(j), first[j]) == key)
            .count();
        (tied == 1).then_some(cands[k])
    }))
}

/// A lane-changing player: see the module's table. `x` is the player's x.
pub(crate) fn find_lanes(
    m: &mut Machine,
    at: &(Vec<u8>, u64, [u8; 4]),
    pads: &Pads,
    x: usize,
) -> Result<Option<Lanes>> {
    let (u, d) = (pads.up, pads.down);
    if u == 0 || d == 0 {
        return Ok(None);
    }
    // One way twice, then back twice: no wrap from a middle lane of three
    // or more, and the steps come out equal and opposite.
    for (first, second) in [(d, u), (u, d)] {
        let seq = [
            (first, 10),
            (0, 30),
            (first, 10),
            (0, 30),
            (second, 10),
            (0, 30),
            (second, 10),
            (0, 30),
        ];
        let t = ram_run(m, at, &seq)?;
        // The frames each press's window covers.
        let windows: Vec<std::ops::Range<usize>> =
            (0..4).map(|k| 1 + k * 40..1 + k * 40 + 40).collect();
        let mut best: Option<(usize, i64, Vec<usize>)> = None;
        for a in 0..0x800usize {
            if a == x || m.noise(a) {
                continue;
            }
            let mut steps = Vec::new();
            let mut at_frames = Vec::new();
            let mut ok = true;
            for w in &windows {
                let big: Vec<(usize, i64)> = w
                    .clone()
                    .filter(|&i| i < t.len())
                    .map(|i| (i, i64::from(t[i][a]) - i64::from(t[i - 1][a])))
                    .filter(|(_, dv)| dv.abs() >= 8)
                    .collect();
                let small_ok = w.clone().filter(|&i| i < t.len()).all(|i| {
                    (i64::from(t[i][a]) - i64::from(t[i - 1][a])).abs() <= 2
                        || big.iter().any(|(f, _)| *f == i)
                });
                if big.len() != 1 || !small_ok {
                    ok = false;
                    break;
                }
                steps.push(big[0].1);
                at_frames.push(big[0].0);
            }
            if !ok {
                continue;
            }
            // Down twice by one amount, up twice by its opposite (or the
            // other way round), the amount alike within 4.
            let s = steps[0];
            if (steps[1] - s).abs() > 4 || (steps[2] + s).abs() > 4 || (steps[3] + s).abs() > 4 {
                continue;
            }
            if best.as_ref().is_none_or(|(_, bs, _)| s.abs() > bs.abs()) {
                best = Some((a, s, at_frames));
            }
        }
        let Some((y, s, frames)) = best else { continue };
        // The lane: a small number changing by one on those frames and on
        // no other.
        let lane = (0..0x800usize).find(|&a| {
            a != x
                && a != y
                && !m.noise(a)
                && t.iter().all(|r| r[a] < 16)
                && (1..t.len()).all(|i| {
                    let dv = i64::from(t[i][a]) - i64::from(t[i - 1][a]);
                    if frames.contains(&i) {
                        dv.abs() == 1
                    } else {
                        dv == 0
                    }
                })
        });
        // y's direction: the player's sprites (those within 24 of x) before
        // the first change and after it (the board may draw a frame late).
        let f0 = frames[0];
        m.restore(&at.0, at.1, at.2)?;
        let mut tops: Vec<Vec<[i64; 4]>> = vec![m.cells()];
        for &(p, n) in &seq[..2] {
            for _ in 0..n {
                m.step(frame(p))?;
                tops.push(m.cells());
            }
        }
        m.restore(&at.0, at.1, at.2)?;
        let top_at = |i: usize| -> Option<f64> {
            let xv = i64::from(t[i.min(t.len() - 1)][x]);
            let near: Vec<i64> = tops
                .get(i)?
                .iter()
                .filter(|c| (c[0] - xv).abs() <= 24)
                .map(|c| c[1])
                .collect();
            (!near.is_empty()).then(|| near.iter().sum::<i64>() as f64 / near.len() as f64)
        };
        let y_up = match (top_at(f0 - 1), top_at(f0 + 2)) {
            (Some(a0), Some(a1)) => (a1 < a0) == (s > 0),
            _ => false,
        };
        let up_step = if first == u { s } else { -s };
        return Ok(Some(Lanes {
            y,
            lane,
            step: up_step,
            y_up,
            up: u,
            down: d,
        }));
    }
    Ok(None)
}

/// A button that counts while held: see the module's table.
pub(crate) fn find_held(
    m: &mut Machine,
    at: &(Vec<u8>, u64, [u8; 4]),
    button: u8,
    skip: &[usize],
) -> Result<Option<Held>> {
    if button == 0 {
        return Ok(None);
    }
    // Held twice: the first may finish a count the start left half done;
    // the second starts from nothing.
    let held = ram_run(m, at, &[(button, 12), (0, 8), (button, 12), (0, 8)])?;
    let idle = ram_run(m, at, &[(0, 40)])?;
    let hold = 21..=32usize;
    let mut timer: Option<(usize, usize, usize)> = None;
    for a in 0..0x800usize {
        if skip.contains(&a) || m.noise(a) {
            continue;
        }
        if (1..idle.len()).any(|i| idle[i][a] != idle[i - 1][a]) {
            continue;
        }
        // The longest run of steps by one, all in one direction, inside
        // the second hold.
        let (mut run, mut best, mut end, mut dir) = (0usize, 0usize, 0usize, 0i64);
        for i in hold.clone() {
            let dv = i64::from(held[i][a]) - i64::from(held[i - 1][a]);
            if dv.abs() == 1 && (run == 0 || dv == dir) {
                run += 1;
                dir = dv;
                if run > best {
                    best = run;
                    end = i;
                }
            } else {
                run = 0;
            }
        }
        // A count the button runs: it starts within 2 frames of the press
        // and holds still once the button is let go.
        let started = end + 1 - best <= hold.start() + 2;
        let still_after = (33..held.len()).all(|i| held[i][a] == held[i - 1][a]);
        if best >= 3 && started && still_after && timer.is_none_or(|(_, b, _)| best > b) {
            timer = Some((a, best, end));
        }
    }
    let Some((t_at, _, end)) = timer else {
        return Ok(None);
    };
    // The done byte: changes on the frame the count ends and on no other
    // of the hold; one that goes back on the release (what was made is
    // sent) before one that doesn't.
    let dones: Vec<usize> = (0..0x800usize)
        .filter(|&a| {
            a != t_at
                && !skip.contains(&a)
                && !m.noise(a)
                && (1..idle.len()).all(|i| idle[i][a] == idle[i - 1][a])
                && hold
                    .clone()
                    .all(|i| (held[i][a] != held[i - 1][a]) == (i == end))
        })
        .collect();
    // A flag (0 and 1 only) before another byte.
    let back = |a: usize| held[34][a] == held[hold.start() - 1][a];
    let flag = |a: usize| held.iter().all(|r| r[a] <= 1);
    let done = dones
        .iter()
        .copied()
        .max_by_key(|&a| (back(a), flag(a), std::cmp::Reverse(a)));
    let end = end - (hold.start() - 1);
    Ok(Some(Held {
        button,
        timer: t_at,
        done,
        frames: end,
    }))
}

/// The score: see the module's table. `plays` are short runs of play from
/// the savestate, each on its own (one life lost in one doesn't end the
/// others); the bytes are the score's digits, most significant first.
pub(crate) fn find_score(
    m: &mut Machine,
    at: &(Vec<u8>, u64, [u8; 4]),
    plays: &[Vec<(u8, usize)>],
) -> Result<Option<Vec<usize>>> {
    let mut runs = Vec::new();
    for p in plays {
        runs.push(ram_run(m, at, p)?);
    }
    // Nothing pressed for as long: a clock or a timer rises there too.
    let frames = plays
        .iter()
        .map(|p| p.iter().map(|q| q.1).sum::<usize>())
        .max()
        .unwrap_or(0);
    let idle = ram_run(m, at, &[(0, frames)])?;
    let value = |r: &[u8], a: usize, n: usize| -> Option<u64> {
        r[a..a + n]
            .iter()
            .try_fold(0u64, |v, &d| (d <= 9).then(|| v * 10 + u64::from(d)))
    };
    for n in (3..=8).rev() {
        for a in 0..0x800 - n {
            if (a..a + n).any(|k| m.noise(k)) {
                continue;
            }
            if idle.windows(2).any(|w| w[0][a..a + n] != w[1][a..a + n]) {
                continue;
            }
            let (mut rose, mut fell, mut moves, mut len) = (false, false, 0usize, 0usize);
            for t in &runs {
                let vals: Option<Vec<u64>> = t.iter().map(|r| value(r, a, n)).collect();
                let Some(vals) = vals else {
                    fell = true;
                    break;
                };
                rose |= vals.last() > vals.first();
                fell |= vals.windows(2).any(|w| w[1] < w[0]);
                moves += vals.windows(2).filter(|w| w[1] != w[0]).count();
                len += vals.len();
            }
            // A counter of frames rises too: the score moves on few frames.
            if rose && !fell && moves * 20 <= len {
                // The digits it holds: drop leading bytes that never moved
                // past what a shorter group from the next byte gives.
                if std::env::var_os("GP_BLIND_DEBUG").is_some() {
                    eprintln!("score: ${:04X}, {n} digits", a + m.ram_base());
                }
                // Leading zeros and a last digit that never moved can't be
                // told from their neighbours by value: on the board, the
                // digits are the bytes the instructions that wrote the
                // score's change write (the points routine adds into the
                // last digit and carries up).
                if let Some(d) = score_writes(m, at, plays, a, n)? {
                    return Ok(Some(d));
                }
                return Ok(Some((a..a + n).collect()));
            }
        }
    }
    Ok(None)
}

/// The score's value in a frame's RAM.
pub(crate) fn score_of(ram: &[u8], digits: &[usize]) -> i64 {
    digits
        .iter()
        .fold(0i64, |v, &a| v * 10 + i64::from(ram[a].min(9)))
}

/// The order the frame's work runs in, from the board's access log over
/// `pads`: on each frame, whether the player's bytes are written before
/// the other movers' (bytes that hold another sprite's x, at the offset
/// the player's sprites are drawn from x, on 80% of the frames, and
/// change). The shape is the NES routine trace's (`routines` with their
/// systems and median rank), so `gp model blind` orders the systems the
/// same way. None on the NES.
pub(crate) fn system_order(
    m: &mut Machine,
    at: &(Vec<u8>, u64, [u8; 4]),
    pads: &[(u8, usize)],
    player: &[usize],
    x: usize,
) -> Result<Option<serde_json::Value>> {
    if !m.is_arcade() {
        return Ok(None);
    }
    // The frames' RAM and sprites first, without the log.
    m.restore(&at.0, at.1, at.2)?;
    let (mut rams, mut cells) = (Vec::new(), Vec::new());
    for &(p, n) in pads {
        for _ in 0..n {
            m.step(frame(p))?;
            rams.push(m.wram().to_vec());
            cells.push(m.cells());
        }
    }
    // The player's sprites' offset from x: the commonest within 24.
    let mut offs: std::collections::BTreeMap<i64, usize> = Default::default();
    for (r, c) in rams.iter().zip(&cells) {
        for b in c {
            let d = b[0] - i64::from(r[x]);
            if d.abs() <= 24 {
                *offs.entry(d).or_default() += 1;
            }
        }
    }
    let Some((&off, _)) = offs.iter().max_by_key(|(_, n)| **n) else {
        return Ok(None);
    };
    let n = rams.len();
    let movers: Vec<usize> = (0..0x800usize)
        .filter(|a| !player.contains(a) && !m.noise(*a))
        .filter(|&a| (1..n).any(|i| rams[i][a] != rams[i - 1][a]))
        .filter(|&a| {
            // This frame's value or the last (the board draws a frame
            // late), within 2 of the offset.
            let hits = (1..n)
                .filter(|&i| {
                    cells[i].iter().any(|b| {
                        (b[0] - i64::from(rams[i][x])).abs() > 24
                            && [rams[i][a], rams[i - 1][a]]
                                .iter()
                                .any(|v| (b[0] - i64::from(*v) - off).abs() <= 2)
                    })
                })
                .count();
            hits * 10 >= n * 8
        })
        .collect();
    if movers.is_empty() {
        return Ok(None);
    }
    // Then the same frames with the log on: who writes first.
    m.restore(&at.0, at.1, at.2)?;
    let base = m.ram_base();
    let Machine::Arcade(t) = m else {
        return Ok(None);
    };
    t.set_accesses(true);
    let (mut player_first, mut movers_first) = (0usize, 0usize);
    for &(p, k) in pads {
        for _ in 0..k {
            t.step(gp_arcade::Controls::from_bits(u32::from(p)));
            let acc = t.take_accesses();
            let first = |set: &[usize]| {
                acc.iter().position(|a| {
                    a.cpu == 0
                        && a.kind == gp_arcade::mcr::AccessKind::Write
                        && usize::from(a.addr) >= base
                        && set.contains(&(usize::from(a.addr) - base))
                })
            };
            // Each mover written this frame against the player: a pair
            // each, so a buffer of many bytes doesn't outvote the rest.
            let Some(pf) = first(player) else { continue };
            for &mv in &movers {
                if let Some(mf) = first(&[mv]) {
                    if pf < mf {
                        player_first += 1;
                    } else {
                        movers_first += 1;
                    }
                }
            }
        }
    }
    t.set_accesses(false);
    m.restore(&at.0, at.1, at.2)?;
    let both = player_first + movers_first;
    if both == 0 {
        return Ok(None);
    }
    let (rp, re) = if player_first * 2 >= both {
        (0, 1)
    } else {
        (1, 0)
    };
    Ok(Some(serde_json::json!({
        "from": "the board's access log: on each frame, the first write to the player's bytes against the first to each other mover's, a pair each",
        "pairs": both,
        "player_first": player_first,
        "movers": movers.iter().map(|a| format!("${:04X}", a + base)).collect::<Vec<_>>(),
        "routines": [
            {"systems": ["player"], "median_rank": rp},
            {"systems": ["enemies"], "median_rank": re},
        ],
    })))
}

/// The score's digits by the board's access log: the instructions that
/// write a byte of the group `a..a+n` while its value changes, and every
/// byte within 8 of the group they write; the longest run of those in a
/// row. None on the NES, or when nothing was written.
fn score_writes(
    m: &mut Machine,
    at: &(Vec<u8>, u64, [u8; 4]),
    plays: &[Vec<(u8, usize)>],
    a: usize,
    n: usize,
) -> Result<Option<Vec<usize>>> {
    if !m.is_arcade() {
        return Ok(None);
    }
    let base = m.ram_base();
    let lo = a.saturating_sub(8);
    let hi = (a + n + 8).min(0x800);
    let mut pcs = std::collections::BTreeSet::new();
    let mut by_pc: std::collections::BTreeMap<u16, std::collections::BTreeSet<usize>> =
        Default::default();
    for p in plays {
        m.restore(&at.0, at.1, at.2)?;
        let Machine::Arcade(t) = m else {
            return Ok(None);
        };
        t.set_accesses(true);
        for &(pad, k) in p {
            for _ in 0..k {
                let before = t.board.nvram[a..a + n].to_vec();
                t.step(gp_arcade::Controls::from_bits(u32::from(pad)));
                let acc = t.take_accesses();
                let changed = t.board.nvram[a..a + n] != before[..];
                for x in acc.iter().filter(|x| {
                    x.cpu == 0
                        && x.kind == gp_arcade::mcr::AccessKind::Write
                        && usize::from(x.addr) >= base + lo
                        && usize::from(x.addr) < base + hi
                }) {
                    let r = usize::from(x.addr) - base;
                    by_pc.entry(x.pc).or_default().insert(r);
                    if changed && (a..a + n).contains(&r) {
                        pcs.insert(x.pc);
                    }
                }
            }
        }
        t.set_accesses(false);
    }
    m.restore(&at.0, at.1, at.2)?;
    let written: std::collections::BTreeSet<usize> = pcs
        .iter()
        .filter_map(|pc| by_pc.get(pc))
        .flatten()
        .copied()
        .collect();
    // The longest run of bytes in a row.
    let mut best: Vec<usize> = Vec::new();
    let mut cur: Vec<usize> = Vec::new();
    for &r in &written {
        if cur.last().is_some_and(|&l| l + 1 != r) {
            cur.clear();
        }
        cur.push(r);
        if cur.len() > best.len() {
            best = cur.clone();
        }
    }
    Ok((best.len() >= 3).then_some(best))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_score_reads_a_digit_a_byte_most_significant_first() {
        let mut ram = vec![0u8; 16];
        ram[3..9].copy_from_slice(&[0, 0, 1, 5, 0, 0]);
        assert_eq!(score_of(&ram, &(3..9).collect::<Vec<_>>()), 1500);
        assert_eq!(score_of(&ram, &[5, 6]), 15);
    }
}
