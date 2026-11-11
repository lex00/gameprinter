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
            pic("kong.png", 48, 32),
            pic("kong-throw.png", 48, 32),
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
         timed kong.png 0 12 a/push.png 48 38 0 -5 every=60 after=ball.png wait=26 hide\n\
         face bug.png\nface_turn 4\nheld ball.png\n\
         near a/yell.png 48 38 0 -5\nnear_when kong.png\nnear_other hero.png\n\
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
fn a_rule_for_a_missing_picture_is_refused() {
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
    let a = art("pic kong.png a/stand.png 48 38 0 -5\n\
         timed kong.png 0 12 a/push.png 48 38 0 -5\n\
         timed kong.png 12 26 a/hold.png 48 38 0 -5 hide\n");
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
    let a = art("timed kong.png 0 99 a/after.png 48 38 0 0 after=hero.png");
    let mut pt = Painter::new();
    let one = |pic| Scene {
        bodies: vec![draw(pic, 0, 0)],
        ..Scene::default()
    };
    // kong straight away: no picture before it.
    pt.step(&a, &one(3));
    assert!(arts(&pt.frame(&a, &p, &framed(one(3)), 224)).is_empty());
    // foe, then hero, then kong: the picture before the last (hero) is foe.
    let mut pt = Painter::new();
    for pic in [2, 1, 3] {
        pt.step(&a, &one(pic));
    }
    assert!(arts(&pt.frame(&a, &p, &framed(one(3)), 224)).is_empty());
    // hero, then kong-throw, then kong: it is hero.
    let mut pt = Painter::new();
    for pic in [1, 4, 3] {
        pt.step(&a, &one(pic));
    }
    assert_eq!(arts(&pt.frame(&a, &p, &framed(one(3)), 224)).len(), 1);
}

#[test]
fn a_held_ball_is_hidden_while_the_pose_holds_it() {
    let p = present();
    let a = art("timed kong.png 0 99 a/hold.png 48 38 0 0 hide\nheld ball.png");
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
    // The ball in Kong's box isn't drawn; the one away from him is.
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
    let a = art("near a/yell.png 48 38 0 -5\nnear_when kong.png\nnear_other hero.png");
    // Kong at x 100, y 20: his feet at 52. The hero level with them, left.
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
