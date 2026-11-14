//! `gp store`: the content-addressed evidence store's own commands (put,
//! get, stats, verify, reindex, unreferenced, gc), brought from
//! game-printer's main.rs and gc.rs.
//!
//! The old tool read recordings through gp-index, a SQLite index beside the
//! store. gameprinter reads the recordings log and the manifests it names
//! each time instead (recindex.rs), so `store reindex` has no file to
//! rebuild: it reads the log and says what it found, as the old command
//! did after rebuilding. Everything else gives the old output.

use std::collections::HashSet;
use std::fs;
use std::io::Write as _;
use std::path::PathBuf;
use std::time::Instant;

use anyhow::{Context as _, Result, bail};
use clap::Subcommand;
use gp_evidence::recordings::{self, Mark, MarkKind};
use gp_evidence::{Hash, Paths, Store};
use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::Outcome;
use crate::recindex::Index;
use crate::recordings::warn_index_problems as warn_bad_lines;

#[derive(Subcommand)]
pub enum StoreCmd {
    /// Add a file; prints its hash.
    Put {
        /// The file to add.
        file: PathBuf,
    },
    /// Write an object to a file, or to stdout.
    Get {
        /// The object's BLAKE3 hash, 64 hex characters.
        hash: Hash,
        /// Write to this file instead of stdout.
        #[arg(long)]
        out: Option<PathBuf>,
    },
    /// Object count and total size.
    Stats,
    /// Re-hash every object and report any that changed.
    Verify,
    /// Read the recordings log and the manifests it names, and report what
    /// they hold. gameprinter keeps no index file (it reads the log each
    /// time), so this writes nothing.
    Reindex,
    /// List objects that no recording in the recordings log uses, without
    /// deleting anything. Exits 2 if a manifest or log line can't be read,
    /// since the objects it names would then be listed wrongly.
    Unreferenced,
    /// Delete objects that no recording uses and that were unused at the
    /// previous `gc --yes` too (docs/recording.md). Without `--yes`, lists
    /// what it would delete and writes nothing. It deletes nothing and
    /// exits 2 if a manifest or log line can't be read. It also exits 2
    /// while another command writes to the store.
    Gc {
        /// Also count objects used only by retired runs as unused. Those runs
        /// get a `deleted` mark in the log when their objects go.
        #[arg(long)]
        retired: bool,
        /// Delete, and list what's unused now for the next run.
        #[arg(long)]
        yes: bool,
        /// Minutes the previous run's list must have waited before this
        /// run deletes from it, so a write by a `gp` too old to take the
        /// store's lock can finish in between.
        #[arg(long, default_value_t = 10)]
        min_age: u32,
    },
}

pub(crate) fn run(cmd: &StoreCmd, paths: &Paths) -> Result<Outcome> {
    if let StoreCmd::Gc {
        retired,
        yes,
        min_age,
    } = cmd
    {
        return gc(*retired, *yes, *min_age, paths);
    }
    let store = Store::open(&paths.store)?;
    match cmd {
        StoreCmd::Put { file } => {
            let f = fs::File::open(file).with_context(|| format!("opening {}", file.display()))?;
            let (hash, bytes) = store.put_reader(std::io::BufReader::new(f))?;
            Ok(Outcome::ok(
                json!({ "hash": hash, "bytes": bytes }),
                hash.to_string(),
            ))
        }
        StoreCmd::Get { hash, out } => {
            let data = store.get(hash)?;
            match out {
                Some(path) => {
                    fs::write(path, &data)
                        .with_context(|| format!("writing {}", path.display()))?;
                    Ok(Outcome::ok(
                        json!({ "hash": hash, "bytes": data.len(), "out": path }),
                        format!("wrote {} bytes to {}", data.len(), path.display()),
                    ))
                }
                None => {
                    std::io::stdout().write_all(&data)?;
                    Ok(Outcome::ok(
                        json!({ "hash": hash, "bytes": data.len() }),
                        String::new(),
                    ))
                }
            }
        }
        StoreCmd::Stats => {
            let s = store.stats()?;
            Ok(Outcome::ok(
                json!(s),
                format!("{} objects, {} bytes", s.objects, s.bytes),
            ))
        }
        StoreCmd::Verify => {
            let report = store.verify_all()?;
            let text = if report.corrupt.is_empty() {
                format!("{} objects verified", report.checked)
            } else {
                format!(
                    "{} of {} objects are corrupt:\n{}",
                    report.corrupt.len(),
                    report.checked,
                    report
                        .corrupt
                        .iter()
                        .map(Hash::to_hex)
                        .collect::<Vec<_>>()
                        .join("\n")
                )
            };
            let exit = u8::from(!report.corrupt.is_empty());
            Ok(Outcome {
                json: json!(report),
                text,
                exit,
            })
        }
        StoreCmd::Reindex => reindex(&store),
        StoreCmd::Unreferenced => unreferenced(&store),
        StoreCmd::Gc { .. } => unreachable!("handled above"),
    }
}

