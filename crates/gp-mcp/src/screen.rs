//! What the game's screen page shows (mechanic/screen.html, docs/mcp.md,
//! "The screen"): the original and the model side by side, one over the
//! other, or their difference, live or at a frame of a clip, in the game's
//! own pictures or in one of the person's themes. The agent asks for a view
//! with `screen_show` (or `gp mechanic screen`) so the person sees what it
//! talks about; the page applies each ask while it is open and reports what
//! it shows, which is the person's choice once they pick something
//! themselves.
//!
//! Two files under `$GP_HOME/work/serve/GAME/`, each written whole:
//! `screen-ask.json`, the agent's last ask (only the parts it set, and a
//! number that goes up with each ask), and `screen-shown.json`, what the
//! page reported last, with the ask it had applied and when. The page polls
//! the server for both, so no server needs to run for the agent to ask.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Map, Value, json};

use crate::{Config, Reply};

/// The views the page has.
pub const VIEWS: &[&str] = &["side", "top", "diff", "solo"];

/// The pictures word for the game's own pictures (no theme).
pub const OWN_PICTURES: &str = "game";

/// A page that reported within this long is open.
const OPEN_WITHIN_MS: u64 = 12_000;

/// How long an ask waits for an open page to show it.
const APPLY_WAIT: Duration = Duration::from_secs(4);

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// A game's, a mechanic's or a theme's name: letters, digits, - and _.
fn name_ok(n: &str) -> bool {
    !n.is_empty()
        && n.len() <= 80
        && !n.starts_with('-')
        && n.bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
}

fn dir(home: &Path, game: &str) -> Result<PathBuf, String> {
    if !name_ok(game) {
        return Err(format!(
            "a game's name is letters, digits, - and _ (got {game:?})"
        ));
    }
    Ok(home.join("work").join("serve").join(game))
}

fn read(f: &Path) -> Value {
    std::fs::read(f)
        .ok()
        .and_then(|b| serde_json::from_slice(&b).ok())
        .unwrap_or(Value::Null)
}

/// Writes `v` to `f` whole: to a file beside it, then moved over it, so a
/// reader never sees half of it.
fn write(f: &Path, v: &Value) -> Result<(), String> {
    let parent = f.parent().ok_or("no folder")?;
    std::fs::create_dir_all(parent).map_err(|e| format!("{}: {e}", parent.display()))?;
    let tmp = f.with_extension(format!("json.{}", std::process::id()));
    std::fs::write(&tmp, serde_json::to_vec_pretty(v).unwrap_or_default())
        .and_then(|()| std::fs::rename(&tmp, f))
        .map_err(|e| format!("{}: {e}", f.display()))
}

/// The agent's last ask, or null.
pub fn ask_of(home: &Path, game: &str) -> Result<Value, String> {
    Ok(read(&dir(home, game)?.join("screen-ask.json")))
}

/// What the page reported last, or null.
pub fn shown_of(home: &Path, game: &str) -> Result<Value, String> {
    Ok(read(&dir(home, game)?.join("screen-shown.json")))
}

/// How long the work file counts as the agent working after its last
/// write: a refine writes on every window and trial, far more often.
pub const WORK_FRESH_MS: u64 = 120_000;

/// What the agent's work is on now (`screen-work.json`): the corpus clip
/// a refine works on, its start on the play from power-on, its length,
/// the original's frames of the window it is fitting and its last line in
/// words. The page follows it while the person isn't playing. Each write
/// keeps the clip's last window when the new event has none.
pub fn work(home: &Path, game: &str, on: &Value) -> Result<(), String> {
    let f = dir(home, game)?.join("screen-work.json");
    let old = read(&f);
    let mut v = match on {
        Value::Object(m) => Value::Object(m.clone()),
        _ => return Err("the work is an object".into()),
    };
    if old["clip"] == v["clip"] {
        for k in ["from", "to"] {
            if v.get(k).is_none() && !old[k].is_null() {
                v[k] = old[k].clone();
            }
        }
    }
    v["at_ms"] = json!(now_ms());
    write(&f, &v)
}

/// The work as last written, with `live` true while it is fresh and not
/// ended; null when nothing was written.
pub fn work_of(home: &Path, game: &str) -> Result<Value, String> {
    let mut v = read(&dir(home, game)?.join("screen-work.json"));
    if v.is_object() {
        let fresh = v["at_ms"]
            .as_u64()
            .is_some_and(|t| now_ms().saturating_sub(t) < WORK_FRESH_MS);
        v["live"] = json!(fresh && v["ended"] != json!(true));
    }
    Ok(v)
}

