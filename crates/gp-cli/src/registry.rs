//! `gp registry`: make a registry in a folder or an S3 bucket, re-sign its
//! list files, and test one from outside, as a browser would read it.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context, Result, anyhow};
use gp_package::registry::{Folder, Publish, Reader, Signer, Target, init, publish, reindex};
use gp_package::sig::{NAMESPACE, fingerprint};
use serde_json::json;

use crate::Outcome;

/// A registry in an S3 bucket, through the AWS CLI and its own login, so
/// gp never holds an AWS key.
pub struct S3 {
    bucket: String,
}

impl S3 {
    fn put(&self, key: &str, bytes: &[u8], once: bool) -> Result<Option<String>, String> {
        let tmp = tempfile::NamedTempFile::new().map_err(|e| e.to_string())?;
        fs::write(tmp.path(), bytes).map_err(|e| e.to_string())?;
        let mut c = Command::new("aws");
        c.args([
            "s3api",
            "put-object",
            "--bucket",
            &self.bucket,
            "--key",
            key,
            "--body",
        ])
        .arg(tmp.path());
        // A write-once key never changes, so a browser keeps it for good. A
        // list file is rewritten, so a browser asks each time.
        if once {
            c.args(["--if-none-match", "*"]);
            c.args(["--cache-control", "public, max-age=31536000, immutable"]);
        } else {
            c.args(["--cache-control", "no-cache"]);
        }
        let o = c.output().map_err(|e| format!("running aws: {e}"))?;
        if o.status.success() {
            return Ok(None);
        }
        Ok(Some(String::from_utf8_lossy(&o.stderr).trim().to_owned()))
    }
}

impl Target for S3 {
    fn read(&self, key: &str) -> Result<Option<Vec<u8>>, String> {
        let tmp = tempfile::NamedTempFile::new().map_err(|e| e.to_string())?;
        let o = Command::new("aws")
            .args([
                "s3api",
                "get-object",
                "--bucket",
                &self.bucket,
                "--key",
                key,
            ])
            .arg(tmp.path())
            .output()
            .map_err(|e| format!("running aws: {e}"))?;
        if o.status.success() {
            return fs::read(tmp.path()).map(Some).map_err(|e| e.to_string());
        }
        let err = String::from_utf8_lossy(&o.stderr);
        if err.contains("NoSuchKey") {
            return Ok(None);
        }
        Err(format!("{key}: {}", err.trim()))
    }

    fn write_once(&self, key: &str, bytes: &[u8]) -> Result<bool, String> {
        match self.put(key, bytes, true)? {
            None => Ok(true),
            Some(e) if e.contains("PreconditionFailed") => Ok(false),
            Some(e) => Err(format!("{key}: {e}")),
        }
    }

    fn replace(&self, key: &str, bytes: &[u8]) -> Result<(), String> {
        match self.put(key, bytes, false)? {
            None => Ok(()),
            Some(e) => Err(format!("{key}: {e}")),
        }
    }
}

/// A registry read over HTTPS, as a player reads it.
pub struct Http(pub String);

impl Target for Http {
    fn read(&self, key: &str) -> Result<Option<Vec<u8>>, String> {
        let url = format!("{}/{key}", self.0.trim_end_matches('/'));
        let tmp = tempfile::NamedTempFile::new().map_err(|e| e.to_string())?;
        let o = Command::new("curl")
            .args(["-sS", "--max-time", "60", "-o"])
            .arg(tmp.path())
            .args(["-w", "%{http_code}", &url])
            .output()
            .map_err(|e| format!("running curl: {e}"))?;
        let code = String::from_utf8_lossy(&o.stdout).trim().to_owned();
        match code.as_str() {
            "200" => fs::read(tmp.path()).map(Some).map_err(|e| e.to_string()),
            // A public bucket with no list permission answers 403 for a key
            // it doesn't hold.
            "404" | "403" => Ok(None),
            _ => Err(format!("{url}: HTTP {code}")),
        }
    }

    fn write_once(&self, key: &str, _: &[u8]) -> Result<bool, String> {
        Err(format!("{key}: an https registry is read only"))
    }

    fn replace(&self, key: &str, _: &[u8]) -> Result<(), String> {
        Err(format!("{key}: an https registry is read only"))
    }
}

