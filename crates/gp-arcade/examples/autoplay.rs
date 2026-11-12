//! A Tapper player for making long input logs: it reads gp-arcade's RAM
//! each frame, decides the controls, and backs up to an earlier frame and
//! tries again when a life is lost. It only writes input logs (the MAME
//! oracle's `gp-arcade-input 0`) and a list of the phases it saw; the logs
//! are what count, and MAME replays them.
//!
//!   cargo run --release -p gp-arcade --example autoplay -- \
//!       --rom ~/Documents/roms/arcade/tapper.zip --out DIR/NAME \
//!       [--levels N] [--idle]
//!
//! It writes NAME.input and NAME.phases. `--from LOG` goes on from an
//! earlier play's log, its presses kept as they were. `--levels N` stops after the
//! N-th level's end sequence; `--idle` coins up, starts and then presses
//! nothing until the game is over (a game-over play).
//!
//! What it reads, all in the main CPU's work RAM (found by experiment on
//! the `tapper` set, which shares its code with tapperg):
//!
//! | Address | What |
//! |---|---|
//! | `E262 + 0x20*bar + 4*i` | Bar `bar`'s objects (0 bottom to 3 top), 8 of them, each as a sprite: y, flags, code, x. y 0 is an empty slot |
//! | `E2E6 + 4*i` | The bartender's sprites and what flies from the taps, the same way; 0x18 entries to E345 |
//! | `E4A7` | Spare lives |
//! | `E4A8` | Levels cleared |
//! | `E3B8` | The bar shown: 0 the saloon, 1 the sports bar, ... |
//! | `E3BD` | 1 in attract, 0 in a game, 2 at game over |
//! | `E3F5` | The score, six decimal digits, one a byte |
//!
//! A bar's line is y = 0x39 + 0x30*bar: customers stand on it, a customer
//! drinking is at y - 4, a full mug sliding out is at y - 5, an empty mug
//! sliding back (code 0x17) or a tip lying still is at y - 3, and the
//! bartender at y - 9. Each bar's tap is a sprite at E346 + 4*bar, its
//! flags' bit 4 set when it is on the left.

use std::fmt::Write as _;
use std::path::PathBuf;

use gp_arcade::{Controls, Romset, Tapper};

const BAR_Y: [u8; 4] = [0x39, 0x69, 0x99, 0xc9];

fn peek(t: &Tapper, a: u16) -> u8 {
    t.peek(a)
}

#[derive(Debug, Clone, Copy)]
struct Obj {
    y: u8,
    code: u8,
    x: u8,
}

/// Every object slot in use, with its slot address.
fn objects(t: &Tapper) -> Vec<(u16, Obj)> {
    let mut out = Vec::new();
    let mut a = 0xe262u16;
    while a < 0xe346 {
        let y = peek(t, a);
        if y != 0 {
            out.push((
                a,
                Obj {
                    y,
                    code: peek(t, a + 2),
                    x: peek(t, a + 3),
                },
            ));
        }
        a += 4;
    }
    out
}

/// What the player sees.
#[derive(Debug, Default, Clone)]
struct Seen {
    /// The bartender's bar and x, when he's on screen.
    bart: Option<(usize, u8)>,
    customers: [Vec<u8>; 4],
    drinking: [Vec<u8>; 4],
    mugs: [Vec<(u8, u8)>; 4],
    other: Vec<(u16, Obj)>,
}

fn see(t: &Tapper) -> Seen {
    let mut s = Seen::default();
    let objs = objects(t);
    for &(a, o) in &objs {
        let bar = BAR_Y
            .iter()
            .position(|&b| o.y == b || o.y + 4 == b || o.y + 5 == b || o.y + 9 == b);
        match bar {
            Some(k) if o.y == BAR_Y[k] && a < 0xe2e2 => s.customers[k].push(o.x),
            Some(k) if o.y + 4 == BAR_Y[k] && a < 0xe2e2 => s.drinking[k].push(o.x),
            Some(k) if o.y + 5 == BAR_Y[k] => s.mugs[k].push((o.code, o.x)),
            Some(k) if o.y + 9 == BAR_Y[k] && a == 0xe2e6 => s.bart = Some((k, o.x)),
            _ => s.other.push((a, o)),
        }
    }
    s
}

