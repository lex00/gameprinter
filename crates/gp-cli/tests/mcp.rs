//! `gp mcp` driven over stdio as an MCP client would (docs/mcp.md): fork
//! Moss Hop into a scratch working copy, describe it, lower the jump's
//! gravity, preview the jump, see validation fail the jump check, put the
//! gravity back and see it pass.

use std::fs;
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};

use serde_json::{Value, json};

fn repo() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn gp(home: &Path) -> Command {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_gp"));
    cmd.env("GP_HOME", home)
        .env_remove("GP_STORE")
        .env_remove("GP_ROM_DIR")
        .env_remove("GP_LOG")
        .env_remove("GP_RUN_GROUP")
        .current_dir(repo());
    cmd
}

/// Builds Moss Hop and lifts its assets into `dir/lifted`; returns the ROM.
fn build_mosshop(dir: &Path, home: &Path) -> PathBuf {
    let out = Command::new(repo().join("scripts/mosshop/build.sh"))
        .arg(dir)
        .output()
        .expect("running scripts/mosshop/build.sh");
    assert!(
        out.status.success(),
        "Moss Hop didn't build; this test needs cc65 (ca65 and ld65), like just check:\n{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let rom = dir.join("mosshop.nes");
    let status = gp(home)
        .args(["assets", "lift"])
        .arg(&rom)
        .arg("-o")
        .arg(dir.join("lifted"))
        .args(["--map", "assets/mosshop.toml"])
        .stdout(Stdio::null())
        .status()
        .unwrap();
    assert!(status.success());
    rom
}

/// Starts `gp mcp` for `home`, with no ROM or assets, and initializes it.
fn connect_bare(home: &Path, work: &Path) -> Client {
    connect_cmd(gp(home), work)
}

/// [`connect_bare`] with the command to start from.
fn connect_cmd(mut cmd: Command, work: &Path) -> Client {
    let mut child = cmd
        .arg("mcp")
        .arg("--repo")
        .arg(repo())
        .arg("--work")
        .arg(work)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    let stdin = child.stdin.take();
    let stdout = BufReader::new(child.stdout.take().unwrap());
    let mut c = Client {
        child,
        stdin,
        stdout,
        next: 0,
        transcript: Vec::new(),
        progress: Vec::new(),
    };
    let init = c.request(
        "initialize",
        json!({
            "protocolVersion": "2025-11-25",
            "capabilities": {},
            "clientInfo": { "name": "gp-cli test", "version": "0" },
        }),
    );
    assert_eq!(
        init["result"]["serverInfo"]["name"], "game-printer",
        "{init}"
    );
    assert!(
        init["result"]["instructions"]
            .as_str()
            .is_some_and(|i| i.contains("Call next at the start of every conversation")),
        "{init}"
    );
    c.send(&json!({ "jsonrpc": "2.0", "method": "notifications/initialized" }));
    c
}

/// A minimal MCP client: newline-delimited JSON-RPC on the child's stdio.
struct Client {
    child: Child,
    stdin: Option<ChildStdin>,
    stdout: BufReader<ChildStdout>,
    next: u64,
    /// Each call and a one-line summary of its reply, for failure messages.
    transcript: Vec<String>,
    /// Progress notifications received.
    progress: Vec<Value>,
}

impl Client {
    fn send(&mut self, msg: &Value) {
        let stdin = self.stdin.as_mut().unwrap();
        writeln!(stdin, "{msg}").unwrap();
        stdin.flush().unwrap();
    }

    fn request(&mut self, method: &str, params: Value) -> Value {
        self.next += 1;
        let id = self.next;
        self.send(&json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params }));
        loop {
            let mut line = String::new();
            let n = self.stdout.read_line(&mut line).unwrap();
            assert!(
                n > 0,
                "the server closed stdout; so far:\n{}",
                self.transcript.join("\n")
            );
            let msg: Value = serde_json::from_str(&line)
                .unwrap_or_else(|e| panic!("not JSON-RPC ({e}): {line}"));
            if msg["id"] == id {
                return msg;
            }
            if msg["method"] == "notifications/progress" {
                self.progress.push(msg["params"].clone());
            }
        }
    }

    /// Calls a tool and returns its structured content and isError.
    fn call(&mut self, name: &str, args: Value) -> (Value, bool) {
        let msg = self.request("tools/call", json!({ "name": name, "arguments": args }));
        let result = &msg["result"];
        assert!(result.is_object(), "{name}: {msg}");
        let content = result["structuredContent"].clone();
        let is_error = result["isError"] == true;
        let text: Value =
            serde_json::from_str(result["content"][0]["text"].as_str().unwrap()).unwrap();
        assert_eq!(text, content, "the text content is the structured content");
        let mut summary = content.to_string();
        summary.truncate(160);
        self.transcript
            .push(format!("{name} {args} -> isError={is_error} {summary}"));
        (content, is_error)
    }
}

fn names(checks: &Value) -> Vec<String> {
    checks
        .as_array()
        .unwrap()
        .iter()
        .map(|c| {
            format!(
                "{} {} {}",
                c["part"].as_str().unwrap(),
                c["quantity"].as_str().unwrap(),
                c["against"].as_str().unwrap()
            )
        })
        .collect()
}