/// A registry in a git checkout, named `git:PATH`: a folder whose changes each
/// command commits, and pushes when the repo has an `origin`. It's a folder
/// for writing, so a write-once key is refused the same way, and a repo host
/// serves it as static files.
pub struct Repo(PathBuf);

impl Repo {
    fn at(spec: &str) -> Option<Self> {
        spec.strip_prefix("git:").map(|p| Self(PathBuf::from(p)))
    }

    fn git(&self, args: &[&str]) -> Result<String> {
        let o = Command::new("git")
            .arg("-C")
            .arg(&self.0)
            .args(args)
            .output()
            .context("running git")?;
        if !o.status.success() {
            anyhow::bail!(
                "git {} in {}: {}",
                args.join(" "),
                self.0.display(),
                String::from_utf8_lossy(&o.stderr).trim()
            );
        }
        Ok(String::from_utf8_lossy(&o.stdout).trim().to_owned())
    }

    fn has_origin(&self) -> Result<bool> {
        Ok(self.git(&["remote"])?.lines().any(|r| r == "origin"))
    }

    /// Before a command writes: a clean checkout, at the remote's tip, so a
    /// key someone else published counts as taken.
    fn ready(&self) -> Result<()> {
        self.git(&["rev-parse", "--is-inside-work-tree"])?;
        let dirty = self.git(&["status", "--porcelain"])?;
        if !dirty.is_empty() {
            anyhow::bail!(
                "{} has changes not committed, so nothing is published from it",
                self.0.display()
            );
        }
        if self.has_origin()? && !self.git(&["ls-remote", "--heads", "origin"])?.is_empty() {
            self.git(&["pull", "-q", "--ff-only", "origin", "HEAD"])?;
        }
        Ok(())
    }

    /// After it: one commit for what the command wrote, then the push.
    fn commit(&self, message: &str) -> Result<()> {
        self.git(&["add", "-A"])?;
        if self.git(&["status", "--porcelain"])?.is_empty() {
            return Ok(());
        }
        self.git(&["commit", "-q", "-m", message])?;
        if self.has_origin()? {
            self.git(&["push", "-q", "origin", "HEAD"])?;
        }
        Ok(())
    }
}

/// `f`, with a repo target made ready first and committed after.
fn committing(target: &str, message: &str, f: impl FnOnce() -> Result<Outcome>) -> Result<Outcome> {
    let Some(repo) = Repo::at(target) else {
        return f();
    };
    repo.ready()?;
    let outcome = f()?;
    if outcome.exit == 0 {
        repo.commit(message)?;
    }
    Ok(outcome)
}

/// Nothing goes into a repo with a ROM's bytes in it: a repo's history keeps
/// what it's given. Every file of a package that names ROMs is compared with
/// each of them, and a ROM not found here stops the publish.
fn no_rom_bytes(dir: &Path) -> Result<()> {
    if !dir.join("package.toml").exists() {
        return Ok(());
    }
    let pkg = crate::package::Package::load(dir)?;
    let wanted = pkg.rom_hashes();
    if wanted.is_empty() {
        return Ok(());
    }
    let paths = gp_evidence::Paths::from_env()?;
    let found = crate::roms::find(&wanted, &[], &paths.roms)?;
    if let Some(gone) = wanted.iter().find(|h| !found.contains_key(*h)) {
        anyhow::bail!(
            "ROM {gone} isn't in $GP_ROM_DIR, so this package can't be compared with it before going into a repo"
        );
    }
    let mut stack = vec![dir.to_owned()];
    while let Some(d) = stack.pop() {
        for e in fs::read_dir(&d).with_context(|| d.display().to_string())? {
            let p = e?.path();
            if p.is_dir() {
                stack.push(p);
                continue;
            }
            let bytes = fs::read(&p)?;
            for (id, rom) in &found {
                if let Some((at, len)) = crate::served::runs_of_rom(&bytes, rom.data()).first() {
                    anyhow::bail!(
                        "{} holds {len} bytes of ROM {} at ROM offset {at:#x}; nothing was published",
                        p.display(),
                        &id[..12]
                    );
                }
            }
        }
    }
    Ok(())
}