/// Whether bar `k`'s tap is on the left (the tap's sprite at E346 +
/// 4*k is flipped, bit 4 of its flags), and where the bartender stands at
/// it: 16 pixels right of a tap on the left, 22 left of one on the right.
/// The saloon's taps are all on the right, the sports bar's all on the
/// left, and the punk bar has both.
fn tap(t: &Tapper, k: usize) -> (bool, i32) {
    let a = 0xe346 + 4 * k as u16;
    let left = peek(t, a + 1) & 0x10 != 0;
    let x = i32::from(peek(t, a + 3));
    (left, if left { x + 16 } else { x - 22 })
}

struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }
    fn below(&mut self, n: u64) -> u64 {
        self.next() % n
    }
}

/// The player's own state between frames: what it is doing now.
#[derive(Clone, Default)]
struct Mind {
    /// Frames the tap has been held, and how long to hold it.
    pour: Option<(u32, u32)>,
    /// Frames to keep the controls released after a pour or a move.
    rest: u32,
    /// A move under way: the bar it left and frames waited.
    moving: Option<(usize, u32)>,
    /// Last frame's mug x by slot, to tell full mugs (going left) from
    /// empty ones (coming back right).
    mug_x: Vec<(u16, u8)>,
    /// Last frame's customer x by slot, to leave out customers being
    /// pushed back to the door (they want nothing more).
    cust_x: Vec<(u16, u8)>,
    /// Decisions to wait before trying a pour again that was found to be
    /// wasted.
    hold_off: u32,
    /// A bar chosen ahead by trying each ([`decide`]), kept until the
    /// bartender is there.
    force: Option<usize>,
}

/// Whether pouring now (the tap held `want` frames) and then doing nothing
/// for 70 frames loses a life that doing nothing at all doesn't.
fn wasted(t: &Tapper, want: u32) -> bool {
    let none = Controls::default();
    let lost = |pour: bool| {
        let mut u = t.clone();
        let lives = peek(&u, 0xe4a7);
        for i in 0..(want + 72) {
            let c = if pour && i < want {
                Controls { tap: true, ..none }
            } else {
                none
            };
            u.step(c);
            if peek(&u, 0xe4a7) != lives || peek(&u, 0xe3bd) == 2 {
                return true;
            }
        }
        false
    };
    lost(true) && !lost(false)
}

