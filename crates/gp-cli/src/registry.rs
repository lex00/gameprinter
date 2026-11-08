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

/// A folder, `s3://BUCKET` or an https address.
pub fn open(spec: &str) -> Box<dyn Target> {
    if let Some(bucket) = spec.strip_prefix("s3://") {
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