/// A folder, `git:PATH`, `s3://BUCKET` or an https address.
pub fn open(spec: &str) -> Box<dyn Target> {
    if let Some(repo) = Repo::at(spec) {
        Box::new(Folder(repo.0))
    } else if let Some(bucket) = spec.strip_prefix("s3://") {
        Box::new(S3 {
            bucket: bucket.trim_end_matches('/').to_owned(),
        })
    } else if spec.starts_with("https://") || spec.starts_with("http://") {
        Box::new(Http(spec.to_owned()))
    } else {
        Box::new(Folder(PathBuf::from(spec)))
    }
}

fn public_of(key: &Path) -> Result<String> {
    let p = PathBuf::from(format!("{}.pub", key.display()));
    Ok(fs::read_to_string(&p)
        .with_context(|| format!("reading {}", p.display()))?
        .trim()
        .to_owned())
}

fn ssh_sign(key: &Path, data: &[u8]) -> Result<Vec<u8>, String> {
    let dir = tempfile::tempdir().map_err(|e| e.to_string())?;
    let f = dir.path().join("d");
    fs::write(&f, data).map_err(|e| e.to_string())?;
    let o = Command::new("ssh-keygen")
        .args(["-q", "-Y", "sign", "-n", NAMESPACE, "-f"])
        .arg(key)
        .arg(&f)
        .output()
        .map_err(|e| format!("running ssh-keygen: {e}"))?;
    if !o.status.success() {
        return Err(String::from_utf8_lossy(&o.stderr).trim().to_owned());
    }
    fs::read(f.with_extension("sig")).map_err(|e| e.to_string())
}

/// `f` given a signer for the key file.
fn signing<T>(key: &Path, f: impl FnOnce(&Signer) -> Result<T, String>) -> Result<T> {
    let public = public_of(key)?;
    let path = key.to_owned();
    let sign = move |d: &[u8]| ssh_sign(&path, d);
    f(&Signer {
        public,
        sign: &sign,
    })
    .map_err(|e| anyhow!(e))
}

/// The UTC time, whole seconds, in the registry's RFC 3339 form.
fn utc(after_days: u32) -> Result<String> {
    let o = Command::new("date")
        .args([
            "-u",
            "-v",
            &format!("+{after_days}d"),
            "+%Y-%m-%dT%H:%M:%SZ",
        ])
        .output();
    let o = match o {
        Ok(o) if o.status.success() => o,
        _ => Command::new("date")
            .args([
                "-u",
                "-d",
                &format!("+{after_days} days"),
                "+%Y-%m-%dT%H:%M:%SZ",
            ])
            .output()
            .context("running date")?,
    };
    Ok(String::from_utf8_lossy(&o.stdout).trim().to_owned())
}

pub fn init_cmd(
    target: &str,
    name: &str,
    root_key: &Path,
    signing_key: &Path,
    namespaces: &[String],
) -> Result<Outcome> {
    committing(target, &format!("Make registry {name}"), || {
        init_inner(target, name, root_key, signing_key, namespaces)
    })
}

fn init_inner(
    target: &str,
    name: &str,
    root_key: &Path,
    signing_key: &Path,
    namespaces: &[String],
) -> Result<Outcome> {
    let t = open(target);
    let mut ns = Vec::new();
    for n in namespaces {
        let (name, key) = n
            .split_once('=')
            .with_context(|| format!("--namespace wants NS=KEY_FILE, not {n}"))?;
        ns.push((name.to_owned(), vec![public_of(Path::new(key))?]));
    }
    let today = utc(0)?[..10].to_owned();
    let index = public_of(signing_key)?;
    signing(root_key, |r| init(&*t, name, r, &[index], &ns, &today))?;
    let fp = fingerprint(&public_of(root_key)?).map_err(|e| anyhow!(e))?;
    Ok(Outcome {
        json: json!({ "name": name, "root": fp }),
        text: format!("made registry {name} at {target}; its root is {fp}"),
        exit: 0,
    })
}

/// Publishes a folder of text files as one exact version.
pub fn publish_cmd(
    target: &str,
    dir: &Path,
    (name, version): (&str, &str),
    (kind, title): (&str, &str),
    for_games: &[String],
    key: &Path,
) -> Result<Outcome> {
    if Repo::at(target).is_some() {
        no_rom_bytes(dir)?;
    }
    committing(target, &format!("Publish {name}@{version}"), || {
        publish_inner(target, dir, (name, version), (kind, title), for_games, key)
    })
}

