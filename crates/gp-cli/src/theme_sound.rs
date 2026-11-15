//! `gp theme sound`: the agent proposes a change to how the person's copy
//! sounds, and the person accepts or rejects it on their page (issue #4),
//! as `gp theme picture` does for a theme's pictures. The change is the
//! one the MCP tool `sound_edit` makes (its ops, checked the same way),
//! and `sound_edit` proposes through the same store by default
//! (gp_mcp::sound_proposal, which lists the files). Nothing in the copy
//! changes until the person's ACCEPT, and an accepted change can be put
//! back. The page answers through gp serve's `/api/sounds` routes
//! (serve/sounds.rs); `gp_run` refuses accept, reject and undo: only the
//! person decides (gp_mcp::policy).
//!
//! With `--role`, `propose` takes a new or changed sound for one sound role
//! of the person's own copy of a large-art theme instead (issue #30): the
//! role's file `sound/roles/<role>.wav`, or with `--take N` a further take
//! `sound/takes/<role>.<N>.wav`, which the page plays in turn with the
//! role's others. It is checked as `gp theme picture propose` checks a
//! picture, and kept in the same store as a song's change.

use anyhow::{Context, Result, anyhow};
use clap::Subcommand;
use gp_evidence::Paths;
use gp_mcp::sound_proposal::{self as store, Answer};

use crate::Outcome;

// Propose's fields are a command's flags, parsed once.
#[allow(clippy::large_enum_variant)]
#[derive(Subcommand)]
pub enum SoundCmd {
    /// Propose a change to how one of the person's copies sounds (a copy
    /// under $GP_HOME/work/mcp/, as template_fork makes): sound_edit's
    /// ops, checked first (every song an op names must be ready, and
    /// nothing is clamped). The clip song is rendered as it sounds now and
    /// with the change, and the proposal waits on the person's page as a
    /// card that plays both, with ACCEPT and REJECT; nothing in the copy
    /// changes until they accept. Writes only the proposal, under
    /// $GP_HOME/work/proposals/sounds/. Exits 1, with why, when a check
    /// refuses it. Example: gp theme sound propose my_hop --ops
    /// '[{"op":"transpose","song":"theme","semitones":-12}]' --why "A
    /// lower tune, as you asked."
    ///
    /// With --role, a new sound for one sound role of the person's own copy
    /// of a large-art theme (a theme folder under $GP_HOME/work/themes/),
    /// in place of --ops: the role's file (sound/roles/ROLE.wav), or with
    /// --take N a further take (sound/takes/ROLE.N.wav, which the page
    /// plays in turn with the role's others), its look.txt line added when
    /// the look hasn't got one. Checked first: a 16-bit PCM WAV the page
    /// plays, a role the game's kit has, takes numbered with no gap, and
    /// `gp theme look check` on the theme with it in place finding no
    /// problem the theme didn't have (the ROM-data check among them). The
    /// card plays the role's sound as it is and the new one. Example: gp
    /// theme sound propose ~/.gameprinter/work/themes/clay --game
    /// donkey-kong --role jump --take 2 --wav /tmp/jump-2.wav --why "A
    /// second jump, so it varies."
    Propose {
        /// The copy's name (a folder under $GP_HOME/work/mcp/); with
        /// --role, the theme's folder under $GP_HOME/work/themes/.
        copy: String,
        /// The ops as a JSON list, as sound_edit takes them: remap, role,
        /// transpose, tempo, mute, solo, gain, swap. Give --ops or --role.
        #[arg(long)]
        ops: Option<String>,
        /// A sound role of the theme's game (its kit's [[sound]]), whose
        /// sound --wav proposes.
        #[arg(long, conflicts_with = "ops", requires_all = ["wav", "game"])]
        role: Option<String>,
        /// With --role: the new sound, a WAV file you made.
        #[arg(long, requires = "role")]
        wav: Option<std::path::PathBuf>,
        /// With --role: which take, 1 (the default) for the role's own
        /// file, 2 and up for a further take.
        #[arg(long, requires = "role")]
        take: Option<u32>,
        /// With --role: the game the theme is for, its folder or its name
        /// in the games checkout.
        #[arg(long, requires = "role")]
        game: Option<std::path::PathBuf>,
        /// With --role: your ROM of the game, for the ROM-data check
        /// (default: one under $GP_ROM_DIR the game accepts).
        #[arg(long, requires = "role")]
        rom: Option<std::path::PathBuf>,
        /// One sentence for the person: why this change.
        #[arg(long)]
        why: String,
        /// The sound slot whose songs the ops change (default: the first).
        #[arg(long)]
        slot: Option<String>,
        /// A theme of the copy (`<copy>/themes/<id>/`) whose restyled-audio
        /// layer takes the mapping, instead of the copy's sound/banks.toml.
        #[arg(long)]
        theme: Option<String>,
        /// The song the two clips play (default: the first song an op
        /// names, else the slot's first music).
        #[arg(long)]
        clip_song: Option<String>,
        /// Seconds of each clip (default 8, at most 60).
        #[arg(long)]
        seconds: Option<f64>,
    },
    /// The proposals and the person's answers: open ones still waiting,
    /// then the ones answered (accepted, rejected with their words, or put
    /// back), newest first. Reading an answer here marks it told, so
    /// `next` stops showing it. Example: gp theme sound list
    List {
        /// Only this copy's.
        #[arg(long)]
        copy: Option<String>,
        /// Every proposal, not only the open ones and the answers from the
        /// last day.
        #[arg(long)]
        all: bool,
    },
    /// The person's ACCEPT, from a terminal: write the change into the
    /// copy, keeping what it had for `undo`. The page's ACCEPT does the
    /// same. Refused to an agent. Example: gp theme sound accept
    /// 1763140000000-1a2b3c4d
    Accept {
        /// The proposal's id.
        id: String,
        /// Words for the agent, if any.
        #[arg(long)]
        words: Option<String>,
    },
    /// The person's REJECT, from a terminal: drop the proposal, with what
    /// to change for the agent. Refused to an agent. Example: gp theme
    /// sound reject 1763140000000-1a2b3c4d --words "Keep it as fast."
    Reject {
        /// The proposal's id.
        id: String,
        /// What the agent should change.
        #[arg(long)]
        words: Option<String>,
    },
    /// Put the old sound back after an ACCEPT (the page's PUT THE OLD ONE
    /// BACK). Refused to an agent. Example: gp theme sound undo
    /// 1763140000000-1a2b3c4d
    Undo {
        /// The proposal's id.
        id: String,
    },
}