#[test]
#[ignore = "needs `gp validate`, which gameprinter hasn't got yet (wave 5)"]
fn an_agent_edits_previews_and_validates_moss_hop() {
    let home = tempfile::tempdir().unwrap();
    let tmp = tempfile::tempdir().unwrap();
    let rom = build_mosshop(tmp.path(), home.path());
    let work = tmp.path().join("work");
    let template = std::fs::read_to_string(repo().join("templates/mosshop/model.toml")).unwrap();

    let mut child = gp(home.path())
        .arg("mcp")
        .arg("--repo")
        .arg(repo())
        .arg("--work")
        .arg(&work)
        .arg("--rom")
        .arg(&rom)
        .arg("--assets")
        .arg(tmp.path().join("lifted"))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    let stdin = child.stdin.take();
    let stdout = BufReader::new(child.stdout.take().unwrap());
    let mut c = Client {
        child,
        stdin,
        stdout,
        next: 0,
        transcript: Vec::new(),
        progress: Vec::new(),
    };

    let init = c.request(
        "initialize",
        json!({
            "protocolVersion": "2025-11-25",
            "capabilities": {},
            "clientInfo": { "name": "gp-cli test", "version": "0" },
        }),
    );
    assert_eq!(
        init["result"]["serverInfo"]["name"], "game-printer",
        "{init}"
    );
    assert!(
        init["result"]["instructions"]
            .as_str()
            .is_some_and(|i| i.contains("Call next at the start of every conversation")),
        "{init}"
    );
    assert!(
        init["result"]["capabilities"]["tools"].is_object(),
        "{init}"
    );
    c.send(&json!({ "jsonrpc": "2.0", "method": "notifications/initialized" }));
    let tools = c.request("tools/list", json!({}));
    let listed: Vec<&str> = tools["result"]["tools"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["name"].as_str().unwrap())
        .collect();
    assert_eq!(
        listed,
        [
            "next",
            "registry_search",
            "template_fork",
            "surface_describe",
            "model_edit",
            "contract_edit",
            "assets_generate",
            "assets_place",
            "assets_edit",
            "assets_history",
            "assets_export",
            "sound_banks",
            "sound_describe",
            "sound_edit",
            "sound_preview",
            "run_preview",
            "validate",
            "model_refine",
            "model_questions",
            "show_explain",
            "build_publish",
            "capture_events",
            "requests",
            "moments",
            "refine_moment",
            "watch_start",
            "watch_events",
            "watch_watch",
            "watch_hook",
            "watch_detail",
            "watch_explore",
            "watch_point",
            "view_now",
            "capture_input"
        ]
    );
    let missing = c.request(
        "tools/call",
        json!({ "name": "no_such_tool", "arguments": {} }),
    );
    assert_eq!(missing["error"]["code"], -32602, "{missing}");

    let (fork, err) = c.call(
        "template_fork",
        json!({ "template": "mosshop", "copy": "hop" }),
    );
    assert!(!err, "{fork}");
    let copy = work.join("hop/model.toml");

    // Describe: the jump's gravity, its level and evidence.
    let (d, err) = c.call("surface_describe", json!({ "copy": "hop" }));
    assert!(!err, "{d}");
    let gravity = d["facts"]
        .as_array()
        .unwrap()
        .iter()
        .find(|f| f["path"] == "entities.player.jump.gravity")
        .cloned()
        .unwrap();
    assert_eq!(
        (&gravity["value"], &gravity["level"]),
        (&json!("0.25"), &json!("metric"))
    );

    // A contract rule whose condition doesn't parse is refused by gp model
    // check, and the contract is left as it was.
    let contract = std::fs::read_to_string(work.join("hop/fidelity.toml")).unwrap();
    let (r, err) = c.call(
        "contract_edit",
        json!({ "copy": "hop", "ops": [{ "op": "add", "rule": { "path": "entities.player.jump", "level": "free", "when": "level ==" } }] }),
    );
    assert!(err && r["written"] == false, "{r}");
    assert!(!r["problems"].as_array().unwrap().is_empty(), "{r}");
    assert_eq!(
        std::fs::read_to_string(work.join("hop/fidelity.toml")).unwrap(),
        contract
    );

    // A float is refused and nothing is written.
    let (r, err) = c.call(
        "model_edit",
        json!({ "copy": "hop", "edits": [{ "path": "entities.player.jump.gravity", "value": 0.1875 }] }),
    );
    assert!(err, "{r}");
    assert_eq!(std::fs::read_to_string(&copy).unwrap(), template);

    // Lower gravity: one fact changes, on one line.
    let (r, err) = c.call(
        "model_edit",
        json!({ "copy": "hop", "edits": [{ "path": "entities.player.jump.gravity", "value": "0.1875" }] }),
    );
    assert!(!err, "{r}");
    assert_eq!(
        r["changes"],
        json!([{ "change": "changed", "path": "entities.player.jump.gravity", "old": "0.25", "new": "0.1875" }])
    );
    let edited = std::fs::read_to_string(&copy).unwrap();
    let differing: Vec<_> = template
        .lines()
        .zip(edited.lines())
        .filter(|(a, b)| a != b)
        .collect();
    assert_eq!(differing, [("gravity = \"0.25\"", "gravity = \"0.1875\"")]);

    // Preview the jump: the peak is the model's own number, 64.1875 pixels
    // above the start at 176, where validation says the original's 47.5.
    let frames: Vec<u64> = (50..=120).collect();
    let (p, err) = c.call(
        "run_preview",
        json!({ "copy": "hop", "frames": 120, "holds": ["a@50-60"], "paths": ["entities.player[0].y"], "at": frames, "picture": true }),
    );
    assert!(!err, "{p}");
    let ys: Vec<String> = p["samples"]
        .as_array()
        .unwrap()
        .iter()
        .map(|s| {
            s["state"]["entities.player[0].y"]
                .as_str()
                .unwrap()
                .to_owned()
        })
        .collect();
    let peak = ys
        .iter()
        .map(|y| y.parse::<f64>().unwrap())
        .fold(f64::INFINITY, f64::min);
    assert_eq!(peak, 176.0 - 64.1875, "{ys:?}");
    assert_eq!(ys.last().unwrap(), "176", "landed again by frame 120");
    assert!(
        p["picture"].is_null() && p["picture_note"].is_string(),
        "{p}"
    );
    let (r, err) = c.call(
        "run_preview",
        json!({ "copy": "hop", "frames": 10, "at": [11] }),
    );
    assert!(
        err && r["error"].as_str().unwrap().contains("past the run"),
        "{r}"
    );
    let (r, err) = c.call(
        "run_preview",
        json!({ "copy": "hop", "frames": 100_000_000_000u64 }),
    );
    assert!(
        err && r["error"].as_str().unwrap().contains("at most"),
        "{r}"
    );

    // Validation fails the jump's height and airtime against the model's
    // numbers, and nothing else there. Against the original the jump fails
    // too, and so do parts the higher jump knocks on: in finish-level the
    // player now dies, so the state, the counters and the level graph part
    // from the original's.
    let (v, err) = c.call("validate", json!({ "copy": "hop" }));
    assert!(!err, "a failed check is a result, not a tool error: {v}");
    assert_eq!(v["ok"], false, "{v}");
    let failing = names(&v["failing"]);
    let against_model: Vec<&String> = failing.iter().filter(|f| f.ends_with(" model")).collect();
    assert_eq!(
        against_model,
        [
            "entities.player.jump height model",
            "entities.player.jump airtime model"
        ],
        "{v}"
    );
    for f in [
        "entities.player.jump height original",
        "entities.player.jump airtime original",
    ] {
        assert!(failing.iter().any(|g| g == f), "{f}: {failing:?}");
    }

    // Put it back: the copy is the template again, byte for byte, and
    // every check passes.
    let (r, err) = c.call(
        "model_edit",
        json!({ "copy": "hop", "edits": [{ "path": "entities.player.jump.gravity", "value": "0.25" }] }),
    );
    assert!(!err, "{r}");
    assert_eq!(std::fs::read_to_string(&copy).unwrap(), template);
    let (v, err) = c.call("validate", json!({ "copy": "hop" }));
    assert!(!err, "{v}");
    // Every check is exercised, the coins against the model's own events
    // too (crates/gp-cli/tests/validate.rs).
    assert_eq!(
        (&v["ok"], &v["failed"], &v["unexercised"]),
        (&json!(true), &json!(0), &json!(0)),
        "{v}"
    );
    assert!(v["passed"].as_u64().unwrap() >= 18, "{v}");

    // The committed template never changed.
    assert_eq!(
        std::fs::read_to_string(repo().join("templates/mosshop/model.toml")).unwrap(),
        template
    );

    // Closing stdin ends the session cleanly, with nothing more on stdout.
    drop(c.stdin.take());
    let mut rest = String::new();
    std::io::Read::read_to_string(&mut c.stdout, &mut rest).unwrap();
    assert_eq!(rest, "");
    let status = c.child.wait().unwrap();
    assert!(status.success(), "{status}\n{}", c.transcript.join("\n"));
    eprintln!("{}", c.transcript.join("\n"));
}

#[test]
fn mcp_takes_no_json_flag_and_no_work_dir_in_the_repo() {
    let home = tempfile::tempdir().unwrap();
    // --json would print gp's report on stdout, the MCP channel.
    let out = gp(home.path())
        .args(["--json", "mcp"])
        .stdin(Stdio::null())
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(2));
    assert!(
        out.stdout.is_empty(),
        "{}",
        String::from_utf8_lossy(&out.stdout)
    );
    // A work directory inside the repo is refused before it's created.
    let inside = repo().join("target/mcp-work-refused/copies");
    let out = gp(home.path())
        .arg("mcp")
        .arg("--work")
        .arg(&inside)
        .stdin(Stdio::null())
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&out.stderr).contains("inside the git work tree"));
    assert!(!inside.exists());
}

