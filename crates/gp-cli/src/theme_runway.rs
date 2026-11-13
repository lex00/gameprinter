//! Runway's API for `gp theme generate` (docs/theme-kit.md, "Generating
//! with Runway"): the key, the credits each call costs, and the calls
//! themselves, made with `curl` so gp links no TLS stack.
//!
//! The API, from https://docs.dev.runwayml.com/ and the types of the
//! `@runwayml/sdk` package (resources/text-to-image.d.ts, tasks.d.ts), read
//! 2026-11-03: `POST /v1/text_to_image` starts a task and returns its `id`;
//! `GET /v1/tasks/{id}` gives its `status` (PENDING, THROTTLED, RUNNING,
//! SUCCEEDED, FAILED, CANCELLED), `output` (URLs that expire in 24 to 48
//! hours) on success, `failure` and `failureCode` on failure, and
//! `cost.credits` once it ends. A failed task's credits are refunded except
//! for `SAFETY.INPUT.*` (errors/task-failures). HTTP 429, 502, 503 and 504
//! are retried with backoff and jitter; 400, 401, 404 and 405 are not
//! (errors/errors). A credit is $0.01 (guides/pricing).
//!
//! The key never goes on a command line, into a file gp writes, or into a
//! message: curl reads its header from standard input, and every message
//! from here passes [`Key::redact`].

use std::fmt;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use anyhow::{Result, bail};
use serde_json::Value;

/// The API's address. `--api` replaces it (the tests' local mock).
pub const API: &str = "https://api.dev.runwayml.com";
/// The API version every request names.
pub const VERSION: &str = "2024-11-06";
/// The keychain item `security` finds the key under.
pub const KEYCHAIN_SERVICE: &str = "game-printer-runway";
/// Dollars a credit.
pub const DOLLARS_PER_CREDIT: f64 = 0.01;

/// The API key. It has no Display, and its Debug prints no part of it.
#[derive(Clone)]
pub struct Key {
    secret: String,
    /// Where it came from, for messages: "RUNWAY_API_KEY", "the keychain",
    /// or the secrets file's path.
    pub source: String,
}

impl fmt::Debug for Key {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Key(from {})", self.source)
    }
}

impl Key {
    /// A key from its text, checked to be one line of printable ASCII
    /// with no quote or backslash, so it can sit in curl's config.
    pub fn new(secret: &str, source: &str) -> Result<Self> {
        let s = secret.trim();
        if s.len() < 8 {
            bail!("the Runway key from {source} is too short to be a key");
        }
        if !s
            .bytes()
            .all(|b| b.is_ascii_graphic() && b != b'"' && b != b'\\')
        {
            bail!(
                "the Runway key from {source} holds a character a key doesn't (a space, a quote, a backslash or a control character)"
            );
        }
        Ok(Self {
            secret: s.to_owned(),
            source: source.to_owned(),
        })
    }

    /// `text` with the key, and any bearer token, replaced by `[key]`.
    pub fn redact(&self, text: &str) -> String {
        redact(&text.replace(&self.secret, "[key]"))
    }
}

/// `text` with every bearer token replaced by `[key]`, for messages
/// written where no key is at hand.
pub fn redact(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(i) = rest.find("Bearer ") {
        out.push_str(&rest[..i + 7]);
        rest = &rest[i + 7..];
        let end = rest
            .find(|c: char| c.is_whitespace() || c == '"' || c == '\'' || c == ',')
            .unwrap_or(rest.len());
        if end > 0 {
            out.push_str("[key]");
        }
        rest = &rest[end..];
    }
    out.push_str(rest);
    out
}

/// The key, from the first of: `RUNWAY_API_KEY`, the macOS keychain
/// (`security find-generic-password -s game-printer-runway -w`), and
/// `$GP_HOME/secrets/runway`, which must be readable by its owner only.
/// `GP_RUNWAY_KEYCHAIN=0` skips the keychain.
pub fn load_key(home: &Path) -> Result<Key> {
    if let Ok(k) = std::env::var("RUNWAY_API_KEY")
        && !k.trim().is_empty()
    {
        return Key::new(&k, "RUNWAY_API_KEY");
    }
    if cfg!(target_os = "macos") && std::env::var("GP_RUNWAY_KEYCHAIN").as_deref() != Ok("0") {
        let out = Command::new("security")
            .args(["find-generic-password", "-s", KEYCHAIN_SERVICE, "-w"])
            .stdin(Stdio::null())
            .stderr(Stdio::null())
            .output();
        if let Ok(o) = out
            && o.status.success()
        {
            let k = String::from_utf8_lossy(&o.stdout).trim().to_owned();
            if !k.is_empty() {
                return Key::new(&k, "the keychain");
            }
        }
    }
    let file = secrets_file(home);
    if file.is_file() {
        return key_file(&file);
    }
    bail!(
        "no Runway API key: set RUNWAY_API_KEY, or add it to the keychain (security add-generic-password -s {KEYCHAIN_SERVICE} -a \"$USER\" -w), or write it to {} with mode 600 (docs/theme-kit.md, \"The key\")",
        file.display()
    )
}

