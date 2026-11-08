//! Stacking on made-up themes.

use std::path::Path;

use super::*;
use crate::format::sha256_hex;

fn theme(dir: &Path, id: &str, body: &str) {
    let d = dir.join(id);
    std::fs::create_dir_all(&d).unwrap();
    std::fs::write(
        d.join("theme.toml"),
        format!(
            "format = \"0.1.0\"\nid = \"{id}\"\ntitle = \"{id}\"\nlicense = \"CC0-1.0\"\n{body}"
        ),
    )
    .unwrap();
}

fn stack(layers: &[(&str, &str)]) -> Stack {
    let mut text = "format = \"0.1.0\"\n".to_string();
    for (t, s) in layers {
        text.push_str(&format!("[[layer]]\ntheme = \"{t}\"\nscope = \"{s}\"\n"));
    }
    Stack::parse(&text).unwrap()
}

#[test]
fn baked_layers_go_under_personal_ones() {
    let tmp = tempfile::tempdir().unwrap();
    let (baked, personal) = (tmp.path().join("b"), tmp.path().join("p"));
    theme(
        &baked,
        "warm",
        "[[palette]]\ntarget = \"*\"\nreplace = [[\"#000000\", \"#ff0000\"]]\n",
    );
    theme(
        &personal,
        "mono",
        "[[palette]]\ntarget = \"*\"\nreplace = [[\"#ff0000\", \"#ffffff\"]]\n",
    );
    let places = Places {
        baked: Some(baked),
        personal: Some(personal),
    };
    // Written personal first, resolved baked first.
    let r = resolve(
        &stack(&[("mono", "personal"), ("warm", "baked")]),
        &places,
        "m",
    )
    .unwrap();
    assert_eq!(r.layers[0].theme.id, "warm");
    assert_eq!(
        r.color(Some("player"), 0, Rgb([0, 0, 0])),
        Rgb([255, 255, 255])
    );
    // A baked layer isn't looked for among the player's themes.
    let e = resolve(&stack(&[("mono", "baked")]), &places, "m").unwrap_err();
    assert!(e.contains("no baked theme `mono`"), "{e}");
    let e = resolve(
        &stack(&[("warm", "baked"), ("warm", "personal")]),
        &places,
        "m",
    )
    .unwrap_err();
    assert!(e.contains("twice"), "{e}");
}

#[test]
fn the_most_specific_rule_in_a_layer_wins() {
    let tmp = tempfile::tempdir().unwrap();
    theme(
        tmp.path(),
        "t",
        "[[palette]]\ntarget = \"*\"\ntint = \"#0000ff\"\ntint_amount = 1.0\n\
         [[palette]]\ntarget = \"actors\"\ntint = \"#00ff00\"\ntint_amount = 1.0\n\
         [[palette]]\ntarget = \"player\"\ntint = \"#ff0000\"\ntint_amount = 1.0\n",
    );
    let places = Places {
        baked: Some(tmp.path().into()),
        personal: None,
    };
    let r = resolve(&stack(&[("t", "baked")]), &places, "m").unwrap();
    let c = Rgb([9, 9, 9]);
    assert_eq!(r.color(Some("player"), 0, c), Rgb([255, 0, 0]));
    assert_eq!(r.color(Some("enemy"), 0, c), Rgb([0, 255, 0]));
    assert_eq!(r.color(Some("sky"), 0, c), Rgb([0, 0, 255]));
    assert_eq!(r.color(None, 0, c), Rgb([0, 0, 255]));
    // A restyled pixel skips the layers at and under its restyle.
    assert_eq!(r.color(Some("player"), 1, c), c);
}