fn publish_inner(
    target: &str,
    dir: &Path,
    (name, version): (&str, &str),
    (kind, title): (&str, &str),
    for_games: &[String],
    key: &Path,
) -> Result<Outcome> {
    let t = open(target);
    let mut files = BTreeMap::new();
    let mut stack = vec![dir.to_owned()];
    while let Some(d) = stack.pop() {
        for e in fs::read_dir(&d).with_context(|| d.display().to_string())? {
            let p = e?.path();
            if p.is_dir() {
                stack.push(p);
            } else {
                let rel = p.strip_prefix(dir)?.to_string_lossy().replace('\\', "/");
                files.insert(rel, fs::read(&p)?);
            }
        }
    }
    let published = utc(0)?;
    let p = Publish {
        name,
        version,
        kind,
        title,
        for_games,
        published: &published,
    };
    let id = signing(key, |s| publish(&*t, &p, &files, s))?;
    Ok(Outcome {
        json: json!({ "name": name, "version": version, "id": id, "files": files.len() }),
        text: format!("published {name}@{version}: {id} ({} files)", files.len()),
        exit: 0,
    })
}

pub fn reindex_cmd(target: &str, versions: &[String], key: &Path) -> Result<Outcome> {
    committing(target, "Re-sign the list files", || {
        reindex_inner(target, versions, key)
    })
}

fn reindex_inner(target: &str, versions: &[String], key: &Path) -> Result<Outcome> {
    let t = open(target);
    let mut list = Vec::new();
    for v in versions {
        let (n, ver) = v
            .rsplit_once('@')
            .with_context(|| format!("wanted name@version, not {v}"))?;
        list.push((n.to_owned(), ver.to_owned()));
    }
    let (now, expires) = (utc(0)?, utc(7)?);
    signing(key, |s| reindex(&*t, &list, s, &now, &expires))?;
    Ok(Outcome {
        json: json!({ "versions": list.len(), "expires": expires }),
        text: format!("re-signed the list files; they expire at {expires}"),
        exit: 0,
    })
}

/// CORS as a browser's fetch from another origin needs it.
fn cors(url: &str) -> Result<bool> {
    let o = Command::new("curl")
        .args([
            "-sS",
            "-o",
            "/dev/null",
            "-D",
            "-",
            "--max-time",
            "60",
            "-H",
        ])
        .arg("Origin: https://gameprinter.invalid")
        .arg(format!("{}/registry.toml", url.trim_end_matches('/')))
        .output()
        .context("running curl")?;
    let head = String::from_utf8_lossy(&o.stdout).to_lowercase();
    Ok(head
        .lines()
        .any(|l| l.starts_with("access-control-allow-origin:")))
}

/// Tests a registry from outside: CORS, signatures, expiry and the catalog,
/// and that every package the catalog lists fetches by its exact id.
pub fn check_cmd(url: &str, root_file: &Path) -> Result<Outcome> {
    let root = fs::read_to_string(root_file)
        .with_context(|| root_file.display().to_string())?
        .trim()
        .to_owned();
    let t = open(url);
    let now = utc(0)?;
    let mut lines = Vec::new();
    let mut failed = false;
    let mut step = |name: &str, r: Result<String, String>| match r {
        Ok(s) => lines.push(format!("ok      {name}: {s}")),
        Err(e) => {
            failed = true;
            lines.push(format!("FAILED  {name}: {e}"));
        }
    };
    if url.starts_with("http") {
        step(
            "cors",
            match cors(url) {
                Ok(true) => Ok("a browser may read it".into()),
                Ok(false) => Err("no Access-Control-Allow-Origin header".into()),
                Err(e) => Err(e.to_string()),
            },
        );
    }
    match Reader::open(&*t, &root) {
        Err(e) => step("registry.toml", Err(e)),
        Ok(r) => {
            step(
                "registry.toml",
                Ok(format!("{} under the pinned root", r.file.name)),
            );
            step(
                "yanks.toml",
                r.yanks(&now).map(|y| format!("{} yanks", y.yank.len())),
            );
            match r.catalog(&now) {
                Err(e) => step("catalog.toml", Err(e)),
                Ok(c) => {
                    step(
                        "catalog.toml",
                        Ok(format!("{} packages, until {}", c.package.len(), c.expires)),
                    );
                    for p in &c.package {
                        step(
                            &p.name,
                            r.fetch(&p.name, &p.latest, &p.id)
                                .map(|(_, f)| format!("{} files by id {}", f.len(), &p.id[..12])),
                        );
                    }
                }
            }
        }
    }
    let _ = &mut step;
    Ok(Outcome {
        json: json!({ "ok": !failed, "lines": lines }),
        text: lines.join("\n"),
        exit: u8::from(failed),
    })
}