/// The MCP tools' settings for the person's home, as gp serve's buttons
/// use them (serve/jobs.rs): copies under $GP_HOME/work/mcp/.
fn config(paths: &Paths) -> Result<gp_mcp::Config> {
    let work = paths.home.join("work").join("mcp");
    let cwd = std::env::current_dir()?;
    Ok(gp_mcp::Config {
        gp: std::env::current_exe().context("finding the gp binary")?,
        repo: crate::bundle::root().unwrap_or(cwd),
        work,
        home: paths.home.clone(),
        rom: None,
        assets: None,
        games: None,
    })
}

pub(crate) fn run(cmd: &SoundCmd, paths: &Paths) -> Result<Outcome> {
    let home = &paths.home;
    let decided =
        |id: &str, a: Answer, words: Option<&str>, text: &dyn Fn(&serde_json::Value) -> String| {
            let v = store::decide(home, id, a, words).map_err(anyhow::Error::msg)?;
            let t = text(&v);
            Ok(Outcome::ok(v, t))
        };
    match cmd {
        SoundCmd::Propose {
            copy,
            role: Some(role),
            wav: Some(wav),
            game: Some(game),
            take,
            rom,
            why,
            ..
        } => propose_role(
            std::path::Path::new(copy),
            game,
            role,
            take.unwrap_or(1),
            wav,
            why,
            rom.as_deref(),
            paths,
        ),
        SoundCmd::Propose {
            copy,
            ops,
            why,
            slot,
            theme,
            clip_song,
            seconds,
            ..
        } => {
            let Some(ops) = ops else {
                return Ok(Outcome {
                    text: "Not proposed: give --ops (a change to a copy's songs) or --role with --wav and --game (a role's sound in a large-art theme).".into(),
                    json: serde_json::json!({"ok": false, "refused": "give --ops, or --role with --wav and --game"}),
                    exit: 1,
                });
            };
            let ops = serde_json::from_str(ops)
                .map_err(|e| anyhow!("--ops isn't a JSON list of sound_edit's ops: {e}"))?;
            let args = gp_mcp::sound::EditArgs {
                copy: copy.clone(),
                slot: slot.clone(),
                ops,
                undo: false,
                place: true,
                clip_song: clip_song.clone(),
                seconds: *seconds,
                theme: theme.clone(),
                prompt: None,
                why: Some(why.clone()),
                write: false,
                confirmed_by_person: false,
            };
            let config = config(paths)?;
            let _lock = gp_mcp::CopyLock::take(copy);
            let r = gp_mcp::sound::edit(&config, args).map_err(anyhow::Error::msg)?;
            if r.is_error {
                let why = r.json["refused"]
                    .as_str()
                    .or(r.json["error"].as_str())
                    .map(str::to_owned)
                    .unwrap_or_else(|| r.json.to_string());
                return Ok(Outcome {
                    text: format!("Not proposed: {why}"),
                    json: r.json,
                    exit: 1,
                });
            }
            let v = &r.json;
            let text = format!(
                "Proposed {}: the song \"{}\" in {} ({} of the open proposals). The change: {}. Checked: every song it names is ready to edit, and nothing was clamped.\n{}",
                v["id"].as_str().unwrap_or(""),
                v["clip_song"].as_str().unwrap_or(""),
                v["copy"].as_str().unwrap_or(""),
                v["waiting"],
                v["said"]
                    .as_array()
                    .map(|s| {
                        s.iter()
                            .filter_map(|x| x.as_str())
                            .collect::<Vec<_>>()
                            .join("; ")
                    })
                    .unwrap_or_default(),
                v["next"].as_str().unwrap_or("")
            );
            Ok(Outcome::ok(r.json, text))
        }
        SoundCmd::List { copy, all } => {
            let (v, text) = store::list(home, copy.as_deref(), *all);
            Ok(Outcome::ok(v, text))
        }
        SoundCmd::Accept { id, words } => decided(id, Answer::Accept, words.as_deref(), &|v| {
            format!(
                "Accepted: the new sound for \"{}\" is in {} now.",
                v["clip_song"]
                    .as_str()
                    .or(v["role"].as_str())
                    .unwrap_or("sound"),
                v["copy"].as_str().unwrap_or("your copy")
            )
        }),
        SoundCmd::Reject { id, words } => decided(id, Answer::Reject, words.as_deref(), &|_| {
            "Rejected; your agent hears your words.".into()
        }),
        SoundCmd::Undo { id } => decided(id, Answer::Undo, None, &|v| {
            format!(
                "The old sound for \"{}\" is back.",
                v["clip_song"]
                    .as_str()
                    .or(v["role"].as_str())
                    .unwrap_or("sound")
            )
        }),
    }
}

