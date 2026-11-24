use super::*;
use crate::view::Window;
use crate::{Draw, Picture};

fn present() -> Present {
    let pic = |f: &str, w, h| Picture {
        file: f.into(),
        size: [w, h],
    };
    Present {
        pictures: vec![
            pic("level.png", 256, 240),
            pic("hero.png", 16, 16),
            pic("foe.png", 16, 16),
            pic("boss.png", 48, 32),
            pic("boss-throw.png", 48, 32),
            pic("ball.png", 16, 16),
            pic("other-level.png", 256, 240),
            pic("bug.png", 16, 16),
        ],
        ..Present::default()
    }
}

fn draw(pic: u32, x: i64, y: i64) -> Option<Draw> {
    Some(Draw {
        pic,
        x,
        y,
        flip_x: false,
        flip_y: false,
    })
}

fn framed(s: Scene) -> Framed {
    Framed {
        scene: s,
        window: Window {
            x: 0,
            y: 8,
            w: 256,
            h: 224,
            hud: [0, 8],
            ..Default::default()
        },
        ..Framed::default()
    }
}

fn art(text: &str) -> Art {
    Art::new(
        ArtRules::read(&format!("gp-art 1\n{text}")).unwrap(),
        &present(),
    )
    .unwrap()
}

/// The draws of the bodies' and props' art, as (file, x, y) in pixels.
fn arts(f: &ArtFrame) -> Vec<(u32, i64, i64)> {
    f.draws
        .iter()
        .filter_map(|d| match d.src {
            Src::Art(a) => Some((a, d.x / ART_SUB, d.y / ART_SUB)),
            _ => None,
        })
        .collect()
}

#[test]
fn reads_rules() {
    let r = ArtRules::read(
        "gp-art 1\nscale 6\nlimit off\n# a note\npic hero.png a/hero.png 16 18 0 -2\n\
         timed boss.png 0 12 a/push.png 48 38 0 -5 every=60 after=ball.png wait=26 hide\n\
         face bug.png\nface_turn 4\nheld ball.png\n\
         near a/yell.png 48 38 0 -5\nnear_when boss.png\nnear_other hero.png\n\
         elsewhere level.png 0 -9 foe.png\n\
         prop level.png a/stump.png 28 200 37 18\nprop_over ball.png\nprop_burst foe.png\n\
         drift level.png 47 8 72\ndrift_art a/s.png 51 208\ndrift_at 23 0\n",
    )
    .unwrap();
    assert_eq!(r.scale, 6);
    assert!(!r.limit);
    assert_eq!(r.pics[0].art.size, [16, 18]);
    let t = &r.timed[0];
    assert_eq!(
        (t.from, t.to, t.every, t.wait, t.hide),
        (0, 12, 60, 26, true)
    );
    assert_eq!(t.after.as_deref(), Some("ball.png"));
    assert_eq!(r.face_turn, 4);
    assert_eq!(r.props[0].burst, vec!["foe.png".to_string()]);
    assert_eq!(r.drift.as_ref().unwrap().at, vec![(23, 0)]);
    assert_eq!(
        r.files(),
        [
            "a/hero.png",
            "a/push.png",
            "a/yell.png",
            "a/stump.png",
            "a/s.png"
        ]
    );
    for bad in [
        "pic a.png ../x.png 1 1 0 0",
        "scale 0",
        "near_when a.png",
        "timed a.png 0 1 b.png 1 1 0 0 soon",
        "what",
    ] {
        assert!(
            ArtRules::read(&format!("gp-art 1\n{bad}")).is_err(),
            "{bad}"
        );
    }
    assert!(ArtRules::read("gp-look 1").is_err());
}

#[test]
fn a_rule_for_a_missing_picture_is_left_out_but_a_look_for_another_build_is_refused() {
    // One dump of a game can lack a picture another has (a glyph): the rest
    // of the look still draws.
    let r = ArtRules::read(
        "gp-art 1\npic nope.png a.png 16 16 0 0\npic hero.png h.png 16 16 0 0\nheld nope.png",
    )
    .unwrap();
    let a = Art::new(r, &present()).unwrap();
    assert!(a.pics[1].is_some());
    // None of its pictures: a look made for another game.
    let r = ArtRules::read("gp-art 1\npic nope.png a.png 16 16 0 0").unwrap();
    assert!(Art::new(r, &present()).is_err());
}

