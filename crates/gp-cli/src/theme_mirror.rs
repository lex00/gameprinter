//! Package mirrors (docs/theme-packages.md, "Mirrors"): plain object storage that
//! holds the local store's key tree, read by anyone over HTTP and written
//! by gp through one S3-compatible client. Cloudflare R2 is the first,
//! Backblaze B2 an optional second (docs/research/package-hosting.md), and
//! a folder a static host serves works as one too.
//!
//! The list is `mirrors.toml` in the repo, with `$GP_HOME/mirrors.toml`
//! first (a person's own, or a test's mock). Keys are never in either:
//! a mirror's write key is `GP_MIRROR_KEY_<NAME>` (`ID:SECRET`) or the file
//! `$GP_HOME/secrets/mirror-<name>`, read only by its owner. The client is
//! curl with its AWS Signature Version 4 (`--aws-sigv4`), given the key on
//! its standard input, never in its arguments, and every message passes
//! through [`redact`].

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use anyhow::{Context, Result, bail};
use serde::Deserialize;

/// The mirrors file's name, in the repo and in `$GP_HOME`.
pub const MIRRORS_FILE: &str = "mirrors.toml";

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Mirror {
    pub name: String,
    /// The base URL anyone reads keys under (`<read>/blobs/<hash>`),
    /// `http(s)://` or `file://`.
    #[serde(default)]
    pub read: Option<String>,
    /// The S3 endpoint gp writes through, path style:
    /// `<endpoint>/<bucket>/<key>`.
    #[serde(default)]
    pub endpoint: Option<String>,
    #[serde(default)]
    pub bucket: Option<String>,
    /// The signing region (`auto` for R2, the bucket's for B2).
    #[serde(default = "auto")]
    pub region: String,
    /// A folder gp writes keys into, for a mirror a static host serves.
    #[serde(default)]
    pub dir: Option<PathBuf>,
}

