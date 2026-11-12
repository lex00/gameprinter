//! The append-only files a gate merges by their rows (docs/gate.md,
//! "Append-only files"), in either repo. Every lane adds dated rows to the
//! end of the deterministic-inference notes log, so two patches made on one
//! tip conflict there, and before 2026-11-04 the second dropped out of its
//! batch. A conflict in such a file is resolved by keeping both sides'
//! added lines, in date order, and never dropping or changing a line the
//! base has; a side that changed or dropped one is still a conflict.
//! Ported from game-printer's gate_append.rs unchanged; the notes log
//! itself comes back with the research docs.

/// The files merged by their rows, relative to the repository. docs/gate.md
/// lists the same files, and a test holds the two together.
pub(crate) const APPEND_ONLY: &[&str] = &["docs/research/deterministic-inference.md"];

/// The marker length `git merge-file` is asked for, longer than any run of
/// `|` a table row starts with.
pub(crate) const MARKER: usize = 13;

/// A table row's date, from a first cell `| YYYY-MM-DD |`.
fn date_of(line: &str) -> Option<&str> {
    let d = line.strip_prefix("| ")?.get(..10)?;
    let b = d.as_bytes();
    let digits = |r: std::ops::Range<usize>| b[r].iter().all(u8::is_ascii_digit);
    (digits(0..4) && b[4] == b'-' && digits(5..7) && b[7] == b'-' && digits(8..10)).then_some(d)
}

/// Each line's date key: its own, else the dated line's before it in the
/// run, else none (it sorts first).
fn keys<'a>(run: &[&'a str]) -> Vec<&'a str> {
    let mut last = "";
    run.iter()
        .map(|l| {
            if let Some(d) = date_of(l) {
                last = d;
            }
            last
        })
        .collect()
}

/// Two runs of added lines merged in date order, each side's own order
/// kept and the tip's (`ours`) first on a tie; a line the tip added too is
/// kept once.
fn by_date<'a>(ours: &[&'a str], theirs: &[&'a str]) -> Vec<&'a str> {
    let theirs: Vec<&str> = theirs
        .iter()
        .copied()
        .filter(|l| !ours.contains(l))
        .collect();
    let (ko, kt) = (keys(ours), keys(&theirs));
    let (mut i, mut j) = (0, 0);
    let mut out = Vec::with_capacity(ours.len() + theirs.len());
    while i < ours.len() || j < theirs.len() {
        if j == theirs.len() || (i < ours.len() && ko[i] <= kt[j]) {
            out.push(ours[i]);
            i += 1;
        } else {
            out.push(theirs[j]);
            j += 1;
        }
    }
    out
}

/// The lines `side` adds at each gap of `base` (before its first line,
/// between each two, after its last), when `side` keeps every line of
/// `base` in order; None when it changed or dropped one.
fn gaps<'a>(base: &[&str], side: &[&'a str]) -> Option<Vec<Vec<&'a str>>> {
    let mut out = vec![Vec::new(); base.len() + 1];
    let mut k = 0;
    for l in side {
        if k < base.len() && *l == base[k] {
            k += 1;
        } else {
            out[k].push(*l);
        }
    }
    (k == base.len()).then_some(out)
}