fn reindex(store: &Store) -> Result<Outcome> {
    let start = Instant::now();
    let index = Index::read(store)?;
    let ms = start.elapsed().as_secs_f64() * 1000.0;
    warn_bad_lines(&index)?;
    let unreadable = index.unreadable()?;
    let entries = index.entry_count();
    let recordings = index.recording_count();
    let mut lines = vec![format!(
        "read {entries} log entries and {recordings} recordings in {ms:.0} ms: {} (gameprinter keeps no index file; the log is read each time)",
        recordings::log_path(store).display()
    )];
    for u in &unreadable {
        lines.push(format!("unreadable  {}  {}", u.id, u.error));
    }
    Ok(Outcome::ok(
        json!({
            "index": null,
            "synced": {
                "entries": entries,
                "recordings": recordings,
                "unreadable": unreadable.len(),
            },
            "unreadable": unreadable,
            "duration_ms": ms,
        }),
        lines.join("\n"),
    ))
}

fn unreferenced(store: &Store) -> Result<Outcome> {
    let index = Index::read(store)?;
    warn_bad_lines(&index)?;
    let objects = index.unreferenced_with(store, false)?;
    let unreadable = index.unreadable()?;
    let bad_lines = index.bad_lines()?;
    let bytes: u64 = objects.iter().map(|o| o.bytes).sum();
    let mut lines: Vec<String> = objects
        .iter()
        .map(|o| format!("{}  {:>10} bytes", o.hash, o.bytes))
        .collect();
    lines.push(format!(
        "{} objects ({bytes} bytes) are used by no recording in the log. Nothing was deleted.",
        objects.len()
    ));
    for u in &unreadable {
        lines.push(format!(
            "can't read the manifest of {}: {}. The objects it uses may be listed above.",
            u.id, u.error
        ));
    }
    if !bad_lines.is_empty() {
        lines.push(format!(
            "{} recordings log lines can't be read. The objects their recordings use may be listed above.",
            bad_lines.len()
        ));
    }
    let incomplete = !unreadable.is_empty() || !bad_lines.is_empty();
    Ok(Outcome {
        json: json!({
            "objects": objects,
            "bytes": bytes,
            "unreadable": unreadable,
            "bad_lines": bad_lines.len(),
            "complete": !incomplete,
        }),
        text: lines.join("\n"),
        exit: if incomplete { 2 } else { 0 },
    })
}