#[test]
fn the_highest_layer_wins_effects_and_restyles() {
    let tmp = tempfile::tempdir().unwrap();
    let art = "pix 1\nsize 1 1\na #102030\na\n";
    for id in ["low", "high"] {
        theme(
            tmp.path(),
            id,
            &format!(
                "[[effect]]\nkind = \"scanlines\"\nstrength = {}\n\
                 [[restyle]]\ntarget = \"player\"\nfile = \"p.pix\"\norigin = \"generated\"\nlicense = \"CC0-1.0\"\nsha256 = \"{}\"\n",
                if id == "low" { "0.2" } else { "0.6" },
                sha256_hex(art.as_bytes())
            ),
        );
        std::fs::write(tmp.path().join(id).join("p.pix"), art).unwrap();
    }
    theme(
        tmp.path(),
        "fill",
        &format!(
            "[[restyle]]\ntarget = \"enemy\"\nfile = \"e.pix\"\norigin = \"generated\"\nlicense = \"CC0-1.0\"\nsha256 = \"{}\"\n",
            sha256_hex(art.as_bytes())
        ),
    );
    std::fs::write(tmp.path().join("fill").join("e.pix"), art).unwrap();
    let places = Places {
        baked: Some(tmp.path().into()),
        personal: None,
    };
    let r = resolve(
        &stack(&[("fill", "baked"), ("low", "baked"), ("high", "baked")]),
        &places,
        "m",
    )
    .unwrap();
    let fx = r.effects();
    assert_eq!(fx.len(), 1);
    assert_eq!(fx[0], &Effect::Scanlines { strength: 0.6 });
    assert_eq!(
        r.restyle_role("player").unwrap().0,
        2,
        "the top layer's player"
    );
    assert_eq!(
        r.restyle_role("enemy").unwrap().0,
        0,
        "a lower layer fills the gap"
    );
    assert!(r.restyle_role("boss").is_none());
    let o = r.overlay(4, 4).unwrap();
    assert_eq!(o.get(0, 0)[3], 0);
    assert!(o.get(0, 1)[3] > 140);
}

#[test]
fn a_restyle_whose_file_changed_is_refused() {
    let tmp = tempfile::tempdir().unwrap();
    theme(
        tmp.path(),
        "t",
        "[[restyle]]\ntarget = \"player\"\nfile = \"p.pix\"\norigin = \"generated\"\nlicense = \"CC0-1.0\"\nsha256 = \"00\"\n",
    );
    std::fs::write(tmp.path().join("t").join("p.pix"), "pix 1\n").unwrap();
    let e = load(&tmp.path().join("t"), Scope::Baked).unwrap_err();
    assert!(e.contains("sha256"), "{e}");
    theme(
        tmp.path(),
        "u",
        "[[restyle]]\ntarget = \"player\"\nfile = \"../t/p.pix\"\norigin = \"generated\"\nlicense = \"LicenseRef-own-copy\"\nsha256 = \"00\"\n",
    );
    let e = load(&tmp.path().join("u"), Scope::Baked).unwrap_err();
    assert!(
        e.contains("inside the theme") && e.contains("own-copy"),
        "{e}"
    );
}

#[test]
fn themes_for_other_models_are_refused() {
    let tmp = tempfile::tempdir().unwrap();
    theme(tmp.path(), "t", "for = [\"other\"]\n");
    let places = Places {
        baked: Some(tmp.path().into()),
        personal: None,
    };
    assert!(
        resolve(&stack(&[("t", "baked")]), &places, "m")
            .unwrap_err()
            .contains("for other")
    );
}

#[test]
fn pictures_take_palette_rules_gradients_and_glows() {
    let tmp = tempfile::tempdir().unwrap();
    theme(
        tmp.path(),
        "t",
        "[[effect]]\nkind = \"gradient\"\ntarget = \"sky\"\ncolors = [\"#000000\", \"#ffffff\"]\n\
         [[effect]]\nkind = \"glow\"\ntarget = \"player\"\ncolor = \"#ffff00\"\n",
    );
    let places = Places {
        baked: Some(tmp.path().into()),
        personal: None,
    };
    let r = resolve(&stack(&[("t", "baked")]), &places, "m").unwrap();
    let mut img = RoleImage::new(3, 3);
    let sky = crate::role_id("sky");
    for y in 0..3 {
        img.set(0, y, [9, 9, 9, 255], sky, 0);
    }
    img.set(2, 1, [1, 2, 3, 255], crate::role_id("player"), 0);
    let out = r.apply(&img);
    assert_eq!(out.get(0, 0), [0, 0, 0, 255]);
    assert_eq!(out.get(0, 2), [255, 255, 255, 255]);
    assert_eq!(out.get(1, 1), [255, 255, 0, 160], "glow beside the player");
    assert_eq!(out.get(2, 1), [1, 2, 3, 255]);
}