#[test]
fn replaced_pictures_draw_as_art_and_the_rest_as_the_builds() {
    let p = present();
    let a = art("scale 6\npic hero.png a/hero.png 16 18 0 -2");
    let mut f = framed(Scene {
        level: Some(0),
        bodies: vec![draw(1, 40, 100), draw(2, 80, 100)],
        cut: vec![vec![], vec![[0, 0, 8, 16]]],
        ..Scene::default()
    });
    f.alpha = vec![255, 128];
    f.scene.bodies[0].as_mut().unwrap().flip_x = true;
    let out = Painter::new().frame(&a, &p, &f, 224);
    assert_eq!(out.scale, 6);
    assert_eq!(out.draws.len(), 3);
    assert_eq!(out.draws[0].src, Src::Own(0));
    assert_eq!(out.draws[0].y, -8 * ART_SUB);
    // The hero: its art, at its size and offset, drawn whole.
    let h = &out.draws[1];
    assert_eq!(h.src, Src::Art(0));
    assert_eq!(
        [h.x, h.y, h.w, h.h],
        [40 * ART_SUB, 90 * ART_SUB, 16 * ART_SUB, 18 * ART_SUB]
    );
    assert!(h.flip_x);
    assert!(h.cut.is_empty());
    // The foe: the build's picture, its cut and alpha kept.
    let e = &out.draws[2];
    assert_eq!(e.src, Src::Own(2));
    assert_eq!(e.alpha, 128);
    assert_eq!(e.cut, vec![[0, 0, 8, 16]]);
}

#[test]
fn timed_frames_follow_how_long_a_picture_has_shown() {
    let p = present();
    let a = art("pic boss.png a/stand.png 48 38 0 -5\n\
         timed boss.png 0 12 a/push.png 48 38 0 -5\n\
         timed boss.png 12 26 a/hold.png 48 38 0 -5 hide\n");
    let s = Scene {
        bodies: vec![draw(3, 10, 30)],
        ..Scene::default()
    };
    let mut pt = Painter::new();
    let mut seen = Vec::new();
    for _ in 0..30 {
        pt.step(&a, &s);
        seen.push(arts(&pt.frame(&a, &p, &framed(s.clone()), 224))[0].0);
    }
    // push for 12 frames, hold for 14, then the base picture.
    assert_eq!(seen[0], 1);
    assert_eq!(seen[11], 1);
    assert_eq!(seen[12], 2);
    assert_eq!(seen[25], 2);
    assert_eq!(seen[26], 0);
}

#[test]
fn a_timed_frame_after_another_picture_needs_it_before() {
    let p = present();
    let a = art("timed boss.png 0 99 a/after.png 48 38 0 0 after=hero.png");
    let mut pt = Painter::new();
    let one = |pic| Scene {
        bodies: vec![draw(pic, 0, 0)],
        ..Scene::default()
    };
    // boss straight away: no picture before it.
    pt.step(&a, &one(3));
    assert!(arts(&pt.frame(&a, &p, &framed(one(3)), 224)).is_empty());
    // foe, then hero, then boss: the picture before the last (hero) is foe.
    let mut pt = Painter::new();
    for pic in [2, 1, 3] {
        pt.step(&a, &one(pic));
    }
    assert!(arts(&pt.frame(&a, &p, &framed(one(3)), 224)).is_empty());
    // hero, then boss-throw, then boss: it is hero.
    let mut pt = Painter::new();
    for pic in [1, 4, 3] {
        pt.step(&a, &one(pic));
    }
    assert_eq!(arts(&pt.frame(&a, &p, &framed(one(3)), 224)).len(), 1);
}