/// The tool `next` before any game (it asks for the game's file), then on
/// a game, where a hand-off the person makes while it waits wakes it.
#[test]
fn an_agent_asks_next_and_hears_a_hand_off() {
    let d = tempfile::tempdir().unwrap();
    let (home, work, games) = (
        d.path().join("home"),
        d.path().join("work"),
        d.path().join("games"),
    );
    fs::create_dir_all(&games).unwrap();
    let mut cmd = gp(&home);
    cmd.env("GP_GAMES_DIR", &games);
    let mut c = connect_cmd(cmd, &work);
    let (v, err) = c.call("next", json!({}));
    assert!(!err, "{v}");
    assert_eq!(v["stage"]["id"], "P1", "{v}");
    assert_eq!(v["stands"], "No game file is on this computer yet.");
    assert_eq!(v["next"]["who"], "person");
    assert!(v["text"].as_str().unwrap().contains("Next (you): "), "{v}");
    let (v, err) = c.call("next", json!({"game": "nope"}));
    assert!(err, "{v}");
    finish(c);

    // A game with Moss Hop's model, its player mapped, and a quest log.
    let g = games.join("hop");
    fs::create_dir_all(&g).unwrap();
    fs::write(g.join("package.toml"), "").unwrap();
    fs::write(g.join("template.toml"), "[template]\ntitle = \"Hop\"\n").unwrap();
    for f in ["model.toml", "mapping.toml"] {
        fs::copy(repo().join("templates/mosshop").join(f), g.join(f)).unwrap();
    }
    let m = home.join("mechanics/hop");
    fs::create_dir_all(m.join("walk/clips")).unwrap();
    fs::write(m.join("walk/clips/001.toml"), "start = 0\nframes = 10\n").unwrap();
    fs::write(
        m.join("queue.json"),
        r#"[{"mechanic": "walk", "status": "next"}]"#,
    )
    .unwrap();
    let mut cmd = gp(&home);
    cmd.env("GP_GAMES_DIR", &games);
    let mut c = connect_cmd(cmd, &work);
    let talk = m.join("walk/talk.ndjson");
    let hand = std::thread::spawn(move || {
        std::thread::sleep(std::time::Duration::from_millis(2500));
        fs::write(
            &talk,
            "{\"by\":\"player\",\"text\":\"Let's do walk\",\"n\":1}\n",
        )
        .unwrap();
    });
    let t = std::time::Instant::now();
    let (v, err) = c.call("next", json!({"game": "hop", "wait_s": 20}));
    hand.join().unwrap();
    assert!(!err, "{v}");
    assert_eq!(v["changed"], true, "{v}");
    assert!(
        t.elapsed() < std::time::Duration::from_secs(15),
        "{:?}",
        t.elapsed()
    );
    assert_eq!(v["facts"]["handoffs"], json!(["walk"]), "{v}");
    finish(c);
}

/// Starts `gp mcp` on Moss Hop's lifted assets and initializes a session.
fn connect(home: &Path, work: &Path, rom: &Path, lifted: &Path) -> Client {
    let mut child = gp(home)
        .arg("mcp")
        .arg("--repo")
        .arg(repo())
        .arg("--work")
        .arg(work)
        .arg("--rom")
        .arg(rom)
        .arg("--assets")
        .arg(lifted)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    let stdin = child.stdin.take();
    let stdout = BufReader::new(child.stdout.take().unwrap());
    let mut c = Client {
        child,
        stdin,
        stdout,
        next: 0,
        transcript: Vec::new(),
        progress: Vec::new(),
    };
    let init = c.request(
        "initialize",
        json!({
            "protocolVersion": "2025-11-25",
            "capabilities": {},
            "clientInfo": { "name": "gp-cli test", "version": "0" },
        }),
    );
    assert_eq!(
        init["result"]["serverInfo"]["name"], "game-printer",
        "{init}"
    );
    assert!(
        init["result"]["instructions"]
            .as_str()
            .is_some_and(|i| i.contains("Call next at the start of every conversation")),
        "{init}"
    );
    c.send(&json!({ "jsonrpc": "2.0", "method": "notifications/initialized" }));
    c
}

/// A made-up 16x16 sprite, 2x2 tiles: a ring of index 1 around a disc of
/// index 2 with a dot of index 3, on index 0.
fn ring_sprite() -> Vec<u8> {
    let mut px = vec![0u8; 16 * 16];
    for y in 0..16i32 {
        for x in 0..16i32 {
            let d = (x - 8) * (x - 8) + (y - 8) * (y - 8);
            px[(y * 16 + x) as usize] = match d {
                0..=4 => 3,
                5..=30 => 2,
                31..=56 => 1,
                _ => 0,
            };
        }
    }
    px
}

