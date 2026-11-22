//! A quest-log row parked on another row (owner, waits-wake-on-landing):
//! `gp mechanic queue GAME ROW --status "in line" --waits-on OTHER` writes
//! `waits_on` ("GAME/OTHER") and `waits_since` on the row. Every minute gp
//! serve's runner ([`wake`]) looks at each row in line that waits, and once
//! the row it waits on has landed a card since the wait began, is done, or
//! is in the archive, moves it to next with the landing named in its note
//! (or done, when its target is met on the board). A row parked in line
//! before the field, with a note "Waits on ROW" or "Waits for ROW to land"
//! naming a row of its game, waits on that row the same way.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use anyhow::{Result, bail};
use serde_json::{Value, json};

use super::archive::read_archive;
use super::session::{name_ok, read_queue, write_queue};

/// A row waited on: (game, row).
pub(crate) type Wait = (String, String);

/// Reads `--waits-on`'s words: ROW (of `game`) or GAME/ROW.
pub(crate) fn parse(game: &str, s: &str) -> Result<Wait> {
    let s = s.trim();
    let (g, r) = match s.split_once('/') {
        Some((g, r)) => (g.trim(), r.trim()),
        None => (game, s),
    };
    if !name_ok(g) || !name_ok(r) {
        bail!("--waits-on is ROW or GAME/ROW, in lower-case letters, digits and dashes: {s}");
    }
    Ok((g.to_owned(), r.to_owned()))
}

/// The game's quest log file.
fn queue_of(home: &Path, game: &str) -> PathBuf {
    home.join("mechanics").join(game).join("queue.json")
}

/// Whether `game` has a row `row`, in its quest log or its archive.
pub(crate) fn exists(home: &Path, game: &str, row: &str) -> bool {
    let has = |q: Vec<Value>| q.iter().any(|r| r["mechanic"] == json!(row));
    has(read_queue(&queue_of(home, game))) || has(read_archive(home, game))
}

/// The row a note's words say it waits on: the word after "waits on",
/// "waits for", "wait on", "wait for", "waiting on" or "waiting for" that
/// `known` takes for a row (GAME/ROW, or ROW of the row's own game).
fn in_words(note: &str, known: &dyn Fn(&str) -> Option<Wait>) -> Option<Wait> {
    let low = note.to_lowercase();
    let words: Vec<&str> = low.split_whitespace().collect();
    for w in words.windows(3) {
        let verb = matches!(w[0], "waits" | "wait" | "waiting");
        if !verb || !matches!(w[1], "on" | "for") {
            continue;
        }
        let name = w[2].trim_start_matches(OPEN).trim_end_matches(CLOSE);
        if let Some(r) = known(name) {
            return Some(r);
        }
    }
    None
}

/// What may open or close a row's name in a note.
const OPEN: [char; 4] = ['\'', '"', '(', '`'];
const CLOSE: [char; 9] = ['.', ',', ';', ':', ')', '\'', '"', '`', '!'];

/// The row that row `r` of `game` waits on, (game, row), when it is in
/// line: its `waits_on`, else its note's words naming a row `known` has.
pub(crate) fn of(game: &str, r: &Value, known: &dyn Fn(&str, &str) -> bool) -> Option<Wait> {
    if r["status"] != json!("in line") {
        return None;
    }
    let me = r["mechanic"].as_str().unwrap_or("");
    if let Some(w) = r["waits_on"].as_str() {
        return parse(game, w).ok();
    }
    let named = |w: &str| {
        let (g, row) = parse(game, w).ok()?;
        let itself = g == game && row == me;
        (!itself && known(&g, &row)).then_some((g, row))
    };
    in_words(r["note"].as_str().unwrap_or(""), &named)
}

/// What the runner knows of the cards beside the quest logs: for each
/// (game, row), the newest landed card's `{"what", "commit", "done_ms"}`,
/// and the rows with a card still open, running or unsettled.
#[derive(Default)]
pub(crate) struct Cards {
    pub landed: BTreeMap<Wait, Value>,
    pub busy: BTreeSet<Wait>,
}