/// What the page shows now, as the page reports it (the server's route
/// for it): kept with the time it came.
pub fn report(home: &Path, game: &str, shown: &Value) -> Result<Value, String> {
    let mut v = match shown {
        Value::Object(m) => Value::Object(m.clone()),
        _ => return Err("the page reports an object".into()),
    };
    v["at_ms"] = json!(now_ms());
    write(&dir(home, game)?.join("screen-shown.json"), &v)?;
    Ok(v)
}

/// The mechanic open on the game's page (`$GP_HOME/mechanics/GAME/current`).
fn open_mechanic(home: &Path, game: &str) -> Option<String> {
    let t = std::fs::read_to_string(home.join("mechanics").join(game).join("current")).ok()?;
    let t = t.trim();
    name_ok(t).then(|| t.to_owned())
}

/// What a call asks the screen to show. Every part is optional; the parts
/// left out stay as the page has them.
#[derive(Debug, Default, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ShowArgs {
    /// The game: its folder's name in the games checkout (tapper).
    pub game: String,
    /// A clip of the mechanic's session to show (its number), stood still
    /// at `frame`. Leave it out to keep what is shown.
    #[serde(default)]
    pub clip: Option<u64>,
    /// The mechanic whose session the clip is in; the one open on the
    /// game's page when left out.
    #[serde(default)]
    pub mechanic: Option<String>,
    /// The frame of the clip, from 0 at its start.
    #[serde(default)]
    pub frame: Option<u64>,
    /// True goes back to live play, which the person plays (no clip).
    #[serde(default)]
    pub live: bool,
    /// `game` for the game's own pictures, or the id of one of the
    /// person's themes for this game (the page lists them).
    #[serde(default)]
    pub pictures: Option<String>,
    /// side (the two side by side), top (the model over the original at
    /// `opacity`), diff (their difference, black where they match) or solo
    /// (the model alone at its own size, played live from its own start).
    #[serde(default)]
    pub view: Option<String>,
    /// How much of the model shows over the original in the top view, 0
    /// to 1.
    #[serde(default)]
    pub opacity: Option<f64>,
}

impl ShowArgs {
    fn asks_anything(&self) -> bool {
        self.clip.is_some()
            || self.frame.is_some()
            || self.live
            || self.pictures.is_some()
            || self.view.is_some()
            || self.opacity.is_some()
    }
}

/// The ask's parts, checked: only those the call set.
fn parts(home: &Path, a: &ShowArgs) -> Result<Map<String, Value>, String> {
    let mut m = Map::new();
    if a.live && (a.clip.is_some() || a.frame.is_some()) {
        return Err("live is play with no clip: give live, or a clip and frame".into());
    }
    if a.live {
        m.insert("clip".into(), Value::Null);
    }
    if let Some(c) = a.clip {
        let mech = match &a.mechanic {
            Some(x) if name_ok(x) => x.clone(),
            Some(x) => return Err(format!("no mechanic {x:?}")),
            None => open_mechanic(home, &a.game)
                .ok_or("no mechanic is open on the game's page: name the clip's mechanic")?,
        };
        let toml = home
            .join("mechanics")
            .join(&a.game)
            .join(&mech)
            .join("clips")
            .join(format!("{c:03}.toml"));
        if !toml.is_file() {
            return Err(format!("{} has no clip {c} in {mech}", a.game));
        }
        m.insert("clip".into(), json!(c));
        m.insert("mechanic".into(), json!(mech));
        m.insert("frame".into(), json!(a.frame.unwrap_or(0)));
    } else if let Some(f) = a.frame {
        m.insert("frame".into(), json!(f));
    }
    if let Some(p) = &a.pictures {
        if !name_ok(p) {
            return Err(format!(
                "pictures is `{OWN_PICTURES}` or a theme's id (got {p:?})"
            ));
        }
        m.insert("pictures".into(), json!(p));
    }
    if let Some(v) = &a.view {
        if !VIEWS.contains(&v.as_str()) {
            return Err(format!("view is side, top, diff or solo (got {v:?})"));
        }
        m.insert("view".into(), json!(v));
    }
    if let Some(o) = a.opacity {
        if !(0.0..=1.0).contains(&o) {
            return Err(format!("opacity is 0 to 1 (got {o})"));
        }
        m.insert("opacity".into(), json!(o));
    }
    Ok(m)
}