/// The security headers a page's `_headers` sets for every path.
fn page_headers(page: &Path) -> Vec<String> {
    let text = fs::read_to_string(page.join("_headers")).unwrap_or_default();
    let mut out = Vec::new();
    let mut on = false;
    for line in text.lines() {
        if line.starts_with('#') {
            continue;
        }
        if !line.starts_with(' ') && !line.is_empty() {
            on = line.trim() == "/*";
        } else if on && !line.trim().is_empty() {
            out.push(line.trim().to_owned());
        }
    }
    out
}

fn content_type(path: &Path) -> &'static str {
    match path.extension().and_then(|e| e.to_str()) {
        Some("html") => "text/html",
        Some("css") => "text/css",
        Some("js") => "text/javascript",
        Some("json") => "application/json",
        Some("wasm") => "application/wasm",
        Some("png") => "image/png",
        Some("toml" | "txt") => "text/plain",
        _ => "application/octet-stream",
    }
}

/// The file a request path names, the page's files first and then the
/// registry's. None for anything else, and for any path that climbs out.
fn local_file(dir: &Path, page: &Path, url: &str) -> Option<PathBuf> {
    let path = url.split(['?', '#']).next().unwrap_or("");
    let rel = match path {
        "/" | "/p" => "index.html",
        p => p.strip_prefix('/')?,
    };
    if rel
        .split('/')
        .any(|part| part.is_empty() || part == ".." || part == ".")
        || rel.contains('\\')
    {
        return None;
    }
    [page, dir]
        .iter()
        .map(|root| root.join(rel))
        .find(|p| p.is_file())
}

fn answer(mut stream: std::net::TcpStream, dir: &Path, page: &Path, headers: &[String]) {
    use std::io::{BufRead as _, BufReader, Write as _};
    let Ok(clone) = stream.try_clone() else {
        return;
    };
    let mut reader = BufReader::new(clone);
    let mut first = String::new();
    if reader.read_line(&mut first).is_err() {
        return;
    }
    let mut line = String::new();
    while reader.read_line(&mut line).is_ok_and(|n| n > 0) && line.trim() != "" {
        line.clear();
    }
    let mut parts = first.split_whitespace();
    let (method, url) = (parts.next().unwrap_or(""), parts.next().unwrap_or(""));
    let file = matches!(method, "GET" | "HEAD")
        .then(|| local_file(dir, page, url))
        .flatten();
    let (status, kind, body) = match file.and_then(|p| Some((content_type(&p), fs::read(&p).ok()?)))
    {
        Some((kind, body)) => ("200 OK", kind, body),
        None => ("404 Not Found", "text/plain", Vec::new()),
    };
    let mut head = format!(
        "HTTP/1.1 {status}\r\nContent-Type: {kind}\r\nContent-Length: {}\r\nConnection: close\r\n",
        body.len()
    );
    for h in headers {
        head.push_str(h);
        head.push_str("\r\n");
    }
    head.push_str("\r\n");
    let _ = stream.write_all(head.as_bytes());
    if method == "GET" {
        let _ = stream.write_all(&body);
    }
}