/// The largest WAV a proposal takes.
const MAX_WAV: u64 = 16 << 20;

fn refused(why: impl Into<String>) -> Outcome {
    let why = why.into();
    Outcome {
        text: format!("Not proposed: {why}"),
        json: serde_json::json!({"ok": false, "refused": why}),
        exit: 1,
    }
}

/// `propose --role`: a role's sound for the person's own copy of a
/// large-art theme, checked, then kept as a proposal.
#[allow(clippy::too_many_arguments)]
fn propose_role(
    dir_arg: &std::path::Path,
    game: &std::path::Path,
    role: &str,
    take: u32,
    wav_path: &std::path::Path,
    why: &str,
    rom: Option<&std::path::Path>,
    paths: &Paths,
) -> Result<Outcome> {
    use gp_present::looks::{LOOK_FILE, Look, take_file};
    use std::fs;
    let home = &paths.home;
    let why = why.trim();
    if why.is_empty() {
        return Ok(refused(
            "--why is empty. Say in one sentence why this sound, for the person.",
        ));
    }
    if why.len() > 400 {
        return Ok(refused(
            "--why is longer than one sentence (400 characters at most).",
        ));
    }
    if take == 0 {
        return Ok(refused(
            "--take is 1 for the role's own file, 2 and up for a further take.",
        ));
    }
    // The person's own copy, nowhere else.
    let Ok(dir) = dir_arg.canonicalize() else {
        anyhow::bail!("{} isn't a folder", dir_arg.display());
    };
    let own = crate::theme_picture::own_themes(home);
    let own = own.canonicalize().unwrap_or(own);
    if !dir.starts_with(&own) || dir == own {
        return Ok(refused(format!(
            "{} isn't the person's own copy of a theme. Proposals change only a theme folder under {}: copy the theme there first and propose on the copy.",
            dir.display(),
            own.display()
        )));
    }
    let (root, theme) = crate::theme_picture::theme_folders(&dir);
    let look_text = fs::read_to_string(theme.join(LOOK_FILE)).map_err(|_| {
        anyhow!(
            "{} has no theme/look.txt: it isn't a large-art theme",
            root.display()
        )
    })?;
    let look = Look::read(&look_text).map_err(|e| anyhow!("look.txt: {e}"))?;

    // A role the game has.
    let game_dir = crate::theme_kit::template_dir(game)?;
    let kit_file = game_dir
        .join("themes")
        .join("kit")
        .join(crate::theme_kit::KIT_FILE);
    let Ok(kit_text) = fs::read_to_string(&kit_file) else {
        return Ok(refused(format!(
            "{} has no theme kit ({}), so its sound roles aren't known. Make one with `gp theme kit {}`.",
            game_dir.display(),
            kit_file.display(),
            game_dir.display()
        )));
    };
    let kit: crate::theme_kit::Kit =
        toml::from_str(&kit_text).with_context(|| format!("reading {}", kit_file.display()))?;
    if !kit.sound.iter().any(|s| s.role == role) {
        let names: Vec<&str> = kit.sound.iter().map(|s| s.role.as_str()).collect();
        return Ok(refused(format!(
            "the game has no sound role `{role}`{}.",
            if names.is_empty() {
                " (it has none)".to_owned()
            } else {
                format!("; its roles are {}", names.join(", "))
            }
        )));
    }
    // Takes in order: a further take follows the role's file and the take
    // before it.
    let have = look.role_sounds().remove(role).unwrap_or_default();
    let has = |n: u32| have.iter().any(|(t, _)| *t == n);
    if take > 1 && !(1..take).all(has) {
        let next = (1..).find(|n| !has(*n)).unwrap_or(1);
        return Ok(refused(format!(
            "`{role}` has no take {next} yet, and takes go in order with no gap (a page from before takes plays only {}). Propose take {next} first.",
            take_file(role, 1)
        )));
    }
    let file = take_file(role, take);

    // The new sound.
    let meta = fs::metadata(wav_path).with_context(|| format!("reading {}", wav_path.display()))?;
    if meta.len() > MAX_WAV {
        return Ok(refused(format!(
            "{} is {} bytes; a sound here is at most 16 MB.",
            wav_path.display(),
            meta.len()
        )));
    }
    let new = fs::read(wav_path).with_context(|| format!("reading {}", wav_path.display()))?;
    let samples = match gp_present::sound::read_wav(&new) {
        Ok(s) if !s.is_empty() => s,
        Ok(_) => return Ok(refused(format!("{} holds no sound.", wav_path.display()))),
        Err(e) => {
            return Ok(refused(format!(
                "{} isn't a sound the page plays ({e}). Save it as a 16-bit PCM WAV.",
                wav_path.display()
            )));
        }
    };
    let old = fs::read(theme.join(&file)).ok();
    let old_seconds = old
        .as_deref()
        .and_then(|b| gp_present::sound::read_wav(b).ok())
        .map_or(0.0, |s| {
            s.len() as f64 / f64::from(gp_present::sound::ROLE_RATE)
        });
    let new_look = if look.has(&file) {
        look_text.clone()
    } else {
        let mut t = look_text.clone();
        if !t.ends_with('\n') {
            t.push('\n');
        }
        t.push_str(&format!("file {file}\n"));
        t
    };
    if old.as_deref() == Some(new.as_slice()) && new_look == look_text {
        return Ok(refused(format!(
            "the new sound is the same as {file}, byte for byte."
        )));
    }

    // The theme check, before and with the sound in place.
    let before = crate::theme_look::check(&root, game, rom, paths)?;
    let scratch_parent = crate::theme_picture::dir_for_checks(home)?;
    let scratch = tempfile::Builder::new()
        .prefix("check-")
        .tempdir_in(&scratch_parent)
        .context("making a folder to check the theme in")?;
    let copy = scratch
        .path()
        .join(root.file_name().unwrap_or_else(|| "theme".as_ref()));
    crate::theme_picture::copy_tree(&root, &copy).context("copying the theme to check it")?;
    let copy_theme = copy.join(
        theme
            .strip_prefix(&root)
            .unwrap_or(std::path::Path::new("theme")),
    );
    let at = copy_theme.join(&file);
    fs::create_dir_all(at.parent().unwrap_or(&copy_theme))?;
    fs::write(&at, &new).context("putting the sound in the copy")?;
    fs::write(copy_theme.join(LOOK_FILE), &new_look).context("putting look.txt in the copy")?;
    let after = crate::theme_look::check(&copy, game, rom, paths)?;
    let had = crate::theme_picture::problems(&before);
    let new_problems: Vec<(String, String, String)> = crate::theme_picture::problems(&after)
        .into_iter()
        .filter(|p| !had.contains(p))
        .collect();
    if let Some(first) = new_problems.first() {
        let lead = if new_problems.iter().any(|p| p.0 == "rom_data") {
            "the sound holds data from the ROM, and a theme may hold nothing from it"
        } else {
            "`gp theme look check` finds a problem with the sound in place that the theme didn't have"
        };
        let mut o = refused(format!(
            "{lead}: {}{}. Fix it and propose again.",
            if first.1.is_empty() {
                String::new()
            } else {
                format!("{}: ", first.1)
            },
            first.2
        ));
        o.json["new_problems"] = serde_json::json!(
            new_problems
                .iter()
                .map(|p| serde_json::json!({"group": p.0, "file": p.1, "message": p.2}))
                .collect::<Vec<_>>()
        );
        return Ok(o);
    }
    let rom_note = after.json["findings"]
        .as_array()
        .into_iter()
        .flatten()
        .find(|f| f["group"] == serde_json::json!("rom_data"))
        .map(|f| {
            if f["level"] == serde_json::json!("ok") {
                "compared with the ROM: holds none of its data".to_owned()
            } else {
                format!(
                    "not compared with the ROM: {}",
                    f["message"].as_str().unwrap_or("")
                )
            }
        })
        .unwrap_or_else(|| "not compared with the ROM".into());
    drop(scratch);

    let game_name = game_dir.file_name().map_or_else(
        || game.display().to_string(),
        |n| n.to_string_lossy().into_owned(),
    );
    let seconds = samples.len() as f64 / f64::from(gp_present::sound::ROLE_RATE);
    let v = store::propose_role(
        home,
        &store::RoleSound {
            theme: &dir,
            theme_dir: &theme,
            title: &look.title,
            game: &game_name,
            role,
            take,
            new: &new,
            new_look: &new_look,
            why,
            seconds,
            old_seconds,
            checked: serde_json::json!({
                "problems_before": had.len(),
                "problems_after": crate::theme_picture::problems(&after).len(),
                "rom": rom_note,
            }),
        },
    )
    .map_err(anyhow::Error::msg)?;
    let out = store::proposed_role(home, &v);
    let text = format!(
        "Proposed {}: {} in {} ({} of the open proposals), {:.2} s. Checked: a WAV the page plays, a sound role the game has, the theme check finds no new problem, {rom_note}.\n{}",
        v["id"].as_str().unwrap_or(""),
        if take == 1 {
            format!("the sound for `{role}` ({file})")
        } else {
            format!("take {take} of `{role}` ({file})")
        },
        look.title,
        out["waiting"],
        seconds,
        out["next"].as_str().unwrap_or("")
    );
    Ok(Outcome::ok(out, text))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn answers_without_a_proposal_are_refused_in_words() {
        let d = tempfile::tempdir().unwrap();
        let paths = Paths {
            home: d.path().join("home"),
            ..Paths::from_env().unwrap()
        };
        let e = run(
            &SoundCmd::Accept {
                id: "1-abcdef01".into(),
                words: None,
            },
            &paths,
        )
        .err()
        .unwrap();
        assert!(e.to_string().contains("no sound proposal"), "{e}");
        let l = run(
            &SoundCmd::List {
                copy: None,
                all: false,
            },
            &paths,
        )
        .unwrap();
        assert_eq!(l.text, "No sound proposals.");
    }

    /// A role's sound proposed for a made-up large-art theme of a copy of
    /// Moss Hop whose kit is given two made-up sound roles (Moss Hop has
    /// none of its own): take 3 refused before take 2, a role the game
    /// hasn't got, a WAV the page can't play and a folder outside the
    /// person's themes refused; take 2 kept with its look.txt line and the
    /// theme unchanged until ACCEPT; ACCEPT and PUT THE OLD ONE BACK.
    /// Made-up tones, nothing from a ROM.
    #[test]
    fn a_role_s_sound_is_checked_proposed_and_accepted() {
        let d = tempfile::tempdir().unwrap();
        let paths = Paths {
            home: d.path().join("home"),
            ..Paths::from_env().unwrap()
        };
        let game = d.path().join("mosshop");
        let repo = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../templates/mosshop");
        crate::theme_picture::copy_tree(&repo, &game).unwrap();
        let kit = game.join("themes/kit/kit.toml");
        let mut k = std::fs::read_to_string(&kit)
            .unwrap()
            .replace("\nsound = []\n", "\n");
        k.push_str("\n[[sound]]\nrole = \"hop\"\nkind = \"sfx\"\ncue = \"made up\"\nloops = false\ndescription = \"made up\"\n\n[[sound]]\nrole = \"tune\"\nkind = \"music\"\ncue = \"made up\"\nloops = true\ndescription = \"made up\"\n");
        std::fs::write(&kit, k).unwrap();
        let root = paths.home.join("work/themes/bright");
        let theme = root.join("theme");
        std::fs::create_dir_all(theme.join("sound/roles")).unwrap();
        std::fs::write(
            root.join("package.toml"),
            "[package]\nname = \"someone/bright\"\nversion = \"0.1.0\"\nkind = \"content\"\ntitle = \"Bright\"\nlicense = \"CC-BY-4.0\"\n",
        )
        .unwrap();
        std::fs::write(theme.join("art.txt"), "gp-art 1\nscale 2\n").unwrap();
        let look = "gp-look 1\nid bright\ntitle Bright\nart art.txt\nfile sound/roles/hop.wav\n";
        std::fs::write(theme.join("look.txt"), look).unwrap();
        let tone = |period: usize| {
            let s: Vec<i16> = (0..600)
                .map(|i| {
                    if (i / period).is_multiple_of(2) {
                        3000
                    } else {
                        -3000
                    }
                })
                .collect();
            gp_theme::tune::wav(&s, gp_present::sound::ROLE_RATE)
        };
        std::fs::write(theme.join("sound/roles/hop.wav"), tone(20)).unwrap();
        let wav = d.path().join("hop-2.wav");
        std::fs::write(&wav, tone(33)).unwrap();
        let propose = |dir: &std::path::Path, role: &str, take: u32, wav: &std::path::Path| {
            run(
                &SoundCmd::Propose {
                    copy: dir.display().to_string(),
                    ops: None,
                    role: Some(role.into()),
                    wav: Some(wav.to_owned()),
                    take: Some(take),
                    game: Some(game.clone()),
                    rom: None,
                    why: "A second hop, so it varies.".into(),
                    slot: None,
                    theme: None,
                    clip_song: None,
                    seconds: None,
                },
                &paths,
            )
            .unwrap()
        };
        let o = propose(&root, "hop", 3, &wav);
        assert!(
            o.exit == 1 && o.text.contains("has no take 2 yet"),
            "{}",
            o.text
        );
        let o = propose(&root, "boom", 1, &wav);
        assert!(
            o.text
                .contains("no sound role `boom`; its roles are hop, tune"),
            "{}",
            o.text
        );
        let bad = d.path().join("bad.wav");
        std::fs::write(&bad, b"RIFF\0\0\0\0WAVEfmt ").unwrap();
        let o = propose(&root, "hop", 2, &bad);
        assert!(
            o.text.contains("isn't a sound the page plays"),
            "{}",
            o.text
        );
        let outside = d.path().join("elsewhere");
        std::fs::create_dir_all(&outside).unwrap();
        let o = propose(&outside, "hop", 2, &wav);
        assert!(
            o.text.contains("isn't the person's own copy of a theme"),
            "{}",
            o.text
        );
        let o = propose(&root, "hop", 2, &wav);
        assert_eq!(o.exit, 0, "{}", o.text);
        assert!(
            o.text
                .contains("take 2 of `hop` (sound/takes/hop.2.wav) in Bright"),
            "{}",
            o.text
        );
        let id = o.json["id"].as_str().unwrap().to_owned();
        assert!(!theme.join("sound/takes/hop.2.wav").exists());
        assert_eq!(
            std::fs::read_to_string(theme.join("look.txt")).unwrap(),
            look
        );
        let a = run(
            &SoundCmd::Accept {
                id: id.clone(),
                words: None,
            },
            &paths,
        )
        .unwrap();
        assert!(a.text.contains("the new sound for \"hop\""), "{}", a.text);
        assert_eq!(
            std::fs::read(theme.join("sound/takes/hop.2.wav")).unwrap(),
            tone(33)
        );
        assert!(
            std::fs::read_to_string(theme.join("look.txt"))
                .unwrap()
                .ends_with("file sound/takes/hop.2.wav\n")
        );
        run(&SoundCmd::Undo { id }, &paths).unwrap();
        assert!(!theme.join("sound/takes/hop.2.wav").exists());
        assert_eq!(
            std::fs::read_to_string(theme.join("look.txt")).unwrap(),
            look
        );
    }
}