/// One decision.
fn decide(t: &Tapper, m: &mut Mind, rng: &mut Rng, wild: u64, deep: bool) -> Controls {
    let none = Controls::default();
    let s = see(t);
    let taps: Vec<(bool, i32)> = (0..4).map(|k| tap(t, k)).collect();
    // Full mugs (on the bar line less 5, going away from the tap) and
    // empty ones (code 0x17 on the bar line less 3, coming back to it; a
    // tip lies still on the same line).
    let mut empties: [Vec<(i32, i32)>; 4] = Default::default();
    let mut fulls = [0usize; 4];
    let mut now_x = Vec::new();
    let mut a = 0xe262u16;
    while a < 0xe346 {
        let y = peek(t, a);
        let x = peek(t, a + 3);
        if let Some(k) = BAR_Y.iter().position(|&b| y != 0 && y + 5 == b) {
            fulls[k] += 1;
        } else if let Some(k) = BAR_Y
            .iter()
            .position(|&b| y != 0 && y + 3 == b && peek(t, a + 2) == 0x17)
        {
            now_x.push((a, x));
            let before = m.mug_x.iter().find(|(s, _)| *s == a).map(|p| p.1);
            if let Some(b) = before {
                let step = i32::from(x) - i32::from(b);
                if step != 0 && (step < 0) == taps[k].0 {
                    empties[k].push((i32::from(x), step.abs()));
                }
            }
        }
        a += 4;
    }
    m.mug_x = now_x;
    // Customers walking to the tap or standing; not those sliding back.
    let mut wanting: [Vec<i32>; 4] = Default::default();
    let mut cust_now = Vec::new();
    for k in 0..4 {
        for i in 0..8u16 {
            let a = 0xe262 + 0x20 * k as u16 + 4 * i;
            if peek(t, a) != BAR_Y[k] {
                continue;
            }
            let x = peek(t, a + 3);
            cust_now.push((a, x));
            let before = m.cust_x.iter().find(|(s, _)| *s == a).map(|p| p.1);
            let away = before.is_some_and(|b| if taps[k].0 { x > b } else { x < b });
            if !away {
                wanting[k].push(i32::from(x));
            }
        }
    }
    m.cust_x = cust_now;
    let Some((bar, _)) = s.bart else {
        m.pour = None;
        m.moving = None;
        return none;
    };
    if let Some((held, want)) = m.pour {
        if held < want {
            m.pour = Some((held + 1, want));
            return Controls { tap: true, ..none };
        }
        m.pour = None;
        m.rest = 2;
    }
    if m.rest > 0 {
        m.rest -= 1;
        return none;
    }
    if let Some((from, waited)) = m.moving {
        if bar == from && waited < 12 {
            m.moving = Some((from, waited + 1));
            return none;
        }
        m.moving = None;
    }
    // Each bar's trouble: frames until a customer reaches the end or an
    // empty mug falls off it.
    let mut trouble: Vec<(i32, usize, bool)> = Vec::new();
    let mut far: [Option<i32>; 4] = [None; 4];
    for k in 0..4 {
        let (left, end) = taps[k];
        let dist = |x: i32| if left { x - end } else { end - x };
        let mut cs: Vec<i32> = wanting[k].iter().map(|&x| dist(x)).collect();
        cs.sort_unstable();
        // Full mugs on the way serve the nearest customers first.
        let uncovered: Vec<i32> = cs.into_iter().skip(fulls[k]).collect();
        if let Some(&d) = uncovered.first() {
            // A customer takes the bartender about 10 pixels short of the
            // tap, and walks up to a pixel a frame.
            trouble.push(((d - 10).max(0) * 3 / 2, k, false));
        }
        // A customer near the door goes out of it when a mug hits them,
        // and the level ends when every customer has gone out.
        if let Some(&d) = uncovered.last() {
            far[k] = Some(d);
        }
        for &(x, speed) in &empties[k] {
            trouble.push((dist(x) / speed, k, true));
        }
    }
    if trouble.is_empty() {
        return none;
    }
    trouble.sort();
    // Nothing urgent: serve the customer nearest a door, on this bar if
    // there is one.
    if trouble[0].0 > 60 {
        let here = far[bar].map(|d| d + 40);
        let best = (0..4)
            .filter_map(|k| far[k].map(|d| (if k == bar { here.unwrap_or(d) } else { d }, k)))
            .max();
        if let Some((_, k)) = best
            && let Some(i) = trouble.iter().position(|p| p.1 == k && !p.2)
        {
            let p = trouble.remove(i);
            trouble.insert(0, p);
        }
    }
    // Now and then, when retrying, take the second most urgent.
    let mut pick = if trouble.len() > 1 && wild > 0 && rng.below(4) < wild.min(3) {
        trouble[1]
    } else {
        trouble[0]
    };
    if let Some(k) = m.force {
        if let Some(p) = trouble.iter().find(|p| p.1 == k) {
            pick = *p;
        }
        if k == bar {
            m.force = None;
        }
    } else if deep && trouble.len() > 1 && trouble[1].0 < 150 {
        // Two bars need the bartender soon: play each choice ahead with
        // this same policy and keep the one that loses no life longest.
        let mut bars: Vec<usize> = Vec::new();
        for p in &trouble {
            if !bars.contains(&p.1) && bars.len() < 3 {
                bars.push(p.1);
            }
        }
        let mut best: Option<(u32, usize)> = None;
        for &k in &bars {
            let mut u = t.clone();
            let mut mm = m.clone();
            mm.force = Some(k);
            let mut r = Rng(rng.0);
            let lives = peek(&u, 0xe4a7);
            let mut lasted = 0;
            while lasted < 240 {
                let c = decide(&u, &mut mm, &mut r, 0, false);
                u.step(c);
                if peek(&u, 0xe4a7) != lives || peek(&u, 0xe3bd) == 2 {
                    break;
                }
                lasted += 1;
            }
            if best.is_none_or(|(b, _)| lasted > b) {
                best = Some((lasted, k));
            }
        }
        if let Some((_, k)) = best
            && let Some(p) = trouble.iter().find(|p| p.1 == k)
        {
            pick = *p;
            if k != bar {
                m.force = Some(k);
            }
        }
    }
    if deep && std::env::var_os("AUTOPLAY_TROUBLE").is_some() {
        eprintln!(
            "t {} bar {bar} trouble {trouble:?} pick {pick:?} force {:?}",
            t.frame(),
            m.force
        );
    }
    let (_, target, empty) = pick;
    if target == bar {
        if empty && wanting[bar].len() <= fulls[bar] {
            return none;
        }
        let want = 8 + if wild > 0 { rng.below(6) as u32 } else { 0 };
        if m.hold_off > 0 {
            m.hold_off -= 1;
            return none;
        }
        // A mug nobody takes falls off the far end: try the pour ahead,
        // and leave it if it costs a life that waiting doesn't.
        if deep && wasted(t, want) {
            m.hold_off = 6;
            return none;
        }
        m.pour = Some((1, want));
        return Controls { tap: true, ..none };
    }
    m.moving = Some((bar, 0));
    // The bartender wraps from the top bar to the bottom and back, a bar
    // every 7 frames, so go the short way round.
    if (target + 4 - bar) % 4 <= 2 {
        Controls { up: true, ..none }
    } else {
        Controls { down: true, ..none }
    }
}

