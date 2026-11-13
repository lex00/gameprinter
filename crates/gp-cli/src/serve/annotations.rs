//! Recording annotations (docs/formats/annotations.md): people's notes on
//! frame ranges of a recording, kept in a file of their own per recording,
//! `$GP_HOME/annotations/<recording ID>.toml`, so recordings stay
//! immutable. These are the only endpoints that write, and they write only
//! that folder, never the store.
//!
//! A write takes the lock `ANNOTATIONS`, reads the file, changes it
//! and replaces it whole (written to a temporary file in the same folder,
//! synced and renamed over it), so two requests to one server can't lose
//! each other's notes. The lock is the process's own: two `gp serve`
//! processes on one `$GP_HOME`, or an edit by hand during a write, can
//! still lose one (docs/open-questions.md, "Should annotation writes lock
//! across processes?").

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use serde_json::json;

use super::{Ctx, Fail, Params, Route};
use crate::mechanic::http::{Request, Response};

/// The format's version.
pub(crate) const SCHEMA: u32 = 0;

/// The longest note, in characters.
const MAX_TEXT: usize = 4000;
/// The most tags a note has, and the longest tag.
const MAX_TAGS: usize = 16;
const MAX_TAG: usize = 32;

/// Paths that take POST, PUT or DELETE as well as GET.
pub(super) const WRITABLE: &[&str] = &[
    "/api/recordings/{id}/annotations",
    "/api/recordings/{id}/annotations/{note}",
];

pub(super) fn routes() -> Vec<Route> {
    vec![
        Route {
            path: WRITABLE[0],
            about: "The recording's annotations; POST a JSON note ({from, to, text, tags}) to add one",
            handler: collection,
        },
        Route {
            path: WRITABLE[1],
            about: "One annotation by its number; PUT a JSON note to replace it, DELETE to remove it",
            handler: one,
        },
    ]
}

/// An annotations file.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct File {
    pub schema: u32,
    /// The recording's ID in full: its manifest's hash.
    pub recording: String,
    /// The number the next note gets. Numbers aren't reused, so a link to
    /// a deleted note never finds another.
    pub next: u64,
    #[serde(default, rename = "note")]
    pub notes: Vec<Note>,
}

/// One note on frames `from` to `to`, both included.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Note {
    pub id: u64,
    pub from: u64,
    pub to: u64,
    pub text: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tags: Vec<String>,
    pub created_at: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub updated_at: Option<String>,
}

/// What a client sends to add or replace a note.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Draft {
    from: u64,
    to: u64,
    text: String,
    #[serde(default)]
    tags: Vec<String>,
}

impl File {
    pub fn new(recording: &str) -> Self {
        Self {
            schema: SCHEMA,
            recording: recording.to_owned(),
            next: 1,
            notes: Vec::new(),
        }
    }

    /// Parses a file and checks it belongs to `recording` with `frames`
    /// frames.
    pub fn parse(text: &str, recording: &str, frames: u64) -> Result<Self, String> {
        let file: File = toml::from_str(text).map_err(|e| e.to_string())?;
        if file.schema != SCHEMA {
            return Err(format!(
                "annotations schema {}; this build reads {SCHEMA}",
                file.schema
            ));
        }
        if file.recording != recording {
            return Err(format!(
                "the file is for recording {}, not {recording}",
                file.recording
            ));
        }
        let mut ids: Vec<u64> = file.notes.iter().map(|n| n.id).collect();
        ids.sort_unstable();
        if ids.windows(2).any(|w| w[0] == w[1]) {
            return Err("two notes have the same id".into());
        }
        if ids.last().is_some_and(|&max| max >= file.next) {
            return Err("a note's id isn't below `next`".into());
        }
        for n in &file.notes {
            check(n.from, n.to, &n.text, &n.tags, frames)
                .map_err(|e| format!("note {}: {e}", n.id))?;
        }
        Ok(file)
    }

    pub fn to_toml(&self) -> String {
        format!(
            "# Annotations of a recording, written by gp serve (docs/formats/annotations.md).\n{}",
            toml::to_string(self).expect("annotations serialize")
        )
    }
}

/// Checks a note's range, text and tags.
fn check(from: u64, to: u64, text: &str, tags: &[String], frames: u64) -> Result<(), String> {
    if from > to {
        return Err(format!("from ({from}) is after to ({to})"));
    }
    if to > frames {
        return Err(format!(
            "to ({to}) is past the recording's last frame, {frames}"
        ));
    }
    if text.trim().is_empty() {
        return Err("the text is empty".into());
    }
    if text.chars().count() > MAX_TEXT {
        return Err(format!("the text is over {MAX_TEXT} characters"));
    }
    if tags.len() > MAX_TAGS {
        return Err(format!("more than {MAX_TAGS} tags"));
    }
    for t in tags {
        let ok = !t.is_empty()
            && t.len() <= MAX_TAG
            && t.bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-');
        if !ok {
            return Err(format!(
                "tag {t:?}: tags are 1 to {MAX_TAG} lower-case letters, digits and dashes"
            ));
        }
    }
    Ok(())
}

