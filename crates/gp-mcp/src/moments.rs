//! `moments` (docs/point-and-tell.md, "Moments in words"): the game's own
//! moments in a recording (jumps, landings, deaths, lives lost, counter
//! changes, level changes, with what the model does at each), found by
//! what the person said ("the very first jump", "this jump" with a box on
//! a paused one). `gp model refine --lockstep --clip ID --moment KIND:N`
//! refines one.

use std::path::{Path, PathBuf};

use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Value, json};

use gp_interp::moments::{KINDS, Moment, find};

use crate::{Config, Reply};

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct MomentsArgs {
    /// The game, by its folder's name in the games checkout (donkey-kong).
    /// No capture run is needed.
    pub run: String,
    /// The recording the person is watching, by the name the page gives it
    /// (a corpus clip, a free play clip or a store recording).
    #[serde(default)]
    pub recording: Option<String>,
    /// What they said, to find the moment it names: "the very first jump",
    /// "the second coin", "the last death". "this jump" with `frame`
    /// names the jump at or nearest that frame.
    #[serde(default)]
    pub said: Option<String>,
    /// The original's frame they paused on, for "this" moment.
    #[serde(default)]
    pub frame: Option<u64>,
    /// Only this kind: jump, landing, death, life_lost, counter or level.
    #[serde(default)]
    pub kind: Option<String>,
}

/// Where a game's moments are kept by recording: beside the page's own
/// cache of the game (`$GP_HOME/work/serve/GAME/moments/`), which gp
/// serve's moments route writes.
pub fn kept_file(home: &Path, game: &str, recording: &str) -> PathBuf {
    home.join("work")
        .join("serve")
        .join(game)
        .join("moments")
        .join(format!("{recording}.json"))
}

/// A game's moments in a recording, with no capture run: the copy the
/// page keeps, else found by the page's server when gp serve runs for the
/// home, else found now with `gp interp moments` when the server knows the
/// ROM (`gp mcp --rom`).
fn game_moments(config: &Config, game: &str, recording: Option<&str>) -> Result<Value, String> {
    if !crate::asks::game_ok(game) {
        return Err(format!("`{game}` isn't a game's folder name"));
    }
    let recording = recording.ok_or(
        "name the recording the person is watching (recording): a corpus clip, a free play clip or a store recording",
    )?;
    if recording.is_empty()
        || !recording
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
    {
        return Err(format!("`{recording}` isn't a recording name"));
    }
    let file = kept_file(&config.home, game, recording);
    if let Some(v) = std::fs::read(&file)
        .ok()
        .and_then(|b| serde_json::from_slice::<Value>(&b).ok())
    {
        return Ok(v);
    }
    if let Some(v) = from_page(config, game, recording) {
        return v;
    }
    let (Some(rom), Some(games)) = (&config.rom, &config.games) else {
        return Err(format!(
            "no moments are kept for {recording} yet, and the page isn't running to find them: ask the person to open {game}'s page (gp serve), or start gp mcp with --rom"
        ));
    };
    let model = games.join(game);
    let args: Vec<std::ffi::OsString> = vec![
        "interp".into(),
        "moments".into(),
        model.into(),
        "--rom".into(),
        rom.into(),
        "--recording".into(),
        recording.into(),
    ];
    let refs: Vec<&std::ffi::OsStr> = args.iter().map(|a| a.as_os_str()).collect();
    let (code, mut j) = crate::run_gp(config, &refs)?;
    if code != 0 {
        return Err(format!("gp interp moments exited {code}: {j}"));
    }
    j["recording"] = json!(recording);
    if let Some(p) = file.parent() {
        let _ = std::fs::create_dir_all(p);
    }
    let _ = std::fs::write(&file, serde_json::to_vec(&j).unwrap_or_default());
    Ok(j)
}

/// The moments from gp serve's route, when gp serve runs for the home.
fn from_page(config: &Config, game: &str, recording: &str) -> Option<Result<Value, String>> {
    use std::io::{Read as _, Write as _};
    if !crate::asks::page_served(&config.home) {
        return None;
    }
    let page: Value =
        serde_json::from_slice(&std::fs::read(crate::asks::page_file(&config.home)).ok()?).ok()?;
    let port = page["port"].as_u64().filter(|p| *p > 0)?;
    let addr = std::net::SocketAddr::from(([127, 0, 0, 1], u16::try_from(port).ok()?));
    let mut c =
        std::net::TcpStream::connect_timeout(&addr, std::time::Duration::from_secs(2)).ok()?;
    let _ = c.set_read_timeout(Some(std::time::Duration::from_secs(300)));
    write!(
        c,
        "GET /api/capture/{game}/moments?recording={recording} HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nConnection: close\r\n\r\n"
    )
    .ok()?;
    let mut out = Vec::new();
    c.read_to_end(&mut out).ok()?;
    let text = String::from_utf8_lossy(&out);
    let (head, body) = text.split_once("\r\n\r\n")?;
    let v: Value = serde_json::from_str(body.trim()).ok()?;
    Some(if head.starts_with("HTTP/1.1 200") {
        Ok(v)
    } else {
        Err(format!(
            "the page couldn't find the moments: {}",
            v["error"].as_str().unwrap_or(body)
        ))
    })
}