/// One conflict's lines resolved: the base's lines in place, and at each
/// gap the lines both sides added there, in date order.
fn resolve_hunk<'a>(
    base: &[&'a str],
    ours: &[&'a str],
    theirs: &[&'a str],
) -> Option<Vec<&'a str>> {
    let (o, t) = (gaps(base, ours)?, gaps(base, theirs)?);
    let mut out = Vec::new();
    for g in 0..=base.len() {
        out.extend(by_date(&o[g], &t[g]));
        if let Some(b) = base.get(g) {
            out.push(b);
        }
    }
    Some(out)
}

/// The output of `git merge-file -p --diff3 --marker-size=MARKER` with
/// each conflict resolved by [`resolve_hunk`]; None when one can't be (a
/// side changed or dropped a line the base has).
pub(crate) fn resolve(merged: &str) -> Option<String> {
    let mark = |c: char| c.to_string().repeat(MARKER);
    let (start, mid, sep, end) = (mark('<'), mark('|'), mark('='), mark('>'));
    let is = |l: &str, m: &str| l == m || l.starts_with(&format!("{m} "));
    let mut out = String::with_capacity(merged.len());
    let mut lines = merged.split_inclusive('\n');
    while let Some(l) = lines.next() {
        let bare = l.trim_end_matches(['\n', '\r']);
        if !is(bare, &start) {
            out.push_str(l);
            continue;
        }
        let (mut ours, mut base, mut theirs) = (Vec::new(), Vec::new(), Vec::new());
        let mut part = 0;
        loop {
            let l = lines.next()?;
            let bare = l.trim_end_matches(['\n', '\r']);
            if part == 0 && is(bare, &mid) {
                part = 1;
            } else if part == 1 && bare == sep {
                part = 2;
            } else if part == 2 && is(bare, &end) {
                break;
            } else {
                [&mut ours, &mut base, &mut theirs][part].push(l);
            }
        }
        for l in resolve_hunk(&base, &ours, &theirs)? {
            out.push_str(l);
            if !l.ends_with('\n') {
                out.push('\n');
            }
        }
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn conflict(base: &[&str], ours: &[&str], theirs: &[&str]) -> String {
        let m = |c: char| c.to_string().repeat(MARKER);
        let mut s = String::from("# Notes\n");
        s.push_str(&format!("{} ours\n", m('<')));
        ours.iter().for_each(|l| s.push_str(&format!("{l}\n")));
        s.push_str(&format!("{} base\n", m('|')));
        base.iter().for_each(|l| s.push_str(&format!("{l}\n")));
        s.push_str(&format!("{}\n", m('=')));
        theirs.iter().for_each(|l| s.push_str(&format!("{l}\n")));
        s.push_str(&format!("{} theirs\n", m('>')));
        s.push_str("## Sources\n");
        s
    }

    #[test]
    fn both_sides_added_rows_are_kept_in_date_order_and_a_changed_row_is_a_conflict() {
        let r = |d: &str, w: &str| format!("| {d} | lane | {w} |");
        let (a, b, c, d) = (
            r("2026-11-02", "a"),
            r("2026-11-03", "b"),
            r("2026-11-04", "c"),
            r("2026-11-04", "d"),
        );
        let base = [a.as_str()];
        // The tip added b and c, the patch d and an older row b2.
        let b2 = r("2026-11-03", "b2");
        let got = resolve(&conflict(&base, &[&a, &b, &c], &[&a, &b2, &d])).unwrap();
        assert_eq!(
            got,
            format!("# Notes\n{a}\n{b}\n{b2}\n{c}\n{d}\n## Sources\n")
        );
        // An added row both sides have is kept once.
        let got = resolve(&conflict(&base, &[&a, &c], &[&a, &c, &d])).unwrap();
        assert_eq!(got, format!("# Notes\n{a}\n{c}\n{d}\n## Sources\n"));
        // A side that changes or drops a row the base has stays a conflict.
        assert_eq!(resolve(&conflict(&base, &[&b, &c], &[&a, &d])), None);
        assert_eq!(resolve(&conflict(&base, &[&a, &c], &[&d])), None);
        // A row's continuation (no date) keeps to the row before it.
        assert_eq!(by_date(&[&c, "  more"], &[&b]), [b.as_str(), &c, "  more"]);
    }

    /// docs/gate.md's list of append-only files is this module's.
    #[test]
    fn the_docs_list_the_append_only_files() {
        let doc = std::fs::read_to_string(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../docs/gate.md"),
        )
        .unwrap();
        let section = doc
            .split("## Append-only files")
            .nth(1)
            .expect("docs/gate.md has an Append-only files section");
        let section = section.split("\n## ").next().unwrap();
        let listed: Vec<&str> = section
            .lines()
            .filter_map(|l| l.strip_prefix("| `")?.split('`').next())
            .collect();
        assert_eq!(listed, APPEND_ONLY);
    }
}