#[test]
fn a_held_ball_is_hidden_while_the_pose_holds_it() {
    let p = present();
    let a = art("timed boss.png 0 99 a/hold.png 48 38 0 0 hide\nheld ball.png");
    let s = Scene {
        bodies: vec![draw(3, 20, 40), draw(5, 30, 50), draw(5, 120, 50)],
        ..Scene::default()
    };
    let mut pt = Painter::new();
    pt.step(&a, &s);
    let out = pt.frame(&a, &p, &framed(s), 224);
    let balls: Vec<i64> = out
        .draws
        .iter()
        .filter(|d| d.src == Src::Own(5))
        .map(|d| d.x / ART_SUB)
        .collect();
    // The ball in the boss's box isn't drawn; the one away from it is.
    assert_eq!(balls, vec![120]);
}

#[test]
fn a_face_body_turns_only_after_it_has_gone_far_enough() {
    let p = present();
    let a = art("pic bug.png a/bug.png 16 16 0 0\nface bug.png\nface_turn 6");
    let mut pt = Painter::new();
    let mut flips = Vec::new();
    // Right 5, back 2, right on: it turns once it is 6 right of where it
    // last went left (100), at 106.
    for x in [
        100, 101, 102, 103, 104, 105, 104, 103, 104, 106, 108, 110, 112,
    ] {
        let s = Scene {
            bodies: vec![draw(7, x, 50)],
            ..Scene::default()
        };
        pt.step(&a, &s);
        flips.push(pt.frame(&a, &p, &framed(s), 224).draws[0].flip_x);
    }
    assert_eq!(
        flips,
        [
            false, false, false, false, false, false, false, false, false, true, true, true, true
        ]
    );
}

#[test]
fn the_near_pose_turns_toward_the_other_body() {
    let p = present();
    let a = art("near a/yell.png 48 38 0 -5\nnear_when boss.png\nnear_other hero.png");
    // The boss at x 100, y 20: its feet at 52. The hero level with them, left.
    let s = Scene {
        bodies: vec![draw(3, 100, 20), draw(1, 60, 36)],
        ..Scene::default()
    };
    let mut pt = Painter::new();
    pt.step(&a, &s);
    let out = pt.frame(&a, &p, &framed(s), 224);
    let k = &out.draws[0];
    assert_eq!(k.src, Src::Art(0));
    assert!(k.flip_x);
    // Lower down, no yell.
    let s = Scene {
        bodies: vec![draw(3, 100, 20), draw(1, 60, 100)],
        ..Scene::default()
    };
    assert_eq!(pt.frame(&a, &p, &framed(s), 224).draws[0].src, Src::Own(3));
}

#[test]
fn elsewhere_moves_pictures_off_their_level() {
    let p = present();
    let a = art("elsewhere level.png 0 -9 foe.png");
    let on = |level| Scene {
        level: Some(level),
        bodies: vec![draw(2, 50, 60)],
        ..Scene::default()
    };
    let pt = Painter::new();
    let y = |s| {
        pt.frame(&a, &p, &framed(s), 224)
            .draws
            .iter()
            .find(|d| d.src == Src::Own(2))
            .unwrap()
            .y
            / ART_SUB
    };
    assert_eq!(y(on(0)), 52);
    assert_eq!(y(on(6)), 43);
}