/// Writes the ask (when the call asks anything) and returns what the
/// screen shows: waits a few seconds for an open page to show the ask.
pub fn show_at(home: &Path, a: &ShowArgs, wait: Duration) -> Result<Value, String> {
    let d = dir(home, &a.game)?;
    let mut asked = None;
    if a.asks_anything() {
        let m = parts(home, a)?;
        let seq = read(&d.join("screen-ask.json"))["seq"]
            .as_u64()
            .unwrap_or(0)
            + 1;
        let mut v = Value::Object(m);
        v["seq"] = json!(seq);
        v["at_ms"] = json!(now_ms());
        write(&d.join("screen-ask.json"), &v)?;
        asked = Some(v);
    }
    let open = |s: &Value| {
        s["at_ms"]
            .as_u64()
            .is_some_and(|t| now_ms().saturating_sub(t) < OPEN_WITHIN_MS)
    };
    let mut shown = read(&d.join("screen-shown.json"));
    if let Some(v) = &asked
        && open(&shown)
    {
        let seq = v["seq"].as_u64().unwrap_or(0);
        let t0 = Instant::now();
        while shown["applied"].as_u64().unwrap_or(0) < seq && t0.elapsed() < wait {
            std::thread::sleep(Duration::from_millis(100));
            shown = read(&d.join("screen-shown.json"));
        }
    }
    let page_open = open(&shown);
    let applied = asked.as_ref().map(|v| {
        page_open && shown["applied"].as_u64().unwrap_or(0) >= v["seq"].as_u64().unwrap_or(0)
    });
    let said = match (&asked, page_open, applied) {
        (_, false, _) => format!(
            "The screen page isn't open: open /mechanic/{}/screen.html on gp serve; it shows the ask when it opens.",
            a.game
        ),
        (Some(_), true, Some(false)) => {
            "The page is open and hasn't shown the ask yet (a clip or a theme can take a few seconds)."
                .to_owned()
        }
        _ => said_of(&shown),
    };
    Ok(json!({
        "game": a.game,
        "asked": asked,
        "page_open": page_open,
        "applied": applied,
        "shown": shown,
        "said": said,
    }))
}

/// What the page shows, in words.
fn said_of(s: &Value) -> String {
    let what = match s["clip"].as_u64() {
        Some(c) => format!(
            "clip {c} of {} at frame {}",
            s["mechanic"].as_str().unwrap_or("?"),
            s["frame"].as_u64().unwrap_or(0)
        ),
        None => match s["work_clip"].as_str() {
            Some(c) => format!(
                "the clip your refine works on, {c}, at frame {}",
                s["frame"].as_u64().unwrap_or(0)
            ),
            None => "live play".to_owned(),
        },
    };
    let view = match s["view"].as_str() {
        Some("top") => format!(
            "the model over the original at {:.0}%",
            s["opacity"].as_f64().unwrap_or(0.5) * 100.0
        ),
        Some("diff") => "the difference".to_owned(),
        Some("solo") => "the model alone".to_owned(),
        _ => "side by side".to_owned(),
    };
    let pics = match s["pictures"].as_str() {
        Some(OWN_PICTURES) | None => "the game's own pictures".to_owned(),
        Some(t) => format!("the theme {t}"),
    };
    let by = if s["by"] == "person" {
        " (the person's choice)"
    } else {
        ""
    };
    let error = match s["error"].as_str() {
        Some(e) => format!(" It couldn't show what you asked last: {e}."),
        None => String::new(),
    };
    format!("The screen shows {what}, {view}, in {pics}{by}.{error}")
}