/// A moment back from its JSON, for [`find`].
fn moment_of(v: &Value) -> Option<Moment> {
    let kind = KINDS.iter().find(|k| v["kind"] == **k)?;
    Some(Moment {
        kind,
        n: usize::try_from(v["n"].as_u64()?).ok()?,
        part: v["part"].as_str().unwrap_or_default().to_owned(),
        from: v["from"].as_u64()?,
        to: v["to"].as_u64()?,
        peak: v["peak"].as_u64(),
        height: None,
        before: v["before"].as_str().map(str::to_owned),
        after: v["after"].as_str().map(str::to_owned),
    })
}

/// A moment in a line for the agent: "jump 1 at frames 197 to 217 (3.3 s):
/// the original goes up 24 px; the model 23.1 px, on the same frame".
fn line(v: &Value) -> String {
    let what = match (v["kind"].as_str(), v["height"].as_f64()) {
        (Some("jump"), Some(h)) => format!("the original goes up {h} px"),
        (Some("counter" | "level"), _) => format!(
            "{} goes from {} to {}",
            v["part"].as_str().unwrap_or(""),
            v["before"].as_str().unwrap_or("?"),
            v["after"].as_str().unwrap_or("?")
        ),
        _ => "in the original".to_owned(),
    };
    let model = match &v["model"] {
        Value::Null => "the model doesn't".to_owned(),
        m => {
            let d = m["offset"].as_i64().unwrap_or(0);
            let when = match d {
                0 => "on the same frame".to_owned(),
                d if d > 0 => format!("{d} frames later"),
                d => format!("{} frames earlier", -d),
            };
            match m["height"].as_f64() {
                Some(h) => format!("the model {h} px, {when}"),
                None => format!("the model too, {when}"),
            }
        }
    };
    format!(
        "{} {} at frames {} to {} ({:.1} s): {what}; {model}",
        v["kind"].as_str().unwrap_or(""),
        v["n"],
        v["from"],
        v["to"],
        v["seconds_from"].as_f64().unwrap_or(0.0)
    )
}

pub fn moments(config: &Config, a: MomentsArgs) -> Result<Reply, String> {
    let j = game_moments(config, &a.run, a.recording.as_deref())?;
    let recording = j["recording"].as_str().unwrap_or_default().to_owned();
    let all: Vec<Value> = j["moments"].as_array().cloned().unwrap_or_default();
    let list: Vec<&Value> = all
        .iter()
        .filter(|m| a.kind.as_deref().is_none_or(|k| m["kind"] == k))
        .collect();
    // What they said: a moment by its words, or "this" one at the frame.
    let found = a.said.as_deref().and_then(|said| {
        let ms: Vec<Moment> = list.iter().filter_map(|v| moment_of(v)).collect();
        let this = said
            .to_lowercase()
            .split_whitespace()
            .any(|w| w == "this" || w == "here");
        let pick = match (this, a.frame) {
            (true, Some(f)) => {
                let named = find(&ms, said).map(|m| m.kind);
                ms.iter()
                    .filter(|m| named.is_none_or(|k| m.kind == k))
                    .min_by_key(|m| {
                        if f >= m.from && f <= m.to {
                            0
                        } else {
                            m.from.abs_diff(f).min(m.to.abs_diff(f))
                        }
                    })
            }
            _ => find(&ms, said),
        }?;
        list.iter()
            .find(|v| v["kind"] == pick.kind && v["n"] == pick.n as u64)
            .map(|v| (*v).clone())
    });
    let lines: Vec<String> = list.iter().take(40).map(|v| line(v)).collect();
    Ok(Reply::ok(json!({
        "run": a.run,
        "recording": recording,
        "kinds": j["kinds"],
        "count": list.len(),
        "found": found.as_ref().map(|v| json!({"kind": v["kind"], "n": v["n"], "line": line(v), "moment": v})),
        "lines": lines,
        "more": list.len().saturating_sub(40),
    })))
}