#[test]
fn a_ball_landing_on_the_prop_bursts_and_is_not_drawn() {
    let p = present();
    let a = art(
        "prop level.png a/stump.png 28 200 37 18\nprop_over ball.png\nprop_burst foe.png hero.png",
    );
    let at = |y| Scene {
        level: Some(0),
        bodies: vec![draw(5, 30, y)],
        ..Scene::default()
    };
    let mut pt = Painter::new();
    pt.step(&a, &at(150));
    let out = pt.frame(&a, &p, &framed(at(150)), 224);
    assert!(out.draws.iter().any(|d| d.src == Src::Own(5)));
    // On the stump: the ball goes, a burst starts at its middle.
    pt.step(&a, &at(190));
    let out = pt.frame(&a, &p, &framed(at(190)), 224);
    assert!(!out.draws.iter().any(|d| d.src == Src::Own(5)));
    let b = out
        .draws
        .iter()
        .find(|d| d.src == Src::Own(2))
        .expect("a burst");
    assert_eq!(b.w, 26 * ART_SUB);
    // Behind the stump's box, the stump is drawn over it, clipped to it.
    let st = out
        .draws
        .iter()
        .find(|d| d.src == Src::Art(0))
        .expect("the stump over it");
    assert_eq!(
        st.clip,
        Some([30 * ART_SUB, 182 * ART_SUB, 16 * ART_SUB, 16 * ART_SUB])
    );
    // The burst plays out in 14 frames, two pictures.
    for _ in 0..14 {
        pt.step(&a, &at(190));
    }
    let out = pt.frame(&a, &p, &framed(at(190)), 224);
    assert!(!out.draws.iter().any(|d| matches!(d.src, Src::Own(1 | 2))));
}

#[test]
fn drift_rises_and_fades_on_its_level_only() {
    let p = present();
    let a = art("drift level.png 47 8 72\ndrift_art a/s.png 48 208\ndrift_at 23 0");
    let mut pt = Painter::new();
    let s = Scene {
        level: Some(0),
        ..Scene::default()
    };
    let mut alphas = Vec::new();
    for _ in 0..72 {
        pt.step(&a, &s);
        alphas.push(pt.frame(&a, &p, &framed(s.clone()), 224).draws[1].alpha);
    }
    // Faint at the ends of a rise, near 0.9 in the middle.
    assert!(alphas[0] < 20 && alphas[71] < 20, "{alphas:?}");
    assert!(alphas[35] > 220, "{alphas:?}");
    let off = Scene {
        level: Some(6),
        ..Scene::default()
    };
    assert_eq!(pt.frame(&a, &p, &framed(off), 224).draws.len(), 1);
}

#[test]
fn whole_number_sine() {
    assert_eq!(isin(0), 0);
    assert_eq!(isin(500), 1000);
    assert_eq!(isin(1000), 0);
    assert_eq!(isin(1500), -1000);
    assert!((isin(250) - 707).abs() <= 2);
}

#[test]
fn a_screen_in_place_of_the_level_draws_as_the_looks_picture() {
    let p = present();
    let a = art("pic other-level.png a/title.png 256 240 0 0");
    let s = Scene {
        level: Some(6),
        ..Scene::default()
    };
    let out = Painter::new().frame(&a, &p, &framed(s), 224);
    assert_eq!(out.draws[0].src, Src::Art(0));
    assert_eq!(out.draws[0].y, -8 * ART_SUB);
}

/// Moss Hop's shape of HUD, made up: a layer picture over the screen, a
/// glyph in the background at row 3 (y 24) and one at row 5, and a HUD
/// sprite; with a body standing across row 3.
fn hud_present() -> Present {
    let mut p = present();
    p.pictures.push(Picture {
        file: "hud.png".into(),
        size: [256, 240],
    });
    p.pictures.push(Picture {
        file: "glyph-1.png".into(),
        size: [8, 8],
    });
    p.levels = vec![crate::LevelLook {
        picture: Some(0),
        hud_layer: Some(8),
        ..Default::default()
    }];
    p
}

fn hud_scene() -> Scene {
    Scene {
        level: Some(0),
        bodies: vec![draw(1, 40, 16)],
        hud: vec![
            draw(8, 0, 0).unwrap(),
            draw(9, 40, 24).unwrap(),
            draw(9, 48, 40).unwrap(),
            draw(5, 200, 16).unwrap(),
        ],
        hud_sprites: 1,
        hud_tints: vec![(8, [1, 2, 3])],
        ..Scene::default()
    }
}

fn hud_art(text: &str) -> Art {
    Art::new(
        ArtRules::read(&format!(
            "gp-art 1\npic hero.png a/hero.png 16 32 0 -8\n{text}"
        ))
        .unwrap(),
        &hud_present(),
    )
    .unwrap()
}