/// The slot catalog in surface_describe and assets_place: an agent replaces
/// Moss Hop's player sprite, the copy's assets record it, and the game
/// runs to the same state on every baseline.
#[test]
fn an_agent_replaces_the_player_sprite() {
    use base64::Engine as _;
    let home = tempfile::tempdir().unwrap();
    let tmp = tempfile::tempdir().unwrap();
    let rom = build_mosshop(tmp.path(), home.path());
    let lifted = tmp.path().join("lifted");
    let work = tmp.path().join("work");
    let mut c = connect(home.path(), &work, &rom, &lifted);

    let (fork, err) = c.call(
        "template_fork",
        json!({ "template": "mosshop", "copy": "art" }),
    );
    assert!(!err, "{fork}");

    // The catalog: the tiles slot is presentation, original, and says the
    // player's sprite is a 16x16 image of tiles 16 to 19.
    let (d, err) = c.call("surface_describe", json!({ "copy": "art" }));
    assert!(!err, "{d}");
    let slot = |d: &Value, name: &str| {
        d["slots"]
            .as_array()
            .unwrap()
            .iter()
            .find(|s| s["name"] == name)
            .cloned()
            .unwrap()
    };
    let tiles = slot(&d, "tiles");
    assert_eq!(
        (&tiles["reads"], &tiles["status"], &tiles["contract_level"]),
        (&json!("presentation"), &json!("original"), &json!("style")),
        "{tiles}"
    );
    assert_eq!(tiles["shape"]["image_size"], json!([128, 256]));
    let player = tiles["used_by"]
        .as_array()
        .unwrap()
        .iter()
        .find(|u| u["entity"] == "player")
        .cloned()
        .unwrap();
    assert_eq!(player["image_size"], json!([16, 16]));
    assert_eq!(player["tiles"], json!([16, 17, 18, 19]));
    assert_eq!(slot(&d, "level")["reads"], "simulation");

    let (before, err) = c.call(
        "run_preview",
        json!({ "copy": "art", "baseline": "stomp-then-pit", "paths": ["counters.deaths"] }),
    );
    assert!(!err, "{before}");

    // A wrong size is refused with nothing written.
    let small = gp_assets::chr::write_png(16, 8, &[1u8; 16 * 8]);
    let b64 = |b: &[u8]| base64::engine::general_purpose::STANDARD.encode(b);
    let (r, err) = c.call(
        "assets_place",
        json!({ "copy": "art", "slot": "tiles", "entity": "player", "png_base64": b64(&small),
                "license": "CC0-1.0", "origin": "generated" }),
    );
    assert!(err, "{r}");
    assert_eq!(r["written"], false);
    assert!(r["problems"][0].as_str().unwrap().contains("16"), "{r}");
    assert!(
        !work.join("art/assets").exists(),
        "a refusal writes nothing"
    );
    // A simulation slot is refused.
    let (r, err) = c.call(
        "assets_place",
        json!({ "copy": "art", "slot": "level", "png_base64": b64(&small),
                "license": "CC0-1.0", "origin": "generated" }),
    );
    assert!(err, "{r}");

    // The player's sprite.
    let sprite = ring_sprite();
    let png = gp_assets::chr::write_png(16, 16, &sprite);
    let (r, err) = c.call(
        "assets_place",
        json!({ "copy": "art", "slot": "tiles", "entity": "player", "png_base64": b64(&png),
                "license": "CC0-1.0", "origin": "generated", "generator": "gp-cli test",
                "note": "a ring" }),
    );
    assert!(!err, "{r}");
    assert_eq!(r["tiles_changed"], json!([16, 17, 18, 19]), "{r}");
    // 4 tiles of 16 bytes; at most every byte of them.
    let changed = r["bytes_changed"].as_u64().unwrap();
    assert!(changed > 0 && changed <= 64, "{r}");
    assert_eq!(r["simulation"]["unchanged"], true);
    assert_eq!(r["simulation"]["digests"].as_object().unwrap().len(), 8);
    assert_eq!(r["simulation"]["baselines"], 8);
    assert_eq!(r["state"]["status"], "replaced");

    // The copy holds its own assets and their state; the lift is untouched.
    let assets = work.join("art/assets");
    let state = std::fs::read_to_string(assets.join("slots.toml")).unwrap();
    assert!(state.contains("status = \"replaced\""), "{state}");
    assert!(
        state.contains("license = \"LicenseRef-template-source AND CC0-1.0\""),
        "{state}"
    );
    // The originals of an open-tier template are under its own terms.
    assert!(
        state.contains("license = \"LicenseRef-template-source\""),
        "{state}"
    );
    let lifted_png = std::fs::read(lifted.join("tiles.png")).unwrap();
    let copy_png = std::fs::read(assets.join("tiles.png")).unwrap();
    assert_ne!(lifted_png, copy_png);
    let sheet = gp_assets::chr::read_png(&copy_png, (128, 256)).unwrap();
    for (k, t) in [16u32, 17, 18, 19].iter().enumerate() {
        let (sx, sy) = ((k % 2) * 8, (k / 2) * 8);
        let (tx, ty) = ((t % 16) * 8, (t / 16) * 8);
        for y in 0..8 {
            for x in 0..8 {
                assert_eq!(
                    sheet[((ty + y) * 128 + tx + x) as usize],
                    sprite[(sy + y as usize) * 16 + sx + x as usize]
                );
            }
        }
    }

    // The catalog reports the replacement.
    let (d, _) = c.call("surface_describe", json!({ "copy": "art" }));
    let tiles = slot(&d, "tiles");
    assert_eq!(tiles["status"], "replaced");
    // Only the player's tiles are new; the rest keep the original's terms.
    assert_eq!(
        tiles["provenance"]["license"],
        "LicenseRef-template-source AND CC0-1.0"
    );
    assert_eq!(slot(&d, "level")["status"], "original");

    // The same run, the same state.
    let (after, err) = c.call(
        "run_preview",
        json!({ "copy": "art", "baseline": "stomp-then-pit", "paths": ["counters.deaths"] }),
    );
    assert!(!err, "{after}");
    assert_eq!(before["state_hash"], after["state_hash"]);

    // game-printer also ran `gp interp run --baseline` on the copy's assets
    // and packed them into a ROM. gameprinter's `interp run` runs a game's
    // module, its `assets pack` only checks a pack against the original,
    // and nothing here writes a ROM, so neither applies to an edited copy.

    drop(c.stdin.take());
    let status = c.child.wait().unwrap();
    assert!(status.success(), "{status}\n{}", c.transcript.join("\n"));
}

fn finish(mut c: Client) {
    drop(c.stdin.take());
    let status = c.child.wait().unwrap();
    assert!(status.success(), "{status}\n{}", c.transcript.join("\n"));
}