/// The folder annotations live in.
pub(crate) fn dir(ctx: &Ctx) -> PathBuf {
    dir_in(&ctx.paths.home)
}

fn dir_in(home: &Path) -> PathBuf {
    home.join("annotations")
}

fn file_path(ctx: &Ctx, recording: &str) -> PathBuf {
    dir(ctx).join(format!("{recording}.toml"))
}

/// The recording's annotations file, or an empty one when there's none.
fn load(path: &Path, recording: &str, frames: u64) -> Result<File, Fail> {
    match std::fs::read_to_string(path) {
        Ok(text) => File::parse(&text, recording, frames).map_err(|e| Fail {
            status: 500,
            message: format!("{}: {e}", path.display()),
        }),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(File::new(recording)),
        Err(e) => Err(anyhow::anyhow!("reading {}: {e}", path.display()).into()),
    }
}

/// Replaces the file whole: written next to it and renamed over it.
/// Refuses a folder inside a git work tree (annotations of a commercial
/// game are as private as its recordings) and a symlinked folder or file.
fn save(ctx: &Ctx, path: &Path, file: &File) -> Result<(), Fail> {
    save_in(&ctx.paths.home, path, file)
}

fn save_in(home: &Path, path: &Path, file: &File) -> Result<(), Fail> {
    let dir = dir_in(home);
    let fail = |message: String| Fail {
        status: 500,
        message,
    };
    if let Some(repo) = gp_evidence::paths::enclosing_git_work_tree(home) {
        return Err(fail(format!(
            "$GP_HOME is inside the git work tree {}; annotations aren't written there",
            repo.display()
        )));
    }
    for p in [&dir, &path.to_path_buf()] {
        if std::fs::symlink_metadata(p).is_ok_and(|m| m.file_type().is_symlink()) {
            return Err(fail(format!(
                "{} is a symlink; annotations aren't written through one",
                p.display()
            )));
        }
    }
    std::fs::create_dir_all(&dir).map_err(|e| fail(format!("creating {}: {e}", dir.display())))?;
    let tmp = dir.join(format!(".{}.{}.tmp", file.recording, std::process::id()));
    {
        use std::io::Write as _;
        let mut f = std::fs::File::create(&tmp)
            .map_err(|e| fail(format!("writing {}: {e}", tmp.display())))?;
        f.write_all(file.to_toml().as_bytes())
            .and_then(|()| f.sync_all())
            .map_err(|e| fail(format!("writing {}: {e}", tmp.display())))?;
    }
    std::fs::rename(&tmp, path).map_err(|e| fail(format!("replacing {}: {e}", path.display())))?;
    Ok(())
}

fn draft(req: &Request, frames: u64) -> Result<Draft, Fail> {
    let d: Draft = serde_json::from_slice(&req.body)
        .map_err(|e| Fail::bad_request(format!("the body isn't a note: {e}")))?;
    check(d.from, d.to, &d.text, &d.tags, frames).map_err(Fail::bad_request)?;
    Ok(d)
}

fn now() -> String {
    crate::open::now_rfc3339()
}

/// Held while an annotations file is read and replaced (game-printer kept
/// it in the server's context).
static ANNOTATIONS: std::sync::Mutex<()> = std::sync::Mutex::new(());

fn method_not_allowed(allowed: &str) -> Fail {
    Fail {
        status: 405,
        message: format!("this endpoint takes {allowed}"),
    }
}

fn collection(ctx: &Ctx, req: &Request, params: &Params) -> Result<Response, Fail> {
    let opened = super::recordings::open(ctx, params.get("id").map_or("", String::as_str))?;
    let id = opened.entry.id.to_hex();
    let frames = opened.manifest.frames;
    let path = file_path(ctx, &id);
    match req.method.as_str() {
        "GET" | "HEAD" => {
            let file = load(&path, &id, frames)?;
            Ok(Response::json(&json!({
                "id": id,
                "file": path,
                "annotations": file.notes,
            })))
        }
        "POST" => {
            let d = draft(req, frames)?;
            let _guard = ANNOTATIONS.lock().unwrap_or_else(|e| e.into_inner());
            let mut file = load(&path, &id, frames)?;
            let note = Note {
                id: file.next,
                from: d.from,
                to: d.to,
                text: d.text,
                tags: d.tags,
                created_at: now(),
                updated_at: None,
            };
            file.next += 1;
            file.notes.push(note.clone());
            save(ctx, &path, &file)?;
            Ok(Response::json(
                &json!({ "id": id, "file": path, "annotation": note }),
            ))
        }
        _ => Err(method_not_allowed("GET, HEAD and POST")),
    }
}