/// Why the row a row waits on no longer holds it, in words, or None while
/// it still does. `since` is when the wait began (ms; 0 unknown); `name`
/// is the row as the note names it.
fn ended(home: &Path, cards: &Cards, on: &Wait, since: u64, name: &str) -> Option<String> {
    let (game, row) = (on.0.as_str(), on.1.as_str());
    let landing = |l: &Value| {
        let what = l["what"].as_str().unwrap_or("").trim();
        match l["commit"].as_str() {
            Some(c) => format!("{name} landed ({c}): {what}"),
            None => format!("{name} landed: {what}"),
        }
    };
    if let Some(l) = cards.landed.get(on)
        && l["done_ms"].as_u64().is_none_or(|d| d > since)
    {
        return Some(landing(l));
    }
    // The row waited on has a card still open or unsettled: the wait is for
    // that card, so it holds until the card is used, refused or gone, even
    // when the row closed meanwhile (another fix lifted its line past its
    // target, 2026-11-22).
    if cards.busy.contains(on) {
        return None;
    }
    let q = read_queue(&queue_of(home, game));
    if let Some(r) = q.iter().find(|r| r["mechanic"] == json!(row)) {
        if r["status"] != json!("done") {
            return None;
        }
        let note = r["note"].as_str().unwrap_or("").trim();
        let done = format!("{name} is done: {}", crate::next::cut(note, 200));
        return Some(done);
    }
    if let Some(r) = read_archive(home, game)
        .into_iter()
        .rev()
        .find(|r| r["mechanic"] == json!(row))
    {
        if r["landed"].is_object() {
            return Some(landing(&r["landed"]));
        }
        let note = r["note"].as_str().unwrap_or("").trim();
        let done = format!("{name} is done: {}", crate::next::cut(note, 200));
        return Some(done);
    }
    let gone = "is in neither the quest log nor its archive";
    Some(format!("{name} {gone}"))
}