/// The last tools: an agent finds Moss Hop in the registry, forks it,
/// generates art into its slots, and `build_publish` checks the copy and
/// says what it would build. The build itself is the ignored test below.
#[test]
fn an_agent_finds_moss_hop_generates_art_and_checks_a_publish() {
    let home = tempfile::tempdir().unwrap();
    let tmp = tempfile::tempdir().unwrap();
    let rom = build_mosshop(tmp.path(), home.path());
    let lifted = tmp.path().join("lifted");
    let work = tmp.path().join("work");
    let mut c = connect(home.path(), &work, &rom, &lifted);

    let (r, err) = c.call(
        "registry_search",
        json!({ "query": "mosshop", "platform": "nes", "genre": "platformer", "roles": ["player", "enemy"] }),
    );
    assert!(!err, "{r}");
    assert_eq!(r["templates_found"], 1, "{r}");
    let t = &r["templates"][0];
    assert_eq!(t["template"], "mosshop");
    assert_eq!(t["tier"], "open");
    assert_eq!(t["ready"], true, "{t}");
    assert_eq!(t["needs"]["extracted"]["matches_pin"], true, "{t}");
    // game-printer also searched for Super Mario Bros., whose template
    // gameprinter doesn't carry. The mechanic index, provenance and search
    // by example (docs/originals.md), over the three originals beside
    // Moss Hop.
    let (r, err) = c.call("registry_search", json!({ "inspired_by": "mosshop" }));
    assert!(!err, "{r}");
    let names: Vec<&str> = r["templates"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|t| t["template"].as_str())
        .collect();
    assert_eq!(names, ["badge-in", "swing-state"], "{r}");
    let (r, _) = c.call(
        "registry_search",
        json!({ "mechanics": ["enemy:contact.kick", "player:jump"] }),
    );
    assert!(
        r["templates"]
            .as_array()
            .unwrap()
            .iter()
            .all(|t| t["mechanics"]
                .as_array()
                .unwrap()
                .contains(&json!("enemy:contact.kick"))),
        "{r}"
    );
    let (r, _) = c.call("registry_search", json!({ "like": "mosshop" }));
    assert_eq!(r["templates"][0]["template"], "mosshop", "{r}");
    assert_eq!(r["templates"][0]["similarity"], 1.0, "{r}");
    assert!(
        r["templates"][0]["taken_by"]
            .as_array()
            .unwrap()
            .contains(&json!("badge-in")),
        "{r}"
    );

    let (r, err) = c.call(
        "template_fork",
        json!({ "template": "mosshop", "copy": "gen" }),
    );
    assert!(!err, "{r}");

    // The same arguments give the same candidate; a seed changes it.
    let candidate = |c: &mut Client, extra: Value| {
        let mut args =
            json!({ "copy": "gen", "slot": "tiles", "entity": "player", "place": false });
        args.as_object_mut()
            .unwrap()
            .extend(extra.as_object().unwrap().clone());
        let (r, err) = c.call("assets_generate", args);
        assert!(!err, "{r}");
        r
    };
    let a = candidate(&mut c, json!({ "seed": 1 }));
    assert_eq!(
        a["candidate"],
        candidate(&mut c, json!({ "seed": 1 }))["candidate"]
    );
    assert_ne!(
        a["candidate"],
        candidate(&mut c, json!({ "seed": 2 }))["candidate"]
    );
    assert_eq!(a["written"], false);

    // Generate into the player's tiles and palette, the whole sheet and the
    // whole palettes; each is placed with the baselines run before and after.
    let mut digests = Vec::new();
    for args in [
        json!({ "slot": "tiles", "entity": "player", "generator": "shapes", "seed": 5 }),
        json!({ "slot": "palettes", "entity": "slug", "generator": "recolor", "hue_shift": 4 }),
        json!({ "slot": "tiles", "generator": "shapes", "seed": 6 }),
        json!({ "slot": "palettes", "generator": "stub" }),
    ] {
        let mut full = json!({ "copy": "gen" });
        full.as_object_mut()
            .unwrap()
            .extend(args.as_object().unwrap().clone());
        let (r, err) = c.call("assets_generate", full);
        assert!(!err, "{args}: {r}");
        assert_eq!(r["written"], true);
        assert_eq!(r["simulation"]["unchanged"], true);
        assert_eq!(r["simulation"]["baselines"], 8, "{r}");
        digests.push(r["simulation"]["digests"].clone());
    }
    assert!(digests.windows(2).all(|w| w[0] == w[1]), "{digests:?}");

    let (d, _) = c.call("surface_describe", json!({ "copy": "gen" }));
    let status = |name: &str| {
        d["slots"]
            .as_array()
            .unwrap()
            .iter()
            .find(|s| s["name"] == name)
            .unwrap()["status"]
            .clone()
    };
    assert_eq!(status("tiles"), "replaced", "{d}");
    assert_eq!(status("palettes"), "stub", "{d}");

    // Moss Hop is an open template: its own level and tileset may ship.
    let out = tmp.path().join("published");
    let (r, err) = c.call(
        "build_publish",
        json!({ "copy": "gen", "out": out, "dry_run": true }),
    );
    assert!(!err, "{r}");
    assert_eq!(r["written"], false);
    assert_eq!(r["manifest"]["template"]["id"], "mosshop");
    assert_eq!(
        r["manifest"]["assets"]["slots"]["tiles"]["origin"],
        "generated"
    );
    assert!(!out.exists());
    // The Bevy game was dropped: a target other than web-preview is refused
    // with the list of targets, before any check runs.
    let (r, err) = c.call(
        "build_publish",
        json!({ "copy": "gen", "out": out, "target": "web", "dry_run": true }),
    );
    assert!(err, "{r}");
    assert!(
        r.to_string()
            .contains("there is no target `web`; the targets are: web-preview"),
        "{r}"
    );

    // An edit by hand to a slot's file shows as modified, and a publish
    // refuses it.
    let tiles = work.join("gen/assets/tiles.png");
    let mut bytes = std::fs::read(&tiles).unwrap();
    bytes.push(0);
    std::fs::write(&tiles, &bytes).unwrap();
    let (r, err) = c.call(
        "build_publish",
        json!({ "copy": "gen", "out": out, "dry_run": true }),
    );
    assert!(err, "{r}");
    assert!(r["problems"].to_string().contains("changed since"), "{r}");
    finish(c);
}

/// A real build into a directory, the interpreter preview: needs the
/// wasm32-unknown-unknown target. Run by `just
/// check-cycle` as the step `publish-mosshop`.
#[test]
#[ignore = "builds the wasm module: minutes; game-printer ran it in just check-cycle, which gameprinter hasn't got"]
fn build_publish_builds_moss_hops_preview() {
    let home = tempfile::tempdir().unwrap();
    let tmp = tempfile::tempdir().unwrap();
    let rom = build_mosshop(tmp.path(), home.path());
    let lifted = tmp.path().join("lifted");
    let work = tmp.path().join("work");
    let mut c = connect(home.path(), &work, &rom, &lifted);
    let (r, err) = c.call(
        "template_fork",
        json!({ "template": "mosshop", "copy": "pub" }),
    );
    assert!(!err, "{r}");
    let (r, err) = c.call(
        "assets_generate",
        json!({ "copy": "pub", "slot": "tiles", "generator": "shapes" }),
    );
    assert!(!err, "{r}");
    let out = tmp.path().join("published");
    let (r, err) = c.call("build_publish", json!({ "copy": "pub", "out": out }));
    assert!(!err, "{r}");
    assert_eq!(r["written"], true);
    for f in ["index.html", "gp_web.wasm", "preview.json", "manifest.json"] {
        assert!(out.join(f).is_file(), "{f} is missing: {r}");
    }
    let manifest: Value =
        serde_json::from_slice(&std::fs::read(out.join("manifest.json")).unwrap()).unwrap();
    assert_eq!(manifest, r["manifest"]);
    let digest = manifest["files"]["gp_web.wasm"].as_str().unwrap();
    assert_eq!(digest.len(), 64);
    // Building again over the earlier build is allowed and gives the same
    // manifest, file for file.
    let (again, err) = c.call("build_publish", json!({ "copy": "pub", "out": out }));
    assert!(!err, "{again}");
    assert_eq!(
        again["manifest"]["files"]["preview.json"],
        manifest["files"]["preview.json"]
    );
    finish(c);
}