/// A saved point to back up to.
#[derive(Clone)]
struct Point {
    t: Tapper,
    log_len: usize,
    mind: Mind,
}

/// The score: six decimal digits, one a byte, at E3F5.
fn score(t: &Tapper) -> u32 {
    (0..6).fold(0, |s, i| s * 10 + u32::from(peek(t, 0xe3f5 + i) % 10))
}

/// Levels cleared (E4A8) and the bar shown (E3B8: 0 the saloon, 1 the
/// sports bar, ...).
fn level(t: &Tapper) -> (u8, u8) {
    (peek(t, 0xe4a8), peek(t, 0xe3b8))
}

/// What is on the screen, coarsely, for the phase list.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Mode {
    /// The bartender at the bars.
    Play,
    /// The bonus round's six cans.
    Bonus,
    /// Anything else in a game: a level's start or end, a life lost.
    Other,
    /// Attract mode, before a game (E3BD 1).
    Attract,
    /// Game over and the initials (E3BD 2).
    GameOver,
}

fn mode(t: &Tapper) -> Mode {
    match peek(t, 0xe3bd) {
        1 => return Mode::Attract,
        2 => return Mode::GameOver,
        _ => {}
    }
    if (0..6).any(|i| peek(t, 0xe262 + 8 * i) == 0x89 && peek(t, 0xe264 + 8 * i) == 0x57) {
        Mode::Bonus
    } else if see(t).bart.is_some() {
        Mode::Play
    } else {
        Mode::Other
    }
}

/// The bonus round is waiting for a choice: the bartender stands behind
/// the cans (codes 5C to 5E in bar 0's slots).
fn choosing(t: &Tapper) -> bool {
    mode(t) == Mode::Bonus && (0..8).any(|i| (0x5c..=0x5e).contains(&peek(t, 0xe264 + 4 * i)))
}