/// Wakes every game's rows in line whose wait is over: next, with what
/// ended the wait and the note it waited with, or done when its target is
/// met on the board (`games`, the games checkout, for the board). A row
/// with a card of its own still open or unsettled is left alone. Returns
/// the rows it moved, as (game, row).
pub(crate) fn wake(home: &Path, games: Option<&Path>, cards: &Cards) -> Vec<Wait> {
    let mut moved = Vec::new();
    let Ok(dir) = std::fs::read_dir(home.join("mechanics")) else {
        return moved;
    };
    let mut names: Vec<String> = dir
        .flatten()
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .filter(|g| name_ok(g))
        .collect();
    names.sort();
    // Each game's row names, quest log and archive, read once.
    let mut rows: BTreeSet<Wait> = BTreeSet::new();
    for g in &names {
        let q = read_queue(&queue_of(home, g));
        for r in q.into_iter().chain(read_archive(home, g)) {
            if let Some(m) = r["mechanic"].as_str() {
                rows.insert((g.clone(), m.to_owned()));
            }
        }
    }
    let known = |g: &str, r: &str| rows.contains(&(g.to_owned(), r.to_owned()));
    for game in &names {
        let f = queue_of(home, game);
        let mut q = read_queue(&f);
        let board = games.and_then(|g| crate::scoreboard::board(home, g, game));
        let mut changed = false;
        for r in &mut q {
            let Some(on) = of(game, r, &known) else {
                continue;
            };
            let me = r["mechanic"].as_str().unwrap_or("").to_owned();
            if cards.busy.contains(&(game.clone(), me.clone())) {
                continue;
            }
            let since = r["waits_since"].as_u64();
            let since = since.or_else(|| r["changed_ms"].as_u64()).unwrap_or(0);
            let name = if on.0 == *game {
                on.1.clone()
            } else {
                format!("{}/{}", on.0, on.1)
            };
            let Some(why) = ended(home, cards, &on, since, &name) else {
                continue;
            };
            let was = r["note"].as_str().unwrap_or("").trim().to_owned();
            let closes = |t: &&str| crate::scoreboard::may_close(board.as_ref(), t).is_ok();
            let met = r["target"].as_str().filter(closes).map(str::to_owned);
            let over = format!("Its wait is over: {why}");
            let again = format!("{over}. The loop takes it up again. It waited: {was}");
            let (status, note) = match met {
                Some(t) => ("done", format!("Met its target: {t}. {over}.")),
                None => ("next", again),
            };
            let Some(o) = r.as_object_mut() else { continue };
            o.insert("status".into(), json!(status));
            o.insert("note".into(), json!(crate::next::cut(&note, 600)));
            o.remove("waits_on");
            o.remove("waits_since");
            o.remove("worker");
            if status == "done" {
                o.remove("clip");
            }
            changed = true;
            moved.push((game.clone(), me));
        }
        if changed {
            let _ = write_queue(&f, &q);
        }
    }
    moved
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn home() -> tempfile::TempDir {
        tempfile::tempdir().unwrap()
    }

    fn put(home: &Path, game: &str, file: &str, rows: Value) {
        let d = home.join("mechanics").join(game);
        fs::create_dir_all(&d).unwrap();
        fs::write(d.join(file), rows.to_string()).unwrap();
    }

    fn row(home: &Path, game: &str, name: &str) -> Value {
        read_queue(&home.join("mechanics").join(game).join("queue.json"))
            .into_iter()
            .find(|r| r["mechanic"] == json!(name))
            .unwrap()
    }

    #[test]
    fn waits_on_reads_row_or_game_row() {
        let pair = |g: &str, r: &str| (g.to_owned(), r.to_owned());
        assert_eq!(parse("rooftop", "plug").unwrap(), pair("rooftop", "plug"));
        assert_eq!(parse("rooftop", "hill/x-1").unwrap(), pair("hill", "x-1"));
        assert!(parse("rooftop", "Not A Row").is_err());
    }

    /// The owner's case: site-starts-with-plugin parked with "Waits for
    /// gameprinter-plugin to land" and four picture rows with "Waits for
    /// sprite-lag-fix:" stayed in line after those landed.
    #[test]
    fn a_row_parked_on_a_row_that_landed_goes_to_next_with_the_landing() {
        let h = home();
        let h = h.path();
        put(
            h,
            "rooftop",
            "queue.json",
            json!([
                {"mechanic": "gameprinter-plugin", "status": "done", "note": "Landed on your YES: the plugin."},
                {"mechanic": "site-starts-with-plugin", "status": "in line", "note": "Waits for gameprinter-plugin to land."},
                {"mechanic": "pic-1a", "status": "in line", "note": "Waits for sprite-lag-fix: the lag first.", "changed_ms": 10},
                {"mechanic": "sprite-lag-fix", "status": "in line", "target": "x sprites at least 1"},
                {"mechanic": "card-up", "status": "in line", "note": "Card up; waits for your YES."},
                {"mechanic": "fielded", "status": "in line", "note": "the plugin first", "waits_on": "rooftop/gameprinter-plugin", "waits_since": 5},
                {"mechanic": "still", "status": "in line", "note": "x", "waits_on": "rooftop/later", "waits_since": 5},
                {"mechanic": "later", "status": "next"},
            ]),
        );
        let mut cards = Cards::default();
        let lag = ("rooftop".to_owned(), "sprite-lag-fix".to_owned());
        let l = json!({"what": "sprites a frame earlier", "commit": "1bc21b4", "done_ms": 20});
        cards.landed.insert(lag, l);
        let moved = wake(h, None, &cards);
        assert_eq!(moved.len(), 3, "{moved:?}");
        let s = row(h, "rooftop", "site-starts-with-plugin");
        assert_eq!(s["status"], "next");
        let n = s["note"].as_str().unwrap();
        assert!(n.contains("gameprinter-plugin is done"), "{n}");
        assert!(n.contains("It waited: Waits for gameprinter"), "{n}");
        let p = row(h, "rooftop", "pic-1a");
        assert_eq!(p["status"], "next");
        let pn = p["note"].as_str().unwrap();
        assert!(pn.contains("sprite-lag-fix landed (1bc21b4)"), "{pn}");
        let f = row(h, "rooftop", "fielded");
        assert_eq!(f["status"], "next");
        assert!(f.get("waits_on").is_none(), "{f}");
        assert!(f.get("waits_since").is_none(), "{f}");
        // Words naming no row, and a wait not over, stay in line.
        assert_eq!(row(h, "rooftop", "card-up")["status"], "in line");
        assert_eq!(row(h, "rooftop", "still")["status"], "in line");
        // Nothing more to wake.
        assert!(wake(h, None, &cards).is_empty());
    }

    /// A landing from before the wait began doesn't end it; an archived row
    /// does, with its commit; a row with its own card up is left alone.
    #[test]
    fn a_wait_ends_on_a_landing_after_it_or_on_the_archive() {
        let h = home();
        let h = h.path();
        put(
            h,
            "rooftop",
            "queue.json",
            json!([
                {"mechanic": "a", "status": "in line", "waits_on": "rooftop/fix", "waits_since": 100},
                {"mechanic": "fix", "status": "next"},
                {"mechanic": "b", "status": "in line", "waits_on": "hill/old", "waits_since": 1},
                {"mechanic": "c", "status": "in line", "waits_on": "hill/old", "waits_since": 1},
            ]),
        );
        let old = json!([{"mechanic": "old", "status": "done",
            "landed": {"what": "barrels", "commit": "abc1234"}}]);
        put(h, "hill", "archive.json", old);
        let at = |r: &str| ("rooftop".to_owned(), r.to_owned());
        let mut cards = Cards::default();
        let early = json!({"what": "w", "done_ms": 50});
        cards.landed.insert(at("fix"), early);
        cards.busy.insert(at("c"));
        assert_eq!(wake(h, None, &cards), vec![at("b")]);
        let b = row(h, "rooftop", "b");
        let bn = b["note"].as_str().unwrap();
        assert!(bn.contains("hill/old landed (abc1234): barrels"), "{bn}");
        assert_eq!(row(h, "rooftop", "a")["status"], "in line");
        let later = json!({"what": "w2", "done_ms": 150});
        cards.landed.insert(at("fix"), later);
        assert_eq!(wake(h, None, &cards), vec![at("a")]);
    }

    /// The owner's case (2026-11-22): a row waited on a row whose card was
    /// up, and woke when that row closed by its target, which another fix
    /// had met; the card never landed. While the row waited on has a card
    /// open, the wait holds; once the card is refused or gone, a done row
    /// ends it.
    #[test]
    fn a_wait_on_a_row_with_an_open_card_holds_until_the_card_ends() {
        let h = home();
        let h = h.path();
        put(
            h,
            "rooftop",
            "queue.json",
            json!([
                {"mechanic": "dup", "status": "in line", "waits_on": "rooftop/fix", "waits_since": 100},
                {"mechanic": "fix", "status": "done", "note": "Met its target: x sprites at least 1."},
            ]),
        );
        let at = |r: &str| ("rooftop".to_owned(), r.to_owned());
        let mut cards = Cards::default();
        cards.busy.insert(at("fix"));
        assert!(wake(h, None, &cards).is_empty());
        assert_eq!(row(h, "rooftop", "dup")["status"], "in line");
        // The card was refused (settled): the wait is over.
        cards.busy.clear();
        assert_eq!(wake(h, None, &cards), vec![at("dup")]);
        let dup = row(h, "rooftop", "dup");
        let n = dup["note"].as_str().unwrap();
        assert!(n.contains("fix is done"), "{n}");
        // Its card landing ends a wait at once, busy or not.
        put(
            h,
            "rooftop",
            "queue.json",
            json!([
                {"mechanic": "dup", "status": "in line", "waits_on": "rooftop/fix", "waits_since": 100},
                {"mechanic": "fix", "status": "in line"},
            ]),
        );
        cards.busy.insert(at("fix"));
        let l = json!({"what": "w", "commit": "abc1234", "done_ms": 200});
        cards.landed.insert(at("fix"), l);
        assert_eq!(wake(h, None, &cards), vec![at("dup")]);
    }
}