/// An agent makes Moss Hop's walk wrong, and model_refine measures it back
/// from the baselines in two rounds; nothing is left to ask.
#[test]
#[ignore = "needs `gp input-log generate` and `gp model refine`, which gameprinter hasn't got yet"]
fn an_agent_refines_a_wrong_walk_speed_back() {
    let home = tempfile::tempdir().unwrap();
    let tmp = tempfile::tempdir().unwrap();
    let rom = build_mosshop(tmp.path(), home.path());
    let work = tmp.path().join("work");
    let mut c = connect(home.path(), &work, &rom, &tmp.path().join("lifted"));
    let (r, err) = c.call(
        "template_fork",
        json!({ "template": "mosshop", "copy": "fix" }),
    );
    assert!(!err, "{r}");
    let (r, err) = c.call(
        "model_edit",
        json!({ "copy": "fix", "edits": [{ "path": "entities.player.run.walk_speed", "value": 2 }] }),
    );
    assert!(!err, "{r}");
    // The person watches a capture run; the agent's calls are logged in it,
    // and the long call reports progress to a client that asks.
    let run = home.path().join("capture/watching");
    std::fs::create_dir_all(&run).unwrap();
    std::fs::write(run.join("run.json"), "{}").unwrap();
    let started = std::time::Instant::now();
    let msg = c.request(
        "tools/call",
        json!({ "name": "model_refine", "arguments": { "copy": "fix", "run": "watching" }, "_meta": { "progressToken": "refine-1" } }),
    );
    let took = started.elapsed();
    let r = msg["result"]["structuredContent"].clone();
    assert_eq!(msg["result"]["isError"], false, "{msg}");
    if took > std::time::Duration::from_millis(1500) {
        assert!(!c.progress.is_empty(), "{took:?} without progress");
        assert!(c.progress.iter().all(|p| p["progressToken"] == "refine-1"));
        let n: Vec<f64> = c
            .progress
            .iter()
            .map(|p| p["progress"].as_f64().unwrap())
            .collect();
        assert!(
            n.windows(2).all(|w| w[1] > w[0]),
            "progress increases: {n:?}"
        );
    }
    let logged = std::fs::read_to_string(run.join("inputs.ndjson")).unwrap();
    let line: Value = serde_json::from_str(logged.lines().next().unwrap()).unwrap();
    assert_eq!(line["actor"], "agent");
    assert_eq!(line["tool"], "model_refine");
    assert_eq!(line["args"], r#"{"copy":"fix"}"#);
    assert_eq!(
        (&r["ok"], &r["iterations"]),
        (&json!(true), &json!(2)),
        "{r}"
    );
    assert!(
        r["applied"]
            .as_array()
            .unwrap()
            .iter()
            .any(|a| a == "entities.player.run.walk_speed = 1"),
        "{r}"
    );
    let (q, err) = c.call("model_questions", json!({ "copy": "fix" }));
    assert!(!err, "{q}");
    assert_eq!(q["open"], 0, "{q}");
    drop(c.stdin.take());
    let _ = c.child.wait();
}

/// `assets_edit` on Moss Hop's player: the edit works from the recorded
/// baselines (every pose the player was seen in), keeps all five state
/// digests, writes a before/after preview and undoes back to the original.
/// With `GP_EDIT_PREVIEW=DIR` the preview is copied there to look at.
#[test]
fn an_agent_edits_the_player_sprite_from_its_recordings() {
    use base64::Engine as _;
    let home = tempfile::tempdir().unwrap();
    let tmp = tempfile::tempdir().unwrap();
    let rom = build_mosshop(tmp.path(), home.path());
    let lifted = tmp.path().join("lifted");
    let work = tmp.path().join("work");
    let mut c = connect(home.path(), &work, &rom, &lifted);
    let (fork, err) = c.call(
        "template_fork",
        json!({ "template": "mosshop", "copy": "edit" }),
    );
    assert!(!err, "{fork}");
    let original = fs::read(lifted.join("tiles.png")).unwrap();

    // Turn him round, thicken his outline at the corners and swap his moss
    // and eye colors: one request, three ops.
    let (r, err) = c.call(
        "assets_edit",
        json!({ "copy": "edit", "entity": "player", "prompt": "face the other way, bolder outline, swap moss and eye colors",
                "ops": [
                    { "op": "flip", "axis": "x" },
                    { "op": "outline", "index": 1, "corners": true },
                    { "op": "recolor", "pairs": [[2, 3], [3, 2]] },
                ] }),
    );
    assert!(!err, "{r}");
    assert_eq!(r["written"], true, "{r}");
    assert_eq!(r["simulation"]["unchanged"], true);
    assert_eq!(r["simulation"]["baselines"], 8, "{r}");
    let changed = r["tiles_changed"].as_array().unwrap();
    assert!(
        !changed.is_empty()
            && changed
                .iter()
                .all(|t| (16..=19).contains(&t.as_u64().unwrap())),
        "{r}"
    );
    // The recordings show the player standing, running and in the air.
    let poses: Vec<&str> = r["contexts"]["poses"]
        .as_array()
        .unwrap()
        .iter()
        .map(|p| p["pose"].as_str().unwrap())
        .collect();
    assert!(poses.iter().any(|p| p.contains("/ground/")), "{poses:?}");
    assert!(poses.iter().any(|p| p.contains("/rising/")), "{poses:?}");
    assert!(r["critic"]["score"].is_number(), "{r}");
    let png = base64::engine::general_purpose::STANDARD
        .decode(r["preview"]["png_base64"].as_str().unwrap())
        .unwrap();
    assert!(work.join("edit/previews/edit-1.png").is_file());
    if let Ok(dir) = std::env::var("GP_EDIT_PREVIEW") {
        fs::write(Path::new(&dir).join("mosshop-player-edit.png"), &png).unwrap();
        fs::write(
            Path::new(&dir).join("mosshop-player-edit.json"),
            r.to_string(),
        )
        .unwrap();
    }

    let (u, err) = c.call("assets_edit", json!({ "copy": "edit", "undo": true }));
    assert!(!err, "{u}");
    assert_eq!(u["simulation"]["unchanged"], true);
    assert_eq!(u["state"]["status"], "original", "{u}");
    assert_eq!(
        fs::read(work.join("edit/assets/tiles.png")).unwrap(),
        original
    );
    finish(c);
}

/// Phase 1 of docs/sprite-editing.md end to end: an agent proposes a
/// recolor of the player's cap (the moss on his head) in a live capture
/// run, a person accepts it there with a scripted input, the agent sees the
/// decision, makes a second edit, and undoes both.
#[test]
#[ignore = "needs `gp capture input`, the capture-run loop the editor port brings"]
fn a_person_accepts_an_agents_sprite_edit_live_and_the_agent_undoes_twice() {
    use base64::Engine as _;
    let home = tempfile::tempdir().unwrap();
    let tmp = tempfile::tempdir().unwrap();
    let rom = build_mosshop(tmp.path(), home.path());
    let lifted = tmp.path().join("lifted");
    let work = tmp.path().join("work");
    // A capture run for the live panel; its stages aren't needed here.
    let run = home.path().join("capture/live");
    fs::create_dir_all(&run).unwrap();
    fs::write(
        run.join("run.json"),
        json!({ "schema": 0, "run": "live", "model": repo().join("templates/mosshop"), "rom": null,
                "baselines": null, "recordings": [], "preview": false, "started_ms": 0, "round": 0,
                "inputs_applied": 0 })
        .to_string(),
    )
    .unwrap();
    let mut c = connect(home.path(), &work, &rom, &lifted);
    let (fork, err) = c.call(
        "template_fork",
        json!({ "template": "mosshop", "copy": "edit" }),
    );
    assert!(!err, "{fork}");
    let original = fs::read(lifted.join("tiles.png")).unwrap();

    // The cap: the top five rows of the 16x16 sprite, moss (index 2) to
    // the eye's color (index 3). A proposal, shown in the run.
    let (p, err) = c.call(
        "assets_edit",
        json!({ "copy": "edit", "entity": "player", "prompt": "make his moss cap the color of his eye",
                "ops": [{ "op": "recolor", "pairs": [[2, 3]], "region": { "x": 0, "y": 0, "w": 16, "h": 5 } }],
                "place": false, "run": "live" }),
    );
    assert!(!err, "{p}");
    assert_eq!(p["status"], "proposed", "{p}");
    assert_eq!(p["written"], false);
    assert!(p["pixels_changed"].as_u64().unwrap() > 0, "{p}");
    let shots = p["in_context"]["shots"].as_array().unwrap();
    assert!(shots.len() >= 5, "a shot per pose: {shots:?}");
    assert!(
        shots.iter().any(|s| {
            // The death: its event, or the player's dying state.
            let pose = s["pose"].as_str().unwrap();
            pose.contains("player.died") || pose.starts_with("dying/")
        }),
        "{shots:?}"
    );
    let ctx = base64::engine::general_purpose::STANDARD
        .decode(p["in_context"]["png_base64"].as_str().unwrap())
        .unwrap();
    let readability = &p["in_context"]["readability"];
    assert_eq!(readability["name"], "context.readability", "{p}");
    assert_eq!(
        readability["frames"], 2280,
        "the eight baselines' frames: {readability}"
    );
    assert!(
        readability["after"]["share_readable"].is_number(),
        "{readability}"
    );
    if let Ok(dir) = std::env::var("GP_EDIT_PREVIEW") {
        fs::write(Path::new(&dir).join("mosshop-cap-context.png"), &ctx).unwrap();
        fs::write(
            Path::new(&dir).join("mosshop-cap-proposal.json"),
            p.to_string(),
        )
        .unwrap();
    }
    assert_eq!(fs::read(lifted.join("tiles.png")).unwrap(), original);
    let inputs = fs::read_to_string(run.join("inputs.ndjson")).unwrap();
    let ann: Value = inputs
        .lines()
        .filter_map(|l| serde_json::from_str::<Value>(l).ok())
        .find(|l| l["kind"] == "asset_edit")
        .unwrap();
    assert_eq!(ann["kind"], "asset_edit", "{ann}");
    assert_eq!(ann["item"], "edit:1");
    assert_eq!(ann["data"]["status"], "proposed");

    // The person accepts it in the run, as the live panel's button does.
    let out = gp(home.path())
        .args([
            "--json", "capture", "input", "live", "--kind", "accept", "--item", "edit:1",
        ])
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let d: Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(d["decision"]["status"], "accepted", "{d}");
    assert_eq!(d["decision"]["simulation"]["unchanged"], true, "{d}");
    let accepted = fs::read(work.join("edit/assets/tiles.png")).unwrap();
    assert_ne!(accepted, original);

    // The agent sees the decision.
    let (h, err) = c.call("assets_history", json!({ "copy": "edit" }));
    assert!(!err, "{h}");
    assert_eq!(h["placed"][0]["status"], "accepted", "{h}");
    assert_eq!(h["placed"][0]["decided_by"], "person");
    assert_eq!(h["waiting"], json!([]));

    // A second edit, placed at once, then undone twice: back to the lift.
    let (e2, err) = c.call(
        "assets_edit",
        json!({ "copy": "edit", "entity": "player", "ops": [{ "op": "outline", "index": 1, "corners": true }], "run": "live" }),
    );
    assert!(!err, "{e2}");
    assert_eq!(e2["seq"], 2, "{e2}");
    let (u, err) = c.call(
        "assets_edit",
        json!({ "copy": "edit", "undo": true, "run": "live" }),
    );
    assert!(!err, "{u}");
    assert_eq!(u["undone"]["seq"], 2);
    assert_eq!(
        fs::read(work.join("edit/assets/tiles.png")).unwrap(),
        accepted
    );
    let (u, err) = c.call("assets_edit", json!({ "copy": "edit", "undo": true }));
    assert!(!err, "{u}");
    assert_eq!(u["undone"]["seq"], 1);
    assert_eq!(u["state"]["status"], "original", "{u}");
    assert_eq!(
        fs::read(work.join("edit/assets/tiles.png")).unwrap(),
        original
    );
    let (h, _) = c.call("assets_history", json!({ "copy": "edit" }));
    assert_eq!(h["undone"].as_array().unwrap().len(), 2, "{h}");
    finish(c);

    // The run's next round reports both edits in its events.
    let ev = gp(home.path())
        .args(["--json", "capture", "resume", "live"])
        .output()
        .unwrap();
    let _ = ev;
    let text = fs::read_to_string(run.join("events.ndjson")).unwrap_or_default();
    let items: Vec<Value> = text
        .lines()
        .filter_map(|l| serde_json::from_str::<Value>(l).ok())
        .filter(|e| e["kind"] == "item" && e["data"]["what"] == "asset_edit")
        .collect();
    assert!(
        items
            .iter()
            .any(|e| e["data"]["id"] == "edit:1" && e["data"]["status"] == "undone"),
        "{items:?}"
    );
}

/// Phase 3 of docs/sprite-editing.md: `gp assets open` writes Moss Hop's
/// player for Pixelorama by the image config, starts the editor (a fake
/// one here, named by GP_PIXELORAMA), and a save over the exported sheet
/// comes back as a proposal in the live run; a person accepts it, and the
/// slot records it as uploaded from Pixelorama.
#[test]
#[ignore = "needs `gp assets open`, which gameprinter hasn't got"]
fn a_save_in_a_pixel_editor_comes_back_as_a_proposal() {
    let home = tempfile::tempdir().unwrap();
    let tmp = tempfile::tempdir().unwrap();
    let rom = build_mosshop(tmp.path(), home.path());
    let _ = rom;
    let lifted = tmp.path().join("lifted");
    let work = tmp.path().join("work");
    let run = home.path().join("capture/live");
    fs::create_dir_all(&run).unwrap();
    fs::write(
        run.join("run.json"),
        json!({ "schema": 1, "run": "live", "model": repo().join("templates/mosshop"), "rom": null,
                "baselines": null, "recordings": [], "preview": false, "started_ms": 0, "round": 0,
                "inputs_applied": 0 })
        .to_string(),
    )
    .unwrap();
    // The fake editor writes down the file it was asked to open.
    let opened = tmp.path().join("opened.txt");
    let editor = tmp.path().join("pixelorama");
    fs::write(
        &editor,
        format!("#!/bin/sh\necho \"$1\" > '{}'\n", opened.display()),
    )
    .unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&editor, fs::Permissions::from_mode(0o755)).unwrap();
    }
    let child = gp(home.path())
        .env("GP_PIXELORAMA", &editor)
        .args([
            "--json",
            "assets",
            "open",
            "mosshop",
            "player",
            "--editor",
            "pixelorama",
            "--run",
            "live",
            "--once",
            "--timeout",
            "120",
            "--poll-ms",
            "100",
        ])
        .arg("--work")
        .arg(&work)
        .arg("--assets")
        .arg(&lifted)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let out_dir = work.join("mosshop/exchange/player");
    let t0 = std::time::Instant::now();
    while !(out_dir.join("gp-edit.json").is_file() && opened.is_file()) {
        assert!(t0.elapsed().as_secs() < 60, "no export");
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
    let sheet = fs::read_to_string(&opened).unwrap();
    assert!(sheet.trim().ends_with("sheet.png"), "{sheet}");
    assert!(
        out_dir.join("palette.json").is_file(),
        "Pixelorama's palette"
    );

    // The person recolors the cap in the editor and saves the sheet.
    let side: gp_imagefmt::sheet::Sidecar =
        serde_json::from_slice(&fs::read(out_dir.join("sprites.json")).unwrap()).unwrap();
    let palette: [[u8; 3]; 16] =
        std::array::from_fn(|i| gp_imagefmt::palette::parse_hex(&side.palette[i]).unwrap());
    let got = gp_imagefmt::sheet::read(&|n| fs::read(out_dir.join(n)).ok(), None).unwrap();
    assert!(got.problems.is_empty(), "{:?}", got.problems);
    let mut frames = got.frames.clone();
    for f in &mut frames {
        // The cap is the top five rows, or the bottom five of the dying
        // frame, which shows the same tiles upside down.
        let rows: Vec<u32> = if f.state == "dying" {
            (f.height - 5..f.height).collect()
        } else {
            (0..5).collect()
        };
        for y in rows {
            for x in 0..f.width {
                let i = (y * f.width + x) as usize;
                if f.px[i] % 4 == 2 {
                    f.px[i] += 1;
                }
            }
        }
    }
    std::thread::sleep(std::time::Duration::from_millis(300));
    for (name, bytes) in gp_imagefmt::sheet::write(&side.config, "mosshop", &frames, &palette) {
        if name.ends_with(".png") {
            fs::write(out_dir.join(name), bytes).unwrap();
        }
    }
    let out = child.wait_with_output().unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let j: Value = serde_json::from_slice(&out.stdout).unwrap();
    let save = &j["saves"][0];
    assert_eq!(save["refused"], false, "{j}");
    assert_eq!(save["result"]["status"], "proposed", "{j}");
    assert!(
        save["result"]["pixels_changed"].as_u64().unwrap() > 0,
        "{j}"
    );
    let item = save["result"]["item"].as_str().unwrap().to_owned();

    // Accepted in the run: placed, with the editor as its provenance.
    let a = gp(home.path())
        .args([
            "--json", "capture", "input", "live", "--kind", "accept", "--item", &item,
        ])
        .output()
        .unwrap();
    assert!(a.status.success(), "{}", String::from_utf8_lossy(&a.stderr));
    let d: Value = serde_json::from_slice(&a.stdout).unwrap();
    assert_eq!(d["decision"]["status"], "accepted", "{d}");
    assert_eq!(d["decision"]["state"]["origin"], "uploaded", "{d}");
    assert!(
        d["decision"]["state"]["generator"]
            .as_str()
            .unwrap()
            .starts_with("pixelorama"),
        "{d}"
    );
}