fn one(ctx: &Ctx, req: &Request, params: &Params) -> Result<Response, Fail> {
    let opened = super::recordings::open(ctx, params.get("id").map_or("", String::as_str))?;
    let id = opened.entry.id.to_hex();
    let frames = opened.manifest.frames;
    let path = file_path(ctx, &id);
    let text = params.get("note").map_or("", String::as_str);
    let n: u64 = text
        .parse()
        .map_err(|_| Fail::bad_request(format!("a note is a number, not {text:?}")))?;
    let missing = || Fail::not_found(format!("recording {} has no note {n}", opened.short()));
    match req.method.as_str() {
        "GET" | "HEAD" => {
            let file = load(&path, &id, frames)?;
            let note = file
                .notes
                .into_iter()
                .find(|x| x.id == n)
                .ok_or_else(missing)?;
            Ok(Response::json(
                &json!({ "id": id, "file": path, "annotation": note }),
            ))
        }
        "PUT" => {
            let d = draft(req, frames)?;
            let _guard = ANNOTATIONS.lock().unwrap_or_else(|e| e.into_inner());
            let mut file = load(&path, &id, frames)?;
            let note = file
                .notes
                .iter_mut()
                .find(|x| x.id == n)
                .ok_or_else(missing)?;
            note.from = d.from;
            note.to = d.to;
            note.text = d.text;
            note.tags = d.tags;
            note.updated_at = Some(now());
            let note = note.clone();
            save(ctx, &path, &file)?;
            Ok(Response::json(
                &json!({ "id": id, "file": path, "annotation": note }),
            ))
        }
        "DELETE" => {
            let _guard = ANNOTATIONS.lock().unwrap_or_else(|e| e.into_inner());
            let mut file = load(&path, &id, frames)?;
            let before = file.notes.len();
            file.notes.retain(|x| x.id != n);
            if file.notes.len() == before {
                return Err(missing());
            }
            save(ctx, &path, &file)?;
            Ok(Response::json(
                &json!({ "id": id, "file": path, "deleted": n }),
            ))
        }
        _ => Err(method_not_allowed("GET, HEAD, PUT and DELETE")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const REC: &str = "ab12";

    #[test]
    fn a_file_round_trips() {
        let mut f = File::new(REC);
        f.notes.push(Note {
            id: 1,
            from: 3,
            to: 9,
            text: "jumps \"early\"\nsecond line".into(),
            tags: vec!["bug".into(), "jump-2".into()],
            created_at: "2026-10-28T00:00:00Z".into(),
            updated_at: None,
        });
        f.next = 2;
        let text = f.to_toml();
        assert_eq!(File::parse(&text, REC, 10).unwrap(), f);
    }

    #[test]
    fn bad_files_are_refused() {
        let ok = "schema = 0\nrecording = \"ab12\"\nnext = 2\n[[note]]\nid = 1\nfrom = 1\nto = 2\ntext = \"x\"\ncreated_at = \"t\"\n";
        assert!(File::parse(ok, REC, 10).is_ok());
        for (bad, why) in [
            (ok.replace("schema = 0", "schema = 1"), "schema"),
            (ok.replace("ab12", "cd34"), "recording"),
            (ok.replace("next = 2", "next = 1"), "next"),
            (ok.replace("to = 2", "to = 11"), "past the end"),
            (ok.replace("from = 1", "from = 3"), "backwards"),
            (ok.replace("text = \"x\"", "text = \" \""), "empty"),
            (ok.replace("next = 2", "next = 2\nextra = 1"), "unknown key"),
            (
                format!(
                    "{ok}[[note]]\nid = 1\nfrom = 1\nto = 1\ntext = \"y\"\ncreated_at = \"t\"\n"
                ),
                "same id",
            ),
        ] {
            assert!(File::parse(&bad, REC, 10).is_err(), "{why} was accepted");
        }
    }

    #[test]
    fn tags_are_plain_words() {
        assert!(check(0, 0, "x", &["a-1".into()], 0).is_ok());
        for t in ["", "Caps", "sp ace", "dots.", &"x".repeat(33)] {
            assert!(check(0, 0, "x", &[t.into()], 0).is_err(), "{t:?}");
        }
        assert!(check(0, 0, "x", &vec!["a".into(); 17], 0).is_err());
        assert!(check(0, 0, &"x".repeat(MAX_TEXT + 1), &[], 0).is_err());
    }
}