/// `screen_show`: asks the game's screen page to show a clip, a frame, a
/// theme or a view, and says what it shows.
pub fn show(config: &Config, a: ShowArgs) -> Result<Reply, String> {
    Ok(Reply::ok(show_at(&config.home, &a, APPLY_WAIT)?))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn home() -> tempfile::TempDir {
        let d = tempfile::tempdir().unwrap();
        let m = d.path().join("mechanics/tapper");
        std::fs::create_dir_all(m.join("move/clips")).unwrap();
        std::fs::write(m.join("current"), "move\n").unwrap();
        std::fs::write(m.join("move/clips/001.toml"), "start = 4\nframes = 20\n").unwrap();
        d
    }

    fn args(v: Value) -> ShowArgs {
        serde_json::from_value(v).unwrap()
    }

    /// An ask keeps only what it set, its number goes up, and with no
    /// page open it says so and waits for nothing.
    #[test]
    fn an_ask_keeps_its_parts_and_counts() {
        let h = home();
        let v = show_at(
            h.path(),
            &args(json!({"game": "tapper", "view": "top", "opacity": 0.25})),
            Duration::ZERO,
        )
        .unwrap();
        assert_eq!(v["asked"]["seq"], 1);
        assert_eq!(v["asked"]["view"], "top");
        assert_eq!(v["asked"]["opacity"], 0.25);
        assert!(v["asked"].get("pictures").is_none());
        assert_eq!(v["page_open"], false);
        assert!(v["said"].as_str().unwrap().contains("isn't open"));
        let v = show_at(
            h.path(),
            &args(json!({"game": "tapper", "clip": 1, "frame": 9})),
            Duration::ZERO,
        )
        .unwrap();
        assert_eq!(v["asked"]["seq"], 2);
        assert_eq!(
            (v["asked"]["clip"].clone(), v["asked"]["mechanic"].clone()),
            (json!(1), json!("move"))
        );
        assert!(v["asked"].get("view").is_none());
        let v = show_at(
            h.path(),
            &args(json!({"game": "tapper", "live": true})),
            Duration::ZERO,
        )
        .unwrap();
        assert_eq!(v["asked"]["clip"], Value::Null);
        // Asking nothing reads it back and writes no ask.
        let v = show_at(h.path(), &args(json!({"game": "tapper"})), Duration::ZERO).unwrap();
        assert_eq!(v["asked"], Value::Null);
        assert_eq!(ask_of(h.path(), "tapper").unwrap()["seq"], 3);
    }

    /// A wrong part is refused in words, and nothing is written.
    #[test]
    fn a_wrong_ask_is_refused() {
        let h = home();
        for (a, says) in [
            (
                json!({"game": "tapper", "view": "over"}),
                "side, top, diff or solo",
            ),
            (json!({"game": "tapper", "opacity": 2.0}), "0 to 1"),
            (json!({"game": "tapper", "clip": 7}), "no clip 7"),
            (json!({"game": "tapper", "pictures": "../x"}), "theme's id"),
            (
                json!({"game": "tapper", "live": true, "clip": 1}),
                "live is play",
            ),
            (
                json!({"game": "../tapper", "view": "side"}),
                "a game's name",
            ),
        ] {
            let e = show_at(h.path(), &args(a), Duration::ZERO).unwrap_err();
            assert!(e.contains(says), "{e}");
        }
        assert_eq!(ask_of(h.path(), "tapper").unwrap(), Value::Null);
    }

    /// The page's report is what the call reads back, in words, and an
    /// open page that has applied the ask says it is shown.
    #[test]
    fn the_page_reports_what_it_shows() {
        let h = home();
        report(
            h.path(),
            "tapper",
            &json!({"applied": 1, "clip": 1, "mechanic": "move", "frame": 9,
                    "view": "top", "opacity": 0.5, "pictures": "last-call", "by": "person"}),
        )
        .unwrap();
        let v = show_at(
            h.path(),
            &args(json!({"game": "tapper", "view": "top"})),
            Duration::ZERO,
        )
        .unwrap();
        assert_eq!(v["page_open"], true);
        assert_eq!(v["applied"], true);
        assert_eq!(
            v["said"],
            "The screen shows clip 1 of move at frame 9, the model over the original at 50%, in the theme last-call (the person's choice)."
        );
        assert!(report(h.path(), "tapper", &json!([1])).is_err());
    }

    /// The work file keeps the clip's window through events with none, is
    /// live while fresh, and isn't once ended.
    #[test]
    fn the_work_keeps_its_window_and_ends() {
        let h = home();
        assert_eq!(work_of(h.path(), "tapper").unwrap(), Value::Null);
        work(
            h.path(),
            "tapper",
            &json!({"clip": "serve-1", "start": 100, "frames": 300, "from": 140, "to": 200}),
        )
        .unwrap();
        work(
            h.path(),
            "tapper",
            &json!({"clip": "serve-1", "start": 100, "frames": 300, "says": "round 1"}),
        )
        .unwrap();
        let v = work_of(h.path(), "tapper").unwrap();
        assert_eq!(
            (v["from"].clone(), v["to"].clone()),
            (json!(140), json!(200))
        );
        assert_eq!(v["live"], true);
        // Another clip starts with no window.
        work(
            h.path(),
            "tapper",
            &json!({"clip": "pour-2", "start": 9, "frames": 50}),
        )
        .unwrap();
        assert!(work_of(h.path(), "tapper").unwrap()["from"].is_null());
        work(
            h.path(),
            "tapper",
            &json!({"clip": "pour-2", "ended": true}),
        )
        .unwrap();
        assert_eq!(work_of(h.path(), "tapper").unwrap()["live"], false);
        let said = said_of(&json!({"work_clip": "pour-2", "frame": 3}));
        assert!(
            said.contains("your refine works on, pour-2, at frame 3"),
            "{said}"
        );
    }
}