/// The bonus round's choice, found by trying each: hold left or right a while,
/// then press the tap, and keep what scores most.
fn bonus_choice(t: &Tapper) -> Vec<Controls> {
    let none = Controls::default();
    let mut best: Option<(u32, Vec<Controls>)> = None;
    // Left of where the bartender stands, and the can or two right of it.
    for k in -6..24i32 {
        let way = if k < 0 {
            Controls {
                right: true,
                ..none
            }
        } else {
            Controls { left: true, ..none }
        };
        let mut plan = vec![way; k.unsigned_abs() as usize * 4];
        plan.extend([none; 2]);
        plan.extend([Controls { tap: true, ..none }; 3]);
        let mut u = t.clone();
        for &c in &plan {
            u.step(c);
        }
        for _ in 0..400 {
            u.step(none);
        }
        let s = score(&u);
        if best.as_ref().is_none_or(|(b, _)| s > *b) {
            best = Some((s, plan));
        }
    }
    best.map(|b| b.1).unwrap_or_default()
}

fn main() {
    let mut args = std::env::args().skip(1);
    let mut rom = PathBuf::new();
    let mut frames = 200_000usize;
    let mut out = PathBuf::from("play");
    let mut idle = false;
    let mut levels = u8::MAX;
    let mut trace = None;
    let mut tries = 24u32;
    let mut from: Option<PathBuf> = None;
    while let Some(a) = args.next() {
        match a.as_str() {
            "--rom" => rom = args.next().unwrap().into(),
            "--frames" => frames = args.next().unwrap().parse().unwrap(),
            "--out" => out = args.next().unwrap().into(),
            "--levels" => levels = args.next().unwrap().parse().unwrap(),
            "--tries" => tries = args.next().unwrap().parse().unwrap(),
            "--idle" => idle = true,
            "--from" => from = Some(PathBuf::from(args.next().unwrap())),
            "--trace" => {
                trace = Some((
                    PathBuf::from(args.next().unwrap()),
                    args.next().unwrap().parse::<u64>().unwrap(),
                    args.next().unwrap().parse::<u64>().unwrap(),
                ))
            }
            _ => panic!("unknown {a}"),
        }
    }
    let set = Romset::load_zip(&rom).unwrap();
    if let Some((file, from, to)) = trace {
        print_trace(&set, &file, from, to);
        return;
    }
    let mut t = Tapper::power_on(&set);
    let none = Controls::default();
    let mut log: Vec<Controls> = Vec::new();
    let push = |t: &mut Tapper, log: &mut Vec<Controls>, c: Controls, n: usize| {
        for _ in 0..n {
            t.step(c);
            log.push(c);
        }
    };
    if let Some(file) = &from {
        // Go on from an earlier play: its presses first, as they were.
        let old =
            gp_arcade::oracle::InputLog::parse(&std::fs::read_to_string(file).unwrap()).unwrap();
        for n in 1..=old.frames.len() {
            push(&mut t, &mut log, old.controls(n), 1);
        }
    } else {
        // Power-on and 30 seconds of attract, a coin, then start 1.
        push(&mut t, &mut log, none, 900);
        push(&mut t, &mut log, Controls { coin: true, ..none }, 6);
        push(&mut t, &mut log, none, 54);
        push(
            &mut t,
            &mut log,
            Controls {
                start: true,
                ..none
            },
            6,
        );
    }
    let mut mind = Mind::default();
    let mut rng = Rng(0x9e37_79b9_7f4a_7c15);
    let mut points: Vec<Point> = Vec::new();
    let mut lives = peek(&t, 0xe4a7);
    let mut fails_at: Option<(u64, u32)> = None;
    let mut notes = String::new();
    let mut plan: std::collections::VecDeque<Controls> = Default::default();
    let mut chose_at = 0u64;
    let mut cleared_at: Option<u64> = None;
    // The frame the current level's play began: backing up stops there.
    let mut level_start = t.frame();
    let mut seen_level = level(&t);
    let debug = std::env::var_os("AUTOPLAY_DEBUG").is_some();
    let mut recent: std::collections::VecDeque<String> = Default::default();
    while log.len() < frames {
        let f = t.frame();
        if f.is_multiple_of(20) && points.last().is_none_or(|p| p.t.frame() < f) {
            points.push(Point {
                t: t.clone(),
                log_len: log.len(),
                mind: mind.clone(),
            });
            if points.len() > 90 {
                points.remove(0);
            }
        }
        if !idle && plan.is_empty() && choosing(&t) && f > chose_at + 600 {
            plan.extend(bonus_choice(&t));
            chose_at = f;
            let _ = writeln!(notes, "# bonus round choice made at frame {f}");
        }
        let wild = fails_at.map_or(0, |(_, n)| u64::from(n));
        let c = if idle {
            none
        } else if let Some(c) = plan.pop_front() {
            c
        } else {
            decide(&t, &mut mind, &mut rng, wild, true)
        };
        if debug {
            let s = see(&t);
            recent.push_back(format!(
                "{} {:07b} b{:?} c{:?} d{:?} m{:x?}",
                t.frame() + 1,
                c.bits(),
                s.bart,
                s.customers,
                s.drinking,
                s.other
                    .iter()
                    .filter(|(_, o)| o.code == 0x17)
                    .map(|(a, o)| (a - 0xe000, o.y, o.x))
                    .collect::<Vec<_>>()
            ));
            if recent.len() > 150 {
                recent.pop_front();
            }
        }
        t.step(c);
        log.push(c);
        if level(&t) != seen_level && mode(&t) == Mode::Play {
            seen_level = level(&t);
            level_start = t.frame();
        }
        let now = peek(&t, 0xe4a7);
        // E3BD goes to 2 at game over (0 in a game, 1 in attract).
        let over = peek(&t, 0xe3bd) == 2;
        if over && idle {
            let _ = writeln!(notes, "# game over at frame {}", t.frame());
            // Let the game-over screens and the initials entry time out.
            push(&mut t, &mut log, none, 1800);
            break;
        }
        if (now < lives || over) && !idle {
            // A life lost: back up and try again, further back each time.
            let n = match fails_at {
                Some((at, n)) if t.frame() <= at + 300 => n + 1,
                _ => 1,
            };
            if n <= tries {
                if debug && n == 1 {
                    eprintln!("--- the frames before it:");
                    for r in &recent {
                        eprintln!("{r}");
                    }
                }
                let back = 40 * u64::from(n.min(20));
                let target = t.frame().saturating_sub(back).max(level_start);
                while points.len() > 1 && points.last().unwrap().t.frame() > target {
                    points.pop();
                }
                let p = points.last().unwrap().clone();
                eprintln!(
                    "life lost at frame {}, try {n}: back to frame {}",
                    t.frame(),
                    p.t.frame()
                );
                t = p.t;
                log.truncate(p.log_len);
                mind = p.mind;
                plan.clear();
                recent.clear();
                if chose_at > t.frame() {
                    chose_at = 0;
                }
                fails_at = Some((
                    fails_at.map_or(t.frame() + back, |(a, _)| a.max(t.frame())),
                    n,
                ));
                rng.next();
                continue;
            }
            if over {
                let _ = writeln!(
                    notes,
                    "# the last life lost at frame {}, after {tries} tries: game over",
                    t.frame()
                );
                push(&mut t, &mut log, none, 1800);
                break;
            }
            let _ = writeln!(
                notes,
                "# life lost at frame {}: kept after {tries} tries",
                t.frame()
            );
            fails_at = None;
        }
        if fails_at.is_some_and(|(at, _)| t.frame() > at + 600) {
            fails_at = None;
        }
        lives = now;
        let (cleared, _) = level(&t);
        if cleared >= levels && cleared_at.is_none() {
            cleared_at = Some(t.frame());
        }
        // Stop once the next level is under way after the last one asked
        // for, so its end sequence and whatever follows are in the log.
        if let Some(at) = cleared_at
            && mode(&t) == Mode::Play
            && t.frame() > at + 60
        {
            push(&mut t, &mut log, none, 30);
            break;
        }
        if t.frame().is_multiple_of(600) {
            // The log so far, so a long run that is stopped keeps its work.
            write_log(
                &PathBuf::from(format!("{}-partial", out.display())),
                &log,
                &notes,
            );
            eprintln!(
                "frame {} lives {} cleared {:?} score {} mode {:?}",
                t.frame(),
                now,
                level(&t),
                score(&t),
                mode(&t)
            );
        }
    }
    eprint!("{notes}");
    write_log(&out, &log, &notes);
    write_phases(&set, &out, &log);
}