/// The frame's draws as (what, clip's top and height in pixels).
fn order(f: &ArtFrame) -> Vec<(String, Option<(i64, i64)>)> {
    f.draws
        .iter()
        .map(|d| {
            let what = match d.src {
                Src::Own(8) => "layer".to_owned(),
                Src::Tint(8, _) => "tint".to_owned(),
                Src::Own(9) => format!("glyph@{}", d.y / ART_SUB),
                Src::Own(5) => "hud-sprite".to_owned(),
                Src::Own(0) => "level".to_owned(),
                Src::Art(0) => "hero".to_owned(),
                other => format!("{other:?}"),
            };
            (what, d.clip.map(|c| (c[1] / ART_SUB, c[3] / ART_SUB)))
        })
        .collect()
}

#[test]
fn added_lines_read_and_an_older_reader_passes_over_them() {
    let plain = "gp-art 1\npic hero.png a/hero.png 16 18 0 -2\n";
    let added = format!(
        "{plain}#+ hud_over 3 1 3\n#+ in top-ledge pic hero.png a/short.png 16 12 0 4\n\
         #+ in top-ledge timed hero.png 0 8 a/blink.png 16 12 0 4 every=60\n\
         #+ sparkle 3\n#+ hud_over x\n#+ in Top! pic hero.png a/x.png 1 1 0 0\n"
    );
    let r = ArtRules::read(&added).unwrap();
    assert_eq!(r.hud_over, Some(vec![1, 3]));
    assert_eq!(r.regions(), ["top-ledge"]);
    assert_eq!(r.region_pics[0].1.art.size, [16, 12]);
    assert_eq!(r.region_timed[0].1.every, 60);
    // Lines this reader can't read are passed over, with their numbers.
    let skipped: Vec<usize> = r.skipped.iter().map(|x| x.0).collect();
    assert_eq!(skipped, [6, 7, 8]);
    // The new art comes after the rest, so the others keep their numbers.
    assert_eq!(r.files(), ["a/hero.png", "a/short.png", "a/blink.png"]);
    // An older reader (gp-art 1 before these lines) reads `#+` as a note:
    // the same rules as the look without them.
    let old = ArtRules::read(plain).unwrap();
    let without = ArtRules {
        hud_over: None,
        region_pics: Vec::new(),
        region_timed: Vec::new(),
        skipped: Vec::new(),
        ..r.clone()
    };
    assert_eq!(without, old);
    // A whole-HUD line wins over rows, in either order.
    for t in ["#+ hud_over\n#+ hud_over 2", "#+ hud_over 2\n#+ hud_over"] {
        let r = ArtRules::read(&format!("gp-art 1\n{t}")).unwrap();
        assert_eq!(r.hud_over, Some(vec![]), "{t}");
    }
}

#[test]
fn the_hud_is_under_the_bodies_unless_a_line_says_it_is_over() {
    let p = hud_present();
    let f = framed(hud_scene());
    let names = |v: Vec<(String, Option<(i64, i64)>)>| -> Vec<String> {
        v.into_iter()
            .map(|(w, c)| match c {
                None => w,
                Some((y, h)) => format!("{w}[{y}+{h}]"),
            })
            .collect()
    };
    // The default, as every look before the line: the HUD's layer, its
    // tints and its text, then the bodies, then the HUD's sprites.
    let a = hud_art("");
    let fr = Painter::new().frame(&a, &p, &f, 224);
    assert_eq!(
        names(order(&fr)),
        [
            "level",
            "layer",
            "tint",
            "glyph@24",
            "glyph@40",
            "hero",
            "hud-sprite"
        ]
    );
    // Over: the bodies first, then the HUD whole.
    let a = hud_art("#+ hud_over");
    let fr = Painter::new().frame(&a, &p, &f, 224);
    assert_eq!(
        names(order(&fr)),
        [
            "level",
            "hero",
            "layer",
            "tint",
            "glyph@24",
            "glyph@40",
            "hud-sprite"
        ]
    );
    // Row 3 only (y 24 to 32 on the screen, and in this window): the layer
    // and its tints in two bands under the bodies and one over them, the
    // text by its row.
    let a = hud_art("#+ hud_over 3");
    let fr = Painter::new().frame(&a, &p, &f, 224);
    let far = FAR;
    assert_eq!(
        names(order(&fr)),
        [
            "level".to_owned(),
            format!("layer[{}+{}]", -far, far + 24),
            format!("layer[32+{}]", far - 32),
            format!("tint[{}+{}]", -far, far + 24),
            format!("tint[32+{}]", far - 32),
            "glyph@40".to_owned(),
            "hero".to_owned(),
            "layer[24+8]".to_owned(),
            "tint[24+8]".to_owned(),
            "glyph@24".to_owned(),
            "hud-sprite".to_owned(),
        ]
    );
    // Rows next to each other make one band.
    assert_eq!(hud_bands(Some(&[2, 3, 6])).0, [(16, 32), (48, 56)]);
    assert_eq!(
        hud_bands(Some(&[2, 3, 6])).1,
        [(-FAR, 16), (32, 48), (56, FAR)]
    );
}