/// The key in a secrets file, refused when others can read it.
fn key_file(file: &Path) -> Result<Key> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(file)?.permissions().mode();
        if mode & 0o077 != 0 {
            bail!(
                "{} can be read by others (mode {:o}); run `chmod 600 {}`",
                file.display(),
                mode & 0o777,
                file.display()
            );
        }
    }
    let k = std::fs::read_to_string(file)?;
    Key::new(&k, &file.display().to_string())
}

/// `$GP_HOME/secrets/runway`.
pub fn secrets_file(home: &Path) -> PathBuf {
    home.join("secrets").join("runway")
}

/// What a call costs in credits, from the pricing guide
/// (https://docs.dev.runwayml.com/guides/pricing/, 2026-11-03).
/// `gen4_image` is 5 at its 720p ratios and 8 at the others;
/// `gen4_image_turbo` is 2; `gpt_image_2` is by quality, low 1, medium 5,
/// high 20, at 1K and 2K, and 2, 11 and 41 at 4K or `auto`, times its
/// outputs. A ratio whose longer side passes 2560 is taken as 4K.
pub fn credits(model: &str, ratio: &str, quality: Option<&str>, outputs: u32) -> Result<u32> {
    let per = match model {
        "gen4_image" => {
            if GEN4_720P.contains(&ratio) {
                5
            } else {
                8
            }
        }
        "gen4_image_turbo" => 2,
        "gpt_image_2" => {
            let big = ratio == "auto" || longer_side(ratio) > 2560;
            match (quality.unwrap_or("high"), big) {
                ("low", false) => 1,
                ("low", true) => 2,
                ("medium", false) => 5,
                ("medium", true) => 11,
                ("high" | "auto", false) => 20,
                ("high" | "auto", true) => 41,
                (q, _) => bail!("gpt_image_2 has no quality `{q}` (low, medium, high, auto)"),
            }
        }
        m => bail!("no price for model `{m}` (gen4_image, gen4_image_turbo, gpt_image_2)"),
    };
    Ok(per * outputs.max(1))
}

fn longer_side(ratio: &str) -> u32 {
    ratio
        .split(':')
        .filter_map(|n| n.parse::<u32>().ok())
        .max()
        .unwrap_or(0)
}

const GEN4_720P: [&str; 6] = [
    "1280:720", "720:1280", "720:720", "960:720", "720:960", "1680:720",
];

/// Each model's ratios (the SDK's literals), prompt limit and reference
/// limit.
pub fn ratios(model: &str) -> &'static [&'static str] {
    match model {
        "gen4_image" | "gen4_image_turbo" => &[
            "1024:1024",
            "1080:1080",
            "1168:880",
            "1360:768",
            "1440:1080",
            "1080:1440",
            "1808:768",
            "1920:1080",
            "1080:1920",
            "2112:912",
            "1280:720",
            "720:1280",
            "720:720",
            "960:720",
            "720:960",
            "1680:720",
        ],
        "gpt_image_2" => &[
            "2048:880",
            "1920:1088",
            "1920:1280",
            "1920:1440",
            "1920:1536",
            "1920:1920",
            "1536:1920",
            "1440:1920",
            "1280:1920",
            "1088:1920",
            "2912:1248",
            "2560:1440",
            "2560:1712",
            "2560:1920",
            "2560:2048",
            "2560:2560",
            "2048:2560",
            "1920:2560",
            "1712:2560",
            "1440:2560",
            "3840:1648",
            "3840:2160",
            "3504:2336",
            "3264:2448",
            "3200:2560",
            "2880:2880",
            "2560:3200",
            "2448:3264",
            "2336:3504",
            "2160:3840",
            "auto",
        ],
        _ => &[],
    }
}

/// The longest prompt a model takes (UTF-16 units for Gen-4).
pub fn prompt_limit(model: &str) -> usize {
    match model {
        "gpt_image_2" => 32_000,
        _ => 1000,
    }
}