/// Plays `log` and writes NAME.phases: each run of frames with the same
/// mode, levels cleared, bar and lives, with the frame numbers the MAME
/// oracle uses (frame 1 is the first stepped).
fn write_phases(set: &Romset, out: &std::path::Path, log: &[Controls]) {
    let mut t = Tapper::power_on(set);
    let mut s = String::from(
        "# frames FIRST-LAST (oracle frame numbers), mode, levels cleared, bar, spare lives, score at the end\n",
    );
    let mut run: Option<(u64, (Mode, u8, u8, u8))> = None;
    let mut last_score = 0;
    for &c in log {
        t.step(c);
        let (cleared, bar) = level(&t);
        let key = (mode(&t), cleared, bar, peek(&t, 0xe4a7));
        match run {
            Some((_, k)) if k == key => {}
            Some((from, k)) => {
                let _ = writeln!(
                    s,
                    "{from}-{} {:?} cleared {} bar {} lives {} score {last_score}",
                    t.frame() - 1,
                    k.0,
                    k.1,
                    k.2,
                    k.3 as i8
                );
                run = Some((t.frame(), key));
            }
            None => run = Some((t.frame(), key)),
        }
        last_score = score(&t);
    }
    if let Some((from, k)) = run {
        let _ = writeln!(
            s,
            "{from}-{} {:?} cleared {} bar {} lives {} score {last_score}",
            t.frame(),
            k.0,
            k.1,
            k.2,
            k.3 as i8
        );
    }
    std::fs::write(out.with_extension("phases"), s).unwrap();
}