#[test]
fn a_pose_by_place_shows_only_in_its_region() {
    let p = present();
    let rules = "pic hero.png a/hero.png 16 32 0 -16\n\
         #+ in top-ledge pic hero.png a/short.png 16 12 0 4\n\
         #+ in top-ledge timed hero.png 0 4 a/blink.png 16 12 0 4\n";
    let mut a = art(rules);
    let at = |a: &Art, painter: &Painter, x: i64, y: i64, level: u32| {
        let f = framed(Scene {
            level: Some(level),
            bodies: vec![draw(1, x, y)],
            ..Scene::default()
        });
        let fr = painter.frame(a, &p, &f, 224);
        arts(&fr)[0].0
    };
    let mut painter = Painter::new();
    // Before the kit's regions are given, no `in` line holds.
    assert_eq!(at(&a, &painter, 40, 20, 0), 0);
    a.set_regions(
        &p,
        &[Region {
            name: "top-ledge".into(),
            level: "level.png".into(),
            rect: [32, 16, 64, 24],
        }],
    );
    // The body's feet (x 48, y 35) on the ledge: its first four frames
    // the timed pose, then the region's picture.
    let s = Scene {
        level: Some(0),
        bodies: vec![draw(1, 40, 20)],
        ..Scene::default()
    };
    painter.step(&a, &s);
    assert_eq!(at(&a, &painter, 40, 20, 0), 2);
    for _ in 0..4 {
        painter.step(&a, &s);
    }
    assert_eq!(at(&a, &painter, 40, 20, 0), 1);
    // Off it, below it, or on another level: the full picture.
    assert_eq!(at(&a, &painter, 120, 20, 0), 0);
    assert_eq!(at(&a, &painter, 40, 40, 0), 0);
    assert_eq!(at(&a, &painter, 40, 20, 6), 0);
    // A region the kit doesn't name: never.
    a.set_regions(&p, &[]);
    assert_eq!(at(&a, &painter, 40, 20, 0), 0);
}