/// The most reference images a model takes, and the fewest it needs.
pub fn reference_limits(model: &str) -> (usize, usize) {
    match model {
        "gpt_image_2" => (0, 16),
        "gen4_image_turbo" => (1, 3),
        _ => (0, 3),
    }
}

/// The model's ratio nearest an aspect (width over height), within the
/// sizes that cost the least.
pub fn nearest_ratio(model: &str, w: u32, h: u32) -> String {
    let want = f64::from(w.max(1)) / f64::from(h.max(1));
    let mut best: Option<(f64, u32, &str)> = None;
    for r in ratios(model) {
        let mut it = r.split(':').filter_map(|n| n.parse::<u32>().ok());
        let (Some(a), Some(b)) = (it.next(), it.next()) else {
            continue;
        };
        let d = (f64::from(a) / f64::from(b) / want).ln().abs();
        let side = a.max(b);
        // Within 2% of the best aspect, the smaller size wins.
        let better = match best {
            None => true,
            Some((bd, bs, _)) => d + 0.02 < bd || (d < bd + 0.02 && side < bs),
        };
        if better {
            best = Some((d, side, r));
        }
    }
    best.map(|b| b.2.to_owned())
        .unwrap_or_else(|| "1024:1024".into())
}

/// A tag the API accepts in `referenceImages`: a letter, then letters,
/// digits or underscores, 3 to 16 in all.
pub fn valid_tag(tag: &str) -> bool {
    let b = tag.as_bytes();
    (3..=16).contains(&b.len())
        && b[0].is_ascii_alphabetic()
        && b.iter().all(|c| c.is_ascii_alphanumeric() || *c == b'_')
}

/// An HTTP reply: the status and the body.
#[derive(Debug, Clone)]
pub struct Reply {
    pub status: u16,
    pub body: String,
}

impl Reply {
    pub fn json(&self) -> Value {
        serde_json::from_str(&self.body).unwrap_or(Value::Null)
    }
    /// 429 and the 5xx the API names as load shedding: try again later.
    pub fn retryable(&self) -> bool {
        matches!(self.status, 0 | 429 | 500 | 502 | 503 | 504)
    }
}

/// The client: the API's address and the key.
pub struct Client {
    pub api: String,
    key: Key,
}

impl Client {
    pub fn new(api: &str, key: Key) -> Self {
        Self {
            api: api.trim_end_matches('/').to_owned(),
            key,
        }
    }

    pub fn redact(&self, text: &str) -> String {
        self.key.redact(text)
    }

    /// One request. `body` is a file holding the JSON to send. The key
    /// goes to curl on its standard input, never in its arguments.
    pub fn request(&self, method: &str, path: &str, body: Option<&Path>) -> Result<Reply> {
        let url = format!("{}{}", self.api, path);
        let mut config = format!(
            "header = \"Authorization: Bearer {}\"\nheader = \"X-Runway-Version: {VERSION}\"\n",
            self.key.secret
        );
        if let Some(b) = body {
            config.push_str("header = \"Content-Type: application/json\"\n");
            config.push_str(&format!("data-binary = \"@{}\"\n", curl_quote(b)));
        }
        let mut child = Command::new("curl")
            .args([
                "-sS",
                "--max-time",
                "120",
                "--connect-timeout",
                "20",
                "-X",
                method,
                "--config",
                "-",
                "-w",
                "\n%{http_code}",
                &url,
            ])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| anyhow::anyhow!("running curl: {e}"))?;
        if let Some(mut stdin) = child.stdin.take() {
            let _ = stdin.write_all(config.as_bytes());
        }
        let out = child.wait_with_output()?;
        let text = String::from_utf8_lossy(&out.stdout).into_owned();
        let (body, code) = match text.rfind('\n') {
            Some(i) => (text[..i].to_owned(), text[i + 1..].trim().to_owned()),
            None => (String::new(), text.trim().to_owned()),
        };
        let status = code.parse::<u16>().unwrap_or(0);
        if status == 0 {
            // curl couldn't reach the API: a retryable reply with its words.
            let err = String::from_utf8_lossy(&out.stderr).trim().to_owned();
            return Ok(Reply {
                status: 0,
                body: self.redact(&err),
            });
        }
        Ok(Reply {
            status,
            body: self.redact(&body),
        })
    }
}

/// A path as curl's config quotes it.
fn curl_quote(p: &Path) -> String {
    p.display()
        .to_string()
        .replace('\\', "\\\\")
        .replace('"', "\\\"")
}