// ---- gp store gc ---------------------------------------------------------
//
// Deletes store objects that no recording uses, once they have stayed
// unused across two runs (docs/recording.md, "Deleting unused objects").
//
// Each run with `--yes` writes the objects it found unused to
// `<store>/gc/pending.json`. The next run with `--yes` deletes those that
// are still unused and writes the new list. Without `--yes` a run lists
// what it would delete and writes nothing. It holds the store's lock
// exclusively, so it never runs while another command writes, and it
// refuses to delete anything while a manifest or log line can't be read,
// since the objects that recording uses would then look unused. Objects
// named in the coverage index (`<store>/index/coverage.jsonl`) or the
// validation index (`<store>/index/validation.jsonl`) are kept, and an
// unreadable line there stops it the same way. The coverage index is read
// through gp-coverage's cache, as before; gameprinter has no gp-validate
// yet, so the validation index's lines are read here as JSON with the
// fields that crate wrote.
//
// With `--retired`, objects that only retired recordings use count as
// unused. A retired recording is deleted whole or not at all: its objects
// go only in a run that deletes its manifest, and just before that run
// deletes anything it appends a `deleted` mark to the recordings log, so
// the recording leaves every listing instead of pointing at missing
// objects.

/// `<store>/gc/pending.json`: what the last `gp store gc --yes` found
/// unused.
#[derive(Debug, Serialize, Deserialize)]
struct Pending {
    format: u32,
    /// RFC 3339.
    at: String,
    /// Whether that run counted retired recordings' objects.
    retired: bool,
    objects: Vec<Hash>,
}

const PENDING_FORMAT: u32 = 1;

fn pending_path(store: &Store) -> PathBuf {
    store.root().join("gc").join("pending.json")
}

/// The last run's list, or none if there isn't one. A list that can't be
/// read counts as none: that only delays deleting by one run.
fn read_pending(store: &Store) -> Option<Pending> {
    let path = pending_path(store);
    let text = match fs::read_to_string(&path) {
        Ok(t) => t,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return None,
        Err(e) => {
            eprintln!(
                "gp: can't read {} ({e}); starting a new list",
                path.display()
            );
            return None;
        }
    };
    match serde_json::from_str::<Pending>(&text) {
        Ok(p) if p.format == PENDING_FORMAT => Some(p),
        Ok(p) => {
            eprintln!(
                "gp: {} is format {}, not {PENDING_FORMAT}; starting a new list",
                path.display(),
                p.format
            );
            None
        }
        Err(e) => {
            eprintln!(
                "gp: can't read {} ({e}); starting a new list",
                path.display()
            );
            None
        }
    }
}

fn write_pending(store: &Store, pending: &Pending) -> Result<()> {
    let path = pending_path(store);
    let dir = path.parent().expect("pending path has a parent");
    fs::create_dir_all(dir).with_context(|| format!("creating {}", dir.display()))?;
    let tmp = dir.join(format!("pending.json.{}", std::process::id()));
    fs::write(&tmp, serde_json::to_string_pretty(pending)?)
        .with_context(|| format!("writing {}", tmp.display()))?;
    fs::rename(&tmp, &path).with_context(|| format!("writing {}", path.display()))?;
    Ok(())
}

/// Lines of a log that can't be read: line numbers (from 1) and errors.
type BadLines = Vec<(usize, String)>;

/// The objects a cache index (`<store>/index/<name>.jsonl`) names in the
/// given fields (the first required, the rest optional), and its lines
/// that can't be read (line numbers from 1).
fn cache_objects(store: &Store, name: &str, fields: &[&str]) -> Result<(Vec<Hash>, BadLines)> {
    let path = store.root().join("index").join(format!("{name}.jsonl"));
    let text = match fs::read_to_string(&path) {
        Ok(t) => t,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Default::default()),
        Err(e) => return Err(e).with_context(|| format!("reading {}", path.display())),
    };
    let (mut good, mut bad) = (Vec::new(), Vec::new());
    for (i, l) in text.lines().enumerate() {
        if l.trim().is_empty() {
            continue;
        }
        let read = || -> std::result::Result<Vec<Hash>, String> {
            let v: serde_json::Value = serde_json::from_str(l).map_err(|e| e.to_string())?;
            let mut out = Vec::new();
            for (n, f) in fields.iter().enumerate() {
                match v.get(f) {
                    // The first field is required, the rest may be null.
                    None | Some(serde_json::Value::Null) if n > 0 => {}
                    None => return Err(format!("missing field `{f}`")),
                    Some(h) => out.push(
                        serde_json::from_value::<Hash>(h.clone()).map_err(|e| e.to_string())?,
                    ),
                }
            }
            Ok(out)
        };
        match read() {
            Ok(h) => good.extend(h),
            Err(e) => bad.push((i + 1, e)),
        }
    }
    Ok((good, bad))
}