/// An agent drives a watch session on Moss Hop's practice variant with the
/// bump handler in UxROM bank 1 (built by scripts/mosshop/practice_variant.py,
/// so the test needs python3 and cc65 like just check): it starts the
/// session, follows its events until a run reaches `sub_01_8000` past the
/// stop the reader left in front of it, asks for that frame's instructions
/// and gets what `gp reach --detail` gives for the same window, hooks
/// `01:8000` and sees the hook's calls, points at the screen, and finds
/// each of its calls logged in the session for the person's page.
#[test]
#[ignore = "needs `gp reach`, which comes with wave 3b"]
fn an_agent_follows_a_watch_session_to_bank_one() {
    let home = tempfile::tempdir().unwrap();
    let tmp = tempfile::tempdir().unwrap();
    let variant = tmp.path().join("pv-bump");
    let built = Command::new("python3")
        .arg(repo().join("scripts/mosshop/practice_variant.py"))
        .args(["pv-bump", "--patch"])
        .arg(repo().join("scripts/mosshop/variants/uxrom-bump-in-bank1.patch"))
        .arg("--cfg")
        .arg(repo().join("scripts/mosshop/variants/uxrom.cfg"))
        .arg("--out")
        .arg(&variant)
        .output()
        .unwrap();
    assert!(
        built.status.success(),
        "the bump variant didn't build; this test needs python3 and cc65:\n{}",
        String::from_utf8_lossy(&built.stderr)
    );
    let rom = variant.join("mosshop.nes");
    let mut c = connect_bare(home.path(), &tmp.path().join("work"));

    let (r, err) = c.call(
        "watch_start",
        json!({"name": "bump", "rom": rom, "budget": 20, "seed": 1, "start_runs": 1, "start_frames": 600}),
    );
    assert!(!err, "{r}");
    let mut after = 0u64;
    let mut events: Vec<Value> = Vec::new();
    loop {
        let (r, err) = c.call(
            "watch_events",
            json!({"name": "bump", "after": after, "wait_ms": 30000}),
        );
        assert!(!err, "{r}");
        events.extend(r["events"].as_array().unwrap().iter().cloned());
        after = r["last_seq"].as_u64().unwrap();
        if events.iter().any(|e| e["kind"] == "watch.end") {
            break;
        }
        assert!(
            r["writing"] == true || !r["events"].as_array().unwrap().is_empty(),
            "the writer stopped before watch.end: {}",
            c.transcript.join("\n")
        );
    }
    for (i, e) in events.iter().enumerate() {
        assert_eq!(e["seq"], i as u64 + 1, "seq has no gaps");
    }
    let bank = events
        .iter()
        .position(|e| e["kind"] == "bank" && e["data"]["bank"] == 1)
        .expect("bank 1's first code");
    let stop = events
        .iter()
        .position(|e| e["kind"] == "stop.reached" && e["data"]["routine"] == "sub_01_8000")
        .expect("a run reaches sub_01_8000 past the stop");
    assert!(bank < stop, "bank 1's code comes before the stop's report");
    let (run, frame) = (
        events[stop]["run"].as_str().unwrap().to_owned(),
        events[stop]["frame"].as_u64().unwrap(),
    );

    // The frame's instructions, as gp reach --detail gives them.
    let window = format!("{frame}..={frame}");
    let (d, err) = c.call(
        "watch_detail",
        json!({"name": "bump", "recipe": run, "frames": window, "max_lines": 1000000}),
    );
    assert!(!err, "{d}");
    let start = events[0]["data"]["starts"]
        .as_array()
        .unwrap()
        .iter()
        .find(|s| s["recipe"] == run.as_str())
        .unwrap()["id"]
        .as_str()
        .unwrap()
        .to_owned();
    let reach = gp(home.path())
        .args(["--json", "reach"])
        .arg(&rom)
        .args([
            "--runs",
            "1",
            "--start-frames",
            "600",
            "--no-store",
            "--detail",
            &format!("{start}:{window}"),
        ])
        .output()
        .unwrap();
    assert!(
        reach.status.success(),
        "{}",
        String::from_utf8_lossy(&reach.stderr)
    );
    let r: Value = serde_json::from_slice(&reach.stdout).unwrap();
    let r = r.get("data").cloned().unwrap_or(r);
    assert_eq!(d["trace"], r["trace"], "watch_detail is gp reach --detail");
    for k in ["start", "end", "instructions", "interrupts"] {
        assert_eq!(d[k], r[k], "{k}");
    }
    assert!(d["lines"].as_u64().unwrap() > 100);

    // A hook on bank 1's routine is called in the run that reached it.
    let (h, err) = c.call("watch_hook", json!({"name": "bump", "at": ["01:8000"]}));
    assert!(!err, "{h}");
    assert!(h["hook_hits"].as_u64().unwrap() > 0, "{h}");
    let (hooks, _) = c.call(
        "watch_events",
        json!({"name": "bump", "after": after, "kinds": ["hook"]}),
    );
    let first = &hooks["events"][0];
    assert_eq!(first["data"]["at"], "01:8000");
    assert_eq!(first["run"], run.as_str());
    assert_eq!(first["frame"], frame);

    // Pointing at the whole screen names the routines that wrote the sprites.
    let (p, err) = c.call(
        "watch_point",
        json!({"name": "bump", "recipe": run, "frame": frame, "rect": [0, 0, 256, 240]}),
    );
    assert!(!err, "{p}");
    assert!(
        p["words"]
            .as_array()
            .unwrap()
            .iter()
            .any(|w| w.as_str().unwrap().contains("written by")),
        "{p}"
    );
    let (v, _) = c.call("view_now", json!({"name": "bump"}));
    assert_eq!(v["point"]["frame"], frame, "{v}");

    // Every call is in the session's inputs, for the person's feed.
    let inputs = std::fs::read_to_string(home.path().join("watch/bump/inputs.ndjson")).unwrap();
    for tool in [
        "watch_start",
        "watch_events",
        "watch_detail",
        "watch_hook",
        "watch_point",
        "view_now",
    ] {
        assert!(
            inputs.contains(&format!("\"tool\":\"{tool}\"")),
            "{tool} isn't logged:\n{inputs}"
        );
    }
    drop(c.stdin.take());
    let _ = c.child.wait();
}