#[test]
fn an_audio_restyle_names_a_sound_role_and_a_mapping() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join("banks.toml"),
        "format = \"gp-sound-banks\"\nversion = \"0.1.0\"\n\n[[map]]\nrole = \"bass\"\ntarget = { kind = \"gm\", program = 33 }\n",
    )
    .unwrap();
    let text = "format = \"0.1.1\"\nid = \"remaster\"\ntitle = \"Remaster\"\nlicense = \"CC0-1.0\"\n\n[[audio]]\ntarget = \"music\"\nbanks = \"banks.toml\"\nlicense = \"CC0-1.0\"\n";
    let t = Theme::parse(text).unwrap();
    assert!(t.check(dir.path()).is_empty(), "{:?}", t.check(dir.path()));
    assert!(t.audio[0].applies("music.level"));
    assert!(!t.audio[0].applies("sfx.jump"));
    let bad = Theme::parse(
        &text
            .replace("target = \"music\"", "target = \"player\"")
            .replace("banks.toml", "nope.toml"),
    )
    .unwrap();
    let p = bad.check(dir.path());
    assert!(p.iter().any(|(at, _)| at == "audio[0].target"), "{p:?}");
    assert!(p.iter().any(|(at, _)| at == "audio[0].banks"), "{p:?}");
}

#[test]
fn a_crt_and_a_grid_draw_three_overlay_pixels_to_a_game_pixel() {
    let tmp = tempfile::tempdir().unwrap();
    theme(
        tmp.path(),
        "tv",
        "[[effect]]\nkind = \"crt\"\nstrength = 1.0\n",
    );
    theme(
        tmp.path(),
        "lcd",
        "[[effect]]\nkind = \"grid\"\nstrength = 0.5\n",
    );
    let places = Places {
        baked: Some(tmp.path().into()),
        personal: None,
    };
    let tv = resolve(&stack(&[("tv", "baked")]), &places, "m").unwrap();
    assert_eq!(tv.overlay_scale(), 3);
    let o = tv.overlay_fine(8, 8).unwrap();
    assert_eq!((o.width, o.height), (24, 24));
    // The middle of the screen: the gap under each line is dark, the rows
    // above it carry only the grille's color.
    let (lit, gap) = (o.get(12, 12), o.get(12, 14));
    assert!(gap[3] > lit[3] + 100, "{lit:?} {gap:?}");
    let reds: Vec<[u8; 4]> = (12..15).map(|x| o.get(x, 12)).collect();
    assert!(
        reds[0][0] > reds[1][0] && reds[1][1] > reds[0][1],
        "{reds:?}"
    );
    // At the view's size the gaps fall on every other row.
    let one = tv.overlay(8, 8).unwrap();
    assert_eq!((one.width, one.height), (8, 8));
    assert!(one.get(4, 5)[3] > one.get(4, 4)[3]);
    let lcd = resolve(&stack(&[("lcd", "baked")]), &places, "m").unwrap();
    let g = lcd.overlay_fine(4, 4).unwrap();
    assert_eq!(g.get(0, 0)[3], 0);
    assert!(g.get(2, 0)[3] > 100 && g.get(0, 2)[3] > 100);
    // Scanlines alone stay at the view's size.
    theme(
        tmp.path(),
        "lines",
        "[[effect]]\nkind = \"scanlines\"\nstrength = 0.5\n",
    );
    let l = resolve(&stack(&[("lines", "baked")]), &places, "m").unwrap();
    assert_eq!(l.overlay_scale(), 1);
}