fn print_trace(set: &Romset, file: &std::path::Path, from: u64, to: u64) {
    let log = gp_arcade::oracle::InputLog::parse(&std::fs::read_to_string(file).unwrap()).unwrap();
    let mut t = Tapper::power_on(set);
    for n in 1..=to as usize {
        let c = log.controls(n);
        if n as u64 >= from {
            let s = see(&t);
            println!(
                "{} {:07b} L{} {:?} {:?} b{:?} c{:?} d{:?} m{:x?} o{:x?}",
                n,
                c.bits(),
                peek(&t, 0xe4a7),
                level(&t),
                mode(&t),
                s.bart,
                s.customers,
                s.drinking,
                s.mugs,
                s.other
                    .iter()
                    .map(|(a, o)| (a - 0xe000, o.y, o.code, o.x))
                    .collect::<Vec<_>>()
            );
        }
        t.step(c);
    }
}

fn write_log(out: &std::path::Path, log: &[Controls], notes: &str) {
    let mut s = String::from(
        "gp-arcade-input 0\n# Made by gp-arcade's autoplay example: a coin and start 1 after\n# 30 seconds of attract, then the player's own presses.\nsystem tapper\ncolumns CSRLDUT\n",
    );
    for l in notes.lines() {
        let _ = writeln!(s, "{l}");
    }
    let mut i = 0;
    while i < log.len() {
        let mut j = i;
        while j < log.len() && log[j] == log[i] {
            j += 1;
        }
        let bits = log[i].bits();
        let row: String = "CSRLDUT"
            .chars()
            .enumerate()
            .map(|(k, ch)| if bits & (1 << k) != 0 { ch } else { '.' })
            .collect();
        let _ = writeln!(s, "{} {row}", j - i);
        i = j;
    }
    std::fs::write(out.with_extension("input"), s).unwrap();
}