/// Downloads an output URL to a file, with no key (the URLs are signed).
pub fn download(url: &str, to: &Path) -> Result<()> {
    let out = Command::new("curl")
        .args(["-sS", "-f", "--max-time", "300", "-o"])
        .arg(to)
        .arg(url)
        .stdin(Stdio::null())
        .output()?;
    if !out.status.success() {
        bail!(
            "downloading a result: {}",
            redact(String::from_utf8_lossy(&out.stderr).trim())
        );
    }
    Ok(())
}

/// Whether a failed task may be tried again, by its failure code
/// (errors/task-failures): a safety refusal or an unusable input is
/// final; an internal or third-party failure is not.
pub fn failure_retryable(code: Option<&str>) -> bool {
    match code {
        None => true,
        Some(c) => {
            c == "INTERNAL"
                || c.starts_with("INTERNAL.")
                || c == "INPUT_PREPROCESSING.INTERNAL"
                || c == "THIRD_PARTY.UNAVAILABLE"
        }
    }
}

/// Whether a failure code keeps its credits (only `SAFETY.INPUT.*`).
pub fn failure_charged(code: Option<&str>) -> bool {
    code.is_some_and(|c| c.starts_with("SAFETY.INPUT"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn credits_follow_the_pricing_guide() {
        assert_eq!(credits("gen4_image", "1920:1080", None, 1).unwrap(), 8);
        assert_eq!(credits("gen4_image", "1280:720", None, 1).unwrap(), 5);
        assert_eq!(
            credits("gen4_image_turbo", "1024:1024", None, 1).unwrap(),
            2
        );
        assert_eq!(
            credits("gpt_image_2", "1920:1920", Some("high"), 1).unwrap(),
            20
        );
        assert_eq!(
            credits("gpt_image_2", "2880:2880", Some("high"), 1).unwrap(),
            41
        );
        assert_eq!(credits("gpt_image_2", "auto", Some("low"), 1).unwrap(), 2);
        assert_eq!(
            credits("gpt_image_2", "1920:1920", Some("medium"), 2).unwrap(),
            10
        );
        assert!(credits("gen4_aleph", "1024:1024", None, 1).is_err());
        assert!(credits("gpt_image_2", "1920:1920", Some("ultra"), 1).is_err());
    }

    #[test]
    fn a_key_never_shows() {
        let k = Key::new("key_abcdef0123456789", "test").unwrap();
        assert_eq!(format!("{k:?}"), "Key(from test)");
        assert_eq!(
            k.redact("401: key_abcdef0123456789 is not valid"),
            "401: [key] is not valid"
        );
        assert_eq!(
            redact("header Authorization: Bearer sk-anything, then"),
            "header Authorization: Bearer [key], then"
        );
        assert!(Key::new("has space inside", "t").is_err());
        assert!(Key::new("quote\"inside1", "t").is_err());
        assert!(Key::new("short", "t").is_err());
    }

    #[test]
    fn a_secrets_file_others_can_read_is_refused() {
        let tmp = tempfile::tempdir().unwrap();
        let f = secrets_file(tmp.path());
        std::fs::create_dir_all(f.parent().unwrap()).unwrap();
        std::fs::write(&f, "key_from_file_0123\n").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&f, std::fs::Permissions::from_mode(0o644)).unwrap();
            let e = key_file(&f).unwrap_err().to_string();
            assert!(e.contains("chmod 600"), "{e}");
            std::fs::set_permissions(&f, std::fs::Permissions::from_mode(0o600)).unwrap();
            let k = key_file(&f).unwrap();
            assert!(k.source.ends_with("runway"));
            assert!(!format!("{k:?}").contains("key_from_file"));
        }
    }

    #[test]
    fn ratios_and_tags() {
        assert_eq!(nearest_ratio("gen4_image", 256, 240), "720:720");
        assert_eq!(nearest_ratio("gen4_image", 1920, 1080), "1280:720");
        assert_eq!(nearest_ratio("gpt_image_2", 1000, 1000), "1920:1920");
        assert!(valid_tag("style"));
        assert!(valid_tag("hero_2"));
        assert!(!valid_tag("ab"));
        assert!(!valid_tag("2hero"));
        assert!(!valid_tag("sheet-entity"));
        assert!(failure_retryable(Some("INTERNAL.BAD_OUTPUT.01")));
        assert!(!failure_retryable(Some("SAFETY.INPUT.TEXT")));
        assert!(failure_charged(Some("SAFETY.INPUT.TEXT")));
        assert!(!failure_charged(Some("INTERNAL")));
    }
}