fn auto() -> String {
    "auto".into()
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct File {
    #[allow(dead_code)]
    format: String,
    #[serde(default)]
    mirror: Vec<Mirror>,
}

/// The mirrors to use, in order: `$GP_HOME/mirrors.toml`'s, then the
/// repo's that the first doesn't name.
pub fn configured(home: &Path, repo: Option<&Path>) -> Result<Vec<Mirror>> {
    let mut out: Vec<Mirror> = Vec::new();
    for f in [
        Some(home.join(MIRRORS_FILE)),
        repo.map(|r| r.join(MIRRORS_FILE)),
    ]
    .into_iter()
    .flatten()
    {
        let Ok(text) = std::fs::read_to_string(&f) else {
            continue;
        };
        let parsed: File = toml::from_str(&text).with_context(|| format!("{}", f.display()))?;
        for m in parsed.mirror {
            if m.name.is_empty()
                || !m
                    .name
                    .chars()
                    .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
            {
                bail!(
                    "{}: `{}` isn't a mirror name (lowercase letters, digits and -)",
                    f.display(),
                    m.name
                );
            }
            if !out.iter().any(|x| x.name == m.name) {
                out.push(m);
            }
        }
    }
    Ok(out)
}

impl Mirror {
    pub fn writable(&self) -> bool {
        self.dir.is_some() || (self.endpoint.is_some() && self.bucket.is_some())
    }

    fn key_env(&self) -> String {
        format!(
            "GP_MIRROR_KEY_{}",
            self.name
                .chars()
                .map(|c| if c.is_ascii_alphanumeric() {
                    c.to_ascii_uppercase()
                } else {
                    '_'
                })
                .collect::<String>()
        )
    }

    /// The write key, `ID:SECRET`.
    fn secret(&self, home: &Path) -> Result<String> {
        if let Ok(k) = std::env::var(self.key_env())
            && !k.trim().is_empty()
        {
            return Ok(k.trim().to_owned());
        }
        let f = home.join("secrets").join(format!("mirror-{}", self.name));
        if let Ok(meta) = std::fs::metadata(&f) {
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                if meta.permissions().mode() & 0o077 != 0 {
                    bail!(
                        "{} can be read by others than its owner; chmod 600 it",
                        f.display()
                    );
                }
            }
            let _ = meta;
            return Ok(std::fs::read_to_string(&f)?.trim().to_owned());
        }
        bail!(
            "no write key for mirror `{}`: set {} to ID:SECRET, or write it to {}",
            self.name,
            self.key_env(),
            f.display()
        )
    }

    /// Writes a key unless it is there. Returns whether it was written.
    /// `replace` writes it either way (a game's index copy, the one key
    /// that changes).
    pub fn put(&self, home: &Path, key: &str, bytes: &[u8], replace: bool) -> Result<bool> {
        if let Some(d) = &self.dir {
            let s = gp_package::Store::at(d);
            return if replace {
                s.replace(key, bytes).map_err(anyhow::Error::msg)?;
                Ok(true)
            } else {
                s.put(key, bytes).map_err(anyhow::Error::msg)
            };
        }
        let (Some(endpoint), Some(bucket)) = (&self.endpoint, &self.bucket) else {
            bail!(
                "mirror `{}` has no endpoint and bucket to write to",
                self.name
            );
        };
        let secret = self.secret(home)?;
        let url = format!("{}/{bucket}/{key}", endpoint.trim_end_matches('/'));
        if !replace && self.s3(&secret, "HEAD", &url, None)?.0 == 200 {
            return Ok(false);
        }
        let (code, body) = self.s3(&secret, "PUT", &url, Some(bytes))?;
        if !(200..300).contains(&code) {
            bail!(
                "mirror `{}`: PUT {key} answered {code}: {}",
                self.name,
                redact(&body, &secret)
            );
        }
        Ok(true)
    }

    /// One signed S3 request through curl: the key on its standard input.
    fn s3(
        &self,
        secret: &str,
        method: &str,
        url: &str,
        body: Option<&[u8]>,
    ) -> Result<(u16, String)> {
        let tmp = match body {
            Some(b) => {
                let f = tempfile::NamedTempFile::new()?;
                std::fs::write(f.path(), b)?;
                Some(f)
            }
            None => None,
        };
        let mut config = format!(
            "user = \"{}\"\n",
            secret.replace('\\', "\\\\").replace('"', "\\\"")
        );
        if let Some(f) = &tmp {
            config.push_str(&format!("upload-file = \"{}\"\n", f.path().display()));
        }
        let mut args = vec![
            "-sS".to_owned(),
            "--max-time".into(),
            "300".into(),
            "--connect-timeout".into(),
            "20".into(),
            "--aws-sigv4".into(),
            format!("aws:amz:{}:s3", self.region),
            "--config".into(),
            "-".into(),
            "-o".into(),
            "-".into(),
            "-w".into(),
            "\n%{http_code}".into(),
        ];
        if method == "HEAD" {
            args.push("-I".into());
        } else {
            args.push("-X".into());
            args.push(method.into());
        }
        args.push(url.into());
        let mut child = Command::new("curl")
            .args(&args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .context("running curl")?;
        if let Some(mut stdin) = child.stdin.take() {
            let _ = stdin.write_all(config.as_bytes());
        }
        let out = child.wait_with_output()?;
        let text = String::from_utf8_lossy(&out.stdout).into_owned();
        let (body, code) = text.rsplit_once('\n').unwrap_or(("", text.as_str()));
        let code: u16 = code.trim().parse().unwrap_or(0);
        if code == 0 {
            bail!(
                "mirror `{}`: {method} {url}: {}",
                self.name,
                redact(String::from_utf8_lossy(&out.stderr).trim(), secret)
            );
        }
        Ok((code, body.to_owned()))
    }

    /// Reads a key over the mirror's read URL, or from its folder. None
    /// when it isn't there.
    pub fn get(&self, key: &str) -> Result<Option<Vec<u8>>> {
        if let Some(read) = &self.read {
            return fetch(&format!("{}/{key}", read.trim_end_matches('/')));
        }
        if let Some(d) = &self.dir {
            let p = d.join(key);
            return Ok(p.is_file().then(|| std::fs::read(&p)).transpose()?);
        }
        Ok(None)
    }
}

/// GET a URL with curl: the body, or None on a 404.
pub fn fetch(url: &str) -> Result<Option<Vec<u8>>> {
    let tmp = tempfile::NamedTempFile::new()?;
    let out = Command::new("curl")
        .args(["-sS", "--max-time", "300", "--connect-timeout", "20", "-o"])
        .arg(tmp.path())
        .args(["-w", "%{http_code}"])
        .arg(url)
        .stdin(Stdio::null())
        .output()
        .context("running curl")?;
    let code = String::from_utf8_lossy(&out.stdout).trim().to_owned();
    // file:// answers 000 with exit 0 when read, exit 37 when missing.
    if url.starts_with("file://") {
        return Ok(out
            .status
            .success()
            .then(|| std::fs::read(tmp.path()))
            .transpose()?);
    }
    match code.as_str() {
        "200" => Ok(Some(std::fs::read(tmp.path())?)),
        "404" | "403" => Ok(None),
        _ => bail!(
            "GET {url}: {} {}",
            code,
            String::from_utf8_lossy(&out.stderr).trim()
        ),
    }
}

/// A message with the key and its secret half taken out.
pub fn redact(s: &str, secret: &str) -> String {
    let mut out = s.to_owned();
    for part in std::iter::once(secret).chain(secret.split(':')) {
        if part.len() >= 4 {
            out = out.replace(part, "[key]");
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mirrors_read_home_first_and_a_key_is_redacted() {
        let d = tempfile::tempdir().unwrap();
        let (home, repo) = (d.path().join("h"), d.path().join("r"));
        std::fs::create_dir_all(&home).unwrap();
        std::fs::create_dir_all(&repo).unwrap();
        std::fs::write(
            home.join(MIRRORS_FILE),
            "format = \"0.1.0\"\n[[mirror]]\nname = \"r2\"\nread = \"http://127.0.0.1:1\"\n",
        )
        .unwrap();
        std::fs::write(
            repo.join(MIRRORS_FILE),
            "format = \"0.1.0\"\n[[mirror]]\nname = \"r2\"\nread = \"https://example.invalid\"\n[[mirror]]\nname = \"b2\"\ndir = \"/tmp/x\"\n",
        )
        .unwrap();
        let m = configured(&home, Some(&repo)).unwrap();
        assert_eq!(m.len(), 2);
        assert_eq!(m[0].read.as_deref(), Some("http://127.0.0.1:1"));
        assert!(m[1].writable() && !m[0].writable());
        assert_eq!(m[1].key_env(), "GP_MIRROR_KEY_B2");
        assert_eq!(redact("id ABCD:sekrit1 x", "ABCD:sekrit1"), "id [key] x");
        assert_eq!(
            redact("Credential=ABCD/", "ABCD:sekrit1"),
            "Credential=[key]/"
        );
    }
}
