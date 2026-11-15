//! A theme made exactly from Moss Hop's theme kit (templates/mosshop, our
//! own game) replaces every picture the kit names: `gp theme look new`
//! writes its art.txt from the kit, `gp theme look check` names the game's
//! picture for each pose, and once every one is drawn and given a rule the
//! check has nothing left to draw. The hero's standing pose is the picture
//! play draws for it, frame-1-0.png, the look of its `normal` state, where
//! the check once named frame-0-0.png, the `dying` look's (the state
//! sorted first), so a theme that followed it left the hero in the
//! original's art. Needs cc65, like just check.

use std::path::{Path, PathBuf};
use std::process::Command;

fn repo() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn gp(home: &Path, args: &[&str]) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_gp"))
        .env("GP_HOME", home)
        .env("GP_ROM_DIR", home.join("roms"))
        .env_remove("GP_STORE")
        .args(args)
        .current_dir(repo())
        .output()
        .unwrap()
}

/// A square picture of one opaque colour, `side` pixels a side.
fn square(path: &Path, side: u32) {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    let file = std::fs::File::create(path).unwrap();
    let mut enc = png::Encoder::new(std::io::BufWriter::new(file), side, side);
    enc.set_color(png::ColorType::Rgba);
    enc.set_depth(png::BitDepth::Eight);
    let mut w = enc.write_header().unwrap();
    let px = [240u8, 0, 200, 255].repeat((side * side) as usize);
    w.write_image_data(&px).unwrap();
}

/// The check's problems that say what is still to draw: each slot's
/// line, and the pictures it names, in order.
fn to_draw(home: &Path, theme: &Path) -> Vec<(String, Vec<String>)> {
    let o = gp(
        home,
        &[
            "--json",
            "theme",
            "look",
            "check",
            theme.to_str().unwrap(),
            "--game",
            "templates/mosshop",
        ],
    );
    let v: serde_json::Value = serde_json::from_slice(&o.stdout).unwrap_or_else(|e| {
        panic!(
            "{e}: {}{}",
            String::from_utf8_lossy(&o.stdout),
            String::from_utf8_lossy(&o.stderr)
        )
    });
    v["findings"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|f| f["message"].as_str())
        .filter(|m| m.starts_with("still to draw: "))
        .map(|m| {
            let pics = m
                .split('(')
                .filter_map(|p| p.split_once(".png)").map(|(n, _)| format!("{n}.png")))
                .collect();
            (m.to_owned(), pics)
        })
        .collect()
}

#[test]
fn a_theme_made_from_moss_hop_s_kit_replaces_every_picture_it_names() {
    let d = tempfile::tempdir().unwrap();
    let home = d.path().join("home");
    let built = Command::new(repo().join("scripts/mosshop/build.sh"))
        .arg(d.path())
        .output()
        .expect("running scripts/mosshop/build.sh");
    assert!(
        built.status.success(),
        "Moss Hop didn't build; this test needs cc65 (ca65 and ld65), like just check:\n{}",
        String::from_utf8_lossy(&built.stderr)
    );
    std::fs::create_dir_all(home.join("roms/nes")).unwrap();
    let rom = home.join("roms/nes/mosshop.nes");
    std::fs::copy(d.path().join("mosshop.nes"), &rom).unwrap();
    // The game's pictures, as the page has them: the assets lifted and the
    // sprite tables captured.
    let assets = home.join("work/assets/mosshop");
    let a = assets.to_str().unwrap();
    let o = gp(
        &home,
        &[
            "assets",
            "lift",
            rom.to_str().unwrap(),
            "--map",
            "assets/mosshop.toml",
            "-o",
            a,
            "--force",
        ],
    );
    assert!(o.status.success(), "{o:?}");
    let o = gp(
        &home,
        &["sprites", "capture", "templates/mosshop", "--assets", a],
    );
    assert!(o.status.success(), "{o:?}");

    let theme = d.path().join("kit-test");
    let t = theme.to_str().unwrap();
    let o = gp(
        &home,
        &[
            "theme",
            "look",
            "new",
            t,
            "--game",
            "templates/mosshop",
            "--name",
            "local/kit-test",
            "--title",
            "Kit Test",
        ],
    );
    assert!(o.status.success(), "{o:?}");

    let slots = to_draw(&home, &theme);
    assert!(!slots.is_empty(), "the kit names nothing to draw");
    let hero = slots
        .iter()
        .find(|(m, _)| m.contains("(entity.player)"))
        .unwrap_or_else(|| panic!("no hero among {slots:?}"));
    assert_eq!(hero.1, ["frame-1-0.png"], "{}", hero.0);

    // Every picture the kit's poses name, drawn and given a rule.
    let art = theme.join("theme/art.txt");
    let mut rules = std::fs::read_to_string(&art).unwrap();
    let scale: u32 = rules
        .lines()
        .find_map(|l| l.strip_prefix("scale "))
        .and_then(|s| s.trim().parse().ok())
        .unwrap();
    let mut drawn = 0;
    for (line, pics) in &slots {
        assert!(!pics.is_empty(), "the check names no picture for: {line}");
        // "..., 16 by 16 game pixels (draw at ...": the size in game pixels.
        let (w, h) = line
            .split_once(" game pixels")
            .and_then(|(before, _)| before.rsplit(", ").next())
            .and_then(|s| s.split_once(" by "))
            .map(|(w, h)| (w.parse::<u32>().unwrap(), h.parse::<u32>().unwrap()))
            .unwrap_or_else(|| panic!("no size in: {line}"));
        assert_eq!(w, h, "{line}");
        for p in pics {
            let file = format!("art/{p}");
            square(&theme.join("theme").join(&file), w * scale);
            rules.push_str(&format!("pic {p} {file} {w} {h} 0 0\n"));
            drawn += 1;
        }
    }
    std::fs::write(&art, rules).unwrap();
    let left = to_draw(&home, &theme);
    assert!(
        left.is_empty(),
        "drew {drawn} pictures, and the check still asks for: {left:?}"
    );
}