/// Serves a folder registry and a built page from one address, on this
/// machine only, until stopped.
pub fn serve(dir: &Path, page: &Path, port: u16) -> Result<Outcome> {
    if !dir.join("registry.toml").is_file() {
        return Err(anyhow!(
            "{} isn't a registry (no registry.toml)",
            dir.display()
        ));
    }
    if !page.join("index.html").is_file() {
        return Err(anyhow!(
            "{} isn't a built page (no index.html)",
            page.display()
        ));
    }
    let listener = std::net::TcpListener::bind(("127.0.0.1", port))
        .with_context(|| format!("listening on 127.0.0.1:{port}"))?;
    let headers = std::sync::Arc::new(page_headers(page));
    println!("playing at http://127.0.0.1:{port}/p?g=lex00/donkey-kong (ctrl-C stops it)");
    for stream in listener.incoming().flatten() {
        let (dir, page, headers) = (dir.to_owned(), page.to_owned(), headers.clone());
        std::thread::spawn(move || answer(stream, &dir, &page, &headers));
    }
    Ok(Outcome {
        json: json!({}),
        text: String::new(),
        exit: 0,
    })
}

#[cfg(test)]
mod repo_tests {
    use super::*;

    fn git(dir: &Path, args: &[&str]) -> String {
        let o = Command::new("git")
            .arg("-C")
            .arg(dir)
            .args(args)
            .output()
            .unwrap();
        assert!(o.status.success(), "{args:?}: {o:?}");
        String::from_utf8_lossy(&o.stdout).trim().to_owned()
    }

    /// A checkout of a repo with an `origin`, and the origin.
    fn pair() -> (tempfile::TempDir, PathBuf, PathBuf) {
        let d = tempfile::tempdir().unwrap();
        let (origin, work) = (d.path().join("origin.git"), d.path().join("work"));
        git(d.path(), &["init", "-q", "--bare", "origin.git"]);
        git(d.path(), &["clone", "-q", origin.to_str().unwrap(), "work"]);
        git(&work, &["config", "user.email", "t@t"]);
        git(&work, &["config", "user.name", "t"]);
        (d, origin, work)
    }

    fn ok() -> Result<Outcome> {
        Ok(Outcome {
            json: json!({}),
            text: String::new(),
            exit: 0,
        })
    }

    #[test]
    fn a_command_commits_what_it_wrote_and_pushes() {
        let (_d, origin, work) = pair();
        let spec = format!("git:{}", work.display());
        committing(&spec, "first", || {
            open(&spec).write_once("a/b", b"x").unwrap();
            ok()
        })
        .unwrap();
        assert_eq!(git(&origin, &["log", "--format=%s", "HEAD"]), "first");
        assert_eq!(git(&origin, &["show", "HEAD:a/b"]), "x");
    }

    #[test]
    fn a_published_key_is_not_written_again() {
        let (_d, _origin, work) = pair();
        let spec = format!("git:{}", work.display());
        assert!(open(&spec).write_once("k", b"one").unwrap());
        assert!(!open(&spec).write_once("k", b"two").unwrap());
        assert_eq!(fs::read(work.join("k")).unwrap(), b"one");
    }

    #[test]
    fn a_dirty_checkout_is_refused_and_a_failed_command_commits_nothing() {
        let (_d, _origin, work) = pair();
        let spec = format!("git:{}", work.display());
        fs::write(work.join("stray"), "x").unwrap();
        let e = committing(&spec, "m", ok).err().unwrap().to_string();
        assert!(e.contains("not committed"), "{e}");
        git(&work, &["add", "-A"]);
        git(&work, &["commit", "-q", "-m", "base"]);
        let r = committing(&spec, "bad", || {
            open(&spec).write_once("half", b"x").unwrap();
            anyhow::bail!("stopped")
        });
        assert!(r.is_err());
        assert_eq!(git(&work, &["log", "--format=%s"]), "base");
    }

    #[test]
    fn a_key_another_publisher_took_counts_as_taken() {
        let (d, origin, work) = pair();
        git(
            d.path(),
            &["clone", "-q", origin.to_str().unwrap(), "other"],
        );
        let other = d.path().join("other");
        git(&other, &["config", "user.email", "o@o"]);
        git(&other, &["config", "user.name", "o"]);
        let (a, b) = (
            format!("git:{}", work.display()),
            format!("git:{}", other.display()),
        );
        committing(&a, "theirs", || {
            open(&a).write_once("k", b"one").unwrap();
            ok()
        })
        .unwrap();
        committing(&b, "mine", || {
            assert!(!open(&b).write_once("k", b"two").unwrap());
            ok()
        })
        .unwrap();
    }
}