/// A `#+ on SCREEN` rule holds on that screen (and a board's stage of it,
/// `SCREEN-N`) and the plain rule on every other: a drink's liquid drawn
/// one way in a room where the bartender stands mirrored and the other way
/// in the rest.
#[test]
fn a_rule_on_a_screen_holds_there_and_the_plain_one_elsewhere() {
    let pic = |f: &str, w, h| Picture {
        file: f.into(),
        size: [w, h],
    };
    let p = Present {
        pictures: vec![
            pic("screen_saloon", 512, 480),
            pic("screen_park", 512, 480),
            pic("screen_park-1", 512, 480),
            pic("sprite-beer-15", 32, 32),
            pic("sprite-man-1", 32, 32),
        ],
        ..Present::default()
    };
    let text = "gp-art 1\nscale 2\n\
        pic sprite-beer-15 a/beer.png 32 32 0 0\n\
        also sprite-man-1 a/mug.png 8 8 20 4\n\
        #+ on screen_park pic sprite-beer-15 a/beer-left.png 32 32 -10 0\n\
        #+ on screen_park also sprite-man-1 a/mug-left.png 8 8 2 4\n\
        #+ on screen_saloon screen_park timed sprite-man-1 0 2 a/blink.png 32 32 0 0\n\
        #+ on pic sprite-beer-15 a/x.png 1 1 0 0\n";
    let r = ArtRules::read(text).unwrap();
    assert_eq!(r.on_screen.len(), 3);
    assert_eq!(r.on_screen[2].screens, ["screen_saloon", "screen_park"]);
    // An `on` with no screen is passed over.
    assert_eq!(r.skipped.iter().map(|x| x.0).collect::<Vec<_>>(), [8]);
    // The screens are names the build must have.
    assert_eq!(
        r.passed_over(&["sprite-beer-15", "sprite-man-1"]),
        ["screen_park", "screen_saloon"]
    );
    let a = Art::new(r, &p).unwrap();
    let file = |n: &str| a.files.iter().position(|f| f == n).unwrap() as u32;
    let on = |level: u32, painter: &Painter| {
        let f = framed(Scene {
            level: Some(level),
            bodies: vec![draw(3, 100, 100), draw(4, 200, 100)],
            ..Scene::default()
        });
        arts(&painter.frame(&a, &p, &f, 224))
    };
    let old = Painter::new();
    let mut painter = Painter::new();
    // Past the timed frames: the screen's own beer and mug in the park and
    // in its stage, the plain ones in the saloon.
    for _ in 0..3 {
        painter.step(
            &a,
            &Scene {
                level: Some(0),
                bodies: vec![draw(3, 100, 100), draw(4, 200, 100)],
                ..Scene::default()
            },
        );
    }
    for level in [1, 2] {
        assert_eq!(
            on(level, &painter),
            [
                (file("a/beer-left.png"), 90, 92),
                (file("a/mug-left.png"), 202, 96)
            ],
            "level {level}"
        );
    }
    assert_eq!(
        on(0, &painter),
        [(file("a/beer.png"), 100, 92), (file("a/mug.png"), 220, 96)]
    );
    // A timed rule on both screens holds on each, before the rest.
    for level in [0, 1] {
        assert_eq!(on(level, &old)[1].0, file("a/blink.png"), "level {level}");
    }
}

/// An `also` picture is drawn with a body while it shows its build, over
/// the body and turned with it; another pose draws none. It is a file of
/// its own: a `pic` before it doesn't take its place.
#[test]
fn an_also_picture_comes_with_its_pose() {
    let p = present();
    let a = art("scale 6\npic bug.png a/first.png 8 8 0 0\nalso hero.png a/lever.png 8 8 4 -6");
    assert_eq!(a.files, ["a/first.png", "a/lever.png"]);
    let mut f = framed(Scene {
        level: Some(0),
        bodies: vec![draw(1, 40, 100), draw(2, 80, 100)],
        ..Scene::default()
    });
    f.scene.bodies[0].as_mut().unwrap().flip_x = true;
    let out = Painter::new().frame(&a, &p, &f, 224);
    assert_eq!(out.draws.len(), 4);
    assert_eq!(out.draws[1].src, Src::Own(1));
    let e = &out.draws[2];
    assert_eq!(e.src, Src::Art(1));
    // Turned with the body about the body's own picture: 4 from its left
    // edge unturned is 4 from its right edge turned.
    let body = &out.draws[1];
    assert_eq!(
        [e.x, e.y, e.w, e.h],
        [
            body.x + body.w - (4 + 8) * ART_SUB,
            86 * ART_SUB,
            8 * ART_SUB,
            8 * ART_SUB
        ]
    );
    assert!(e.flip_x);
    assert_eq!(out.draws[3].src, Src::Own(2));
    // Unturned, it stands at its offset from the body's left edge.
    f.scene.bodies[0].as_mut().unwrap().flip_x = false;
    let out = Painter::new().frame(&a, &p, &f, 224);
    assert_eq!(out.draws[2].x, 44 * ART_SUB);
}