fn gc(drop_retired: bool, yes: bool, min_age_minutes: u32, paths: &Paths) -> Result<Outcome> {
    let store = Store::open_existing(&paths.store)?;
    // A dry run writes nothing, so it needs no lock and works on a store
    // mounted read-only.
    if yes && !store.try_lock_exclusive()? {
        bail!(
            "another gp command is writing to {} (a recording in progress?); run gc again once it has finished",
            store.root().display()
        );
    }
    let index = Index::read(&store)?;
    warn_bad_lines(&index)?;
    let unreadable = index.unreadable()?;
    let bad_lines = index.bad_lines()?;
    // Stored coverage results are named by the coverage index, not by a
    // recording; every object a line names is kept.
    let (coverage, bad_coverage) = gp_coverage::cache::lines_checked(&store)?;
    // So are validation results, by the validation index.
    let (validation, bad_validation) = cache_objects(&store, "validation", &["object"])?;
    if !unreadable.is_empty()
        || !bad_lines.is_empty()
        || !bad_coverage.is_empty()
        || !bad_validation.is_empty()
    {
        let mut lines = Vec::new();
        for u in &unreadable {
            lines.push(format!("can't read the manifest of {}: {}", u.id, u.error));
        }
        if !bad_lines.is_empty() {
            lines.push(format!(
                "{} recordings log lines can't be read",
                bad_lines.len()
            ));
        }
        for (line, error) in &bad_coverage {
            lines.push(format!(
                "coverage index line {line} can't be read ({error}); the object it names would look unused"
            ));
        }
        for (line, error) in &bad_validation {
            lines.push(format!(
                "validation index line {line} can't be read ({error}); the object it names would look unused"
            ));
        }
        lines.push(
            "Nothing was deleted: the objects those recordings use would look unused. Fix or restore them first (`gp recordings show`, `gp store reindex`)."
                .into(),
        );
        return Ok(Outcome {
            json: json!({
                "complete": false,
                "unreadable": unreadable,
                "bad_lines": bad_lines.len(),
                "bad_coverage_lines": bad_coverage.len(),
                "bad_validation_lines": bad_validation.len(),
                "deleted": [],
            }),
            text: lines.join("\n"),
            exit: 2,
        });
    }

    let kept: HashSet<Hash> = coverage
        .iter()
        .flat_map(|l| std::iter::once(l.object).chain(l.definition))
        .chain(validation)
        .collect();
    let unused: Vec<_> = index
        .unreferenced_with(&store, drop_retired)?
        .into_iter()
        .filter(|o| !kept.contains(&o.hash))
        .collect();
    let unused_bytes: u64 = unused.iter().map(|o| o.bytes).sum();
    let previous = read_pending(&store);
    // The list must have waited `min_age_minutes`, so two runs back to back
    // can't skip the wait a writer from an older `gp` (which takes no lock)
    // relies on. A list too young is kept as it is, and its age keeps
    // counting.
    let age_minutes = previous.as_ref().and_then(|p| minutes_since(&p.at));
    let too_young =
        previous.is_some() && age_minutes.is_none_or(|age| age < f64::from(min_age_minutes));
    let was_pending: HashSet<Hash> = previous
        .as_ref()
        .filter(|_| !too_young)
        .map(|p| p.objects.iter().copied().collect())
        .unwrap_or_default();
    let mut delete: Vec<_> = unused
        .iter()
        .filter(|o| was_pending.contains(&o.hash))
        .cloned()
        .collect();
    // A retired recording goes whole: if its manifest stays, so does every
    // object it uses.
    let mut dropping = Vec::new();
    if drop_retired {
        let mut retired: Vec<Hash> = index.retired()?.into_iter().collect();
        retired.sort();
        for id in retired {
            if delete.iter().any(|o| o.hash == id) {
                dropping.push(id);
            } else {
                let keep: HashSet<Hash> = index.objects(&id)?.iter().map(|o| o.hash).collect();
                delete.retain(|o| !keep.contains(&o.hash));
            }
        }
    }
    let names: Vec<String> = dropping
        .iter()
        .map(|id| {
            let name = index
                .entries_for(id)
                .ok()
                .and_then(|e| e.last().map(|e| e.name.clone()))
                .unwrap_or_default();
            format!("{} {name}", &id.to_hex()[..12])
        })
        .collect();
    let delete_bytes: u64 = delete.iter().map(|o| o.bytes).sum();
    let deleting: HashSet<Hash> = delete.iter().map(|o| o.hash).collect();
    let next: Vec<Hash> = unused
        .iter()
        .map(|o| o.hash)
        .filter(|h| !deleting.contains(h))
        .collect();
    let since = previous
        .as_ref()
        .map_or_else(|| "never".to_owned(), |p| p.at.clone());
    let scope = if drop_retired {
        "used by no recording, or only by retired ones"
    } else {
        "used by no recording"
    };
    let mut lines = vec![format!(
        "{} objects ({unused_bytes} bytes) are {scope}. Last gc run with --yes: {since}.",
        unused.len()
    )];
    for n in &names {
        lines.push(format!("retired recording {n}"));
    }

    if !yes {
        if too_young {
            lines.push(format!(
                "The last run's list is {:.0} minutes old, under --min-age {min_age_minutes}, so the next --yes deletes nothing yet.",
                age_minutes.unwrap_or(0.0)
            ));
        }
        for o in &delete {
            lines.push(format!("{}  {:>10} bytes", o.hash, o.bytes));
        }
        lines.push(format!(
            "Would delete {} objects ({delete_bytes} bytes), unused at the last run too{}. Nothing was deleted; run with --yes to delete them and list the other {} for the run after.",
            delete.len(),
            if dropping.is_empty() {
                String::new()
            } else {
                format!(", and drop {} retired recordings", dropping.len())
            },
            next.len()
        ));
        return Ok(Outcome::ok(
            json!({
                "complete": true,
                "yes": false,
                "unused": unused.len(),
                "unused_bytes": unused_bytes,
                "previous_run": previous.as_ref().map(|p| &p.at),
                "would_delete": delete,
                "would_delete_bytes": delete_bytes,
                "would_drop": dropping,
                "deleted": [],
            }),
            lines.join("\n"),
        ));
    }

    // The marks go first: if deleting stops halfway, what's left of a
    // dropped recording is unused, not a recording with missing objects.
    let at = crate::open::now_rfc3339();
    for id in &dropping {
        recordings::append_mark(
            &store,
            &Mark {
                mark: MarkKind::Deleted,
                id: *id,
                at: at.clone(),
                note: Some("gp store gc --retired".into()),
            },
        )?;
    }
    let mut freed = 0u64;
    let mut deleted = 0u64;
    for o in &delete {
        if let Some(bytes) = store.delete(&o.hash)? {
            freed += bytes;
            deleted += 1;
        }
    }
    if !too_young {
        write_pending(
            &store,
            &Pending {
                format: PENDING_FORMAT,
                at: at.clone(),
                retired: drop_retired,
                objects: next.clone(),
            },
        )?;
    }
    drop(index);
    // The old tool brought its SQLite index up to date here; the log is
    // read each time now, so there's nothing to bring up to date.
    let indexed = Index::read(&store).is_ok();
    if too_young {
        lines.push(format!(
            "The last run's list is {:.0} minutes old, under --min-age {min_age_minutes}; nothing was deleted, and the list is kept until it's old enough.",
            age_minutes.unwrap_or(0.0)
        ));
    } else {
        lines.push(format!(
            "Deleted {deleted} objects ({freed} bytes){}. {} unused objects are listed for the next run, which deletes those still unused then.",
            if dropping.is_empty() {
                String::new()
            } else {
                format!(" and dropped {} retired recordings", dropping.len())
            },
            next.len()
        ));
    }
    Ok(Outcome::ok(
        json!({
            "complete": true,
            "yes": true,
            "waiting": too_young,
            "unused": unused.len(),
            "unused_bytes": unused_bytes,
            "previous_run": previous.as_ref().map(|p| &p.at),
            "deleted": delete,
            "deleted_objects": deleted,
            "freed_bytes": freed,
            "dropped": dropping,
            "pending": next.len(),
            "indexed": indexed,
        }),
        lines.join("\n"),
    ))
}

/// Minutes from an RFC 3339 time to now, or none if it can't be read.
fn minutes_since(at: &str) -> Option<f64> {
    let then = rfc3339_seconds(at)?;
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .ok()?
        .as_secs_f64();
    Some((now - then) / 60.0)
}

/// Seconds since 1970 of an RFC 3339 time (`2026-11-12T09:00:00.5Z` or with
/// a `+hh:mm` offset), or none if it can't be read.
fn rfc3339_seconds(at: &str) -> Option<f64> {
    let (date, rest) = at.split_once(['T', 't'])?;
    let mut d = date.splitn(3, '-');
    let y: i64 = d.next()?.parse().ok()?;
    let m: i64 = d.next()?.parse().ok()?;
    let day: i64 = d.next()?.parse().ok()?;
    let (clock, offset) = if let Some(c) = rest.strip_suffix(['Z', 'z']) {
        (c, 0i64)
    } else {
        let i = rest.rfind(['+', '-'])?;
        let (c, o) = rest.split_at(i);
        let sign = if o.starts_with('-') { -1 } else { 1 };
        let (oh, om) = o[1..].split_once(':')?;
        (
            c,
            sign * (oh.parse::<i64>().ok()? * 3600 + om.parse::<i64>().ok()? * 60),
        )
    };
    let mut c = clock.splitn(3, ':');
    let h: i64 = c.next()?.parse().ok()?;
    let mi: i64 = c.next()?.parse().ok()?;
    let s: f64 = c.next()?.parse().ok()?;
    if !(1..=12).contains(&m) || !(1..=31).contains(&day) {
        return None;
    }
    // Howard Hinnant's days-from-civil.
    let yy = if m <= 2 { y - 1 } else { y };
    let era = yy.div_euclid(400);
    let yoe = yy - era * 400;
    let mp = if m > 2 { m - 3 } else { m + 9 };
    let doy = (153 * mp + 2) / 5 + day - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = era * 146_097 + doe - 719_468;
    Some((days * 86_400 + h * 3600 + mi * 60 - offset) as f64 + s)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rfc3339_reads_utc_and_offsets() {
        assert_eq!(rfc3339_seconds("1970-01-01T00:00:00Z"), Some(0.0));
        assert_eq!(
            rfc3339_seconds("1970-01-02T00:00:00.000000Z"),
            Some(86_400.0)
        );
        assert_eq!(rfc3339_seconds("1970-01-01T01:00:00+01:00"), Some(0.0));
        assert_eq!(
            rfc3339_seconds("2026-11-12T00:00:00Z"),
            Some(1_794_441_600.0)
        );
        assert_eq!(rfc3339_seconds("yesterday"), None);
    }

    #[test]
    fn rfc3339_agrees_with_now() {
        let now = crate::open::now_rfc3339();
        let age = minutes_since(&now).unwrap();
        assert!((0.0..1.0).contains(&age), "{now}: {age}");
    }
}
