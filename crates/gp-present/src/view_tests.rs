use gp_sim::Fx;
use gp_sim::world::Body;

use super::*;
use crate::{Picture, SpriteLimit};

fn nes() -> Screen {
    Screen {
        size: [256, 240],
        aspect: [8, 7],
        overscan: [0, 8, 0, 8],
        sprite_limit: None,
        x_wraps: false,
    }
}

fn wide(side: Side) -> Option<Wide> {
    Some(Wide {
        aspect: [16, 9],
        side,
    })
}

#[test]
fn widescreen_widths_keep_the_pixels_shape() {
    let s = nes();
    // 224 lines at 16:9 is 398 square pixels, 348 of the NES's 8/7 ones.
    assert_eq!(s.wide_width([16, 9]), 348);
    assert_eq!(s.wide_width([16, 10]), 312);
    // Narrower than the screen: the screen's own width.
    assert_eq!(s.wide_width([1, 1]), 256);
    let square = Screen::default();
    assert_eq!(square.wide_width([16, 9]), 426);
    assert_eq!(s.window_for(348), [397, 224]);
    assert_eq!(s.window_for(256), s.window(1));
}

#[test]
fn without_widescreen_the_window_is_the_games_view() {
    let w = window(&nes(), [300, 0], None, Some(3000), true);
    assert_eq!((w.x, w.y, w.w, w.h), (300, 8, 256, 224));
    assert_eq!((w.view_x, w.wall), (0, 0));
    assert_eq!(w.hud, [300, 0]);
}

#[test]
fn the_extra_width_goes_mostly_ahead_and_the_wall_side_is_dimmed() {
    let s = nes();
    // 92 extra pixels: 23 behind, 69 ahead.
    let w = window(&s, [1000, 0], wide(Side::Ahead), Some(3000), true);
    assert_eq!((w.x, w.w, w.view_x, w.wall), (977, 348, 23, 23));
    // The HUD is in the window's middle.
    assert_eq!(w.hud, [977 + 46, 0]);
    let c = window(&s, [1000, 0], wide(Side::Center), Some(3000), false);
    assert_eq!((c.x, c.view_x, c.wall), (954, 46, 0));
    let f = window(&s, [1000, 0], wide(Side::Front), Some(3000), true);
    assert_eq!((f.x, f.view_x, f.wall), (1000, 0, 0));
}

#[test]
fn the_window_stays_in_the_level() {
    let s = nes();
    // At the level's start the extra goes ahead.
    let w = window(&s, [0, 0], wide(Side::Center), Some(3000), true);
    assert_eq!((w.x, w.view_x, w.wall), (0, 0, 0));
    // At its end, behind.
    let e = window(&s, [3000 - 256, 0], wide(Side::Front), Some(3000), true);
    assert_eq!((e.x, e.view_x), (3000 - 348, 92));
    // A level narrower than the window is centered in it, and the game's
    // view is still all in the window.
    let n = window(&s, [0, 0], wide(Side::Ahead), Some(256), false);
    assert_eq!((n.x, n.view_x), (-46, 46));
    assert!(!n.outside_view(0, 16, 256));
    assert!(n.outside_view(-30, 16, 256));
    assert!(n.outside_view(256, 16, 256));
}

fn limited(flicker: bool) -> Present {
    Present {
        screen: Screen {
            sprite_limit: Some(SpriteLimit {
                per_line: 8,
                width: 8,
                flicker,
            }),
            ..nes()
        },
        pictures: vec![Picture {
            file: "p.png".into(),
            size: [16, 16],
        }],
        ..Present::default()
    }
}

fn row(n: usize, y: i64) -> Scene {
    Scene {
        bodies: (0..n)
            .map(|i| {
                Some(Draw {
                    pic: 0,
                    x: 20 * i as i64,
                    y,
                    flip_x: false,
                    flip_y: false,
                })
            })
            .collect(),
        ..Scene::default()
    }
}

#[test]
fn the_sprite_limit_drops_bodies_past_eight_sprites_on_a_line() {
    let p = limited(false);
    let s = row(6, 100);
    let win = window(&p.screen, [0, 0], None, None, false);
    // Each body is two sprites wide: four fit on the line.
    let hidden = over_limit(&p, &s, &win, 0);
    assert_eq!(hidden, [false, false, false, false, true, true]);
    assert_eq!(over_limit(&p, &s, &win, 1), hidden);
    // On different lines they all show.
    let mut apart = row(6, 100);
    for (i, b) in apart.bodies.iter_mut().enumerate() {
        b.as_mut().unwrap().y = 20 * i as i64;
    }
    assert!(over_limit(&p, &apart, &win, 0).iter().all(|h| !h));
    // A model without a limit hides nothing.
    let free = Present {
        screen: nes(),
        ..limited(false)
    };
    assert!(over_limit(&free, &s, &win, 0).iter().all(|h| !h));
}

/// Core `present.sprite_order`: the NES drops sprites line by line, by
/// their place in the table, so a body sharing only some lines with a
/// full row loses those lines alone (Original Edition's 50m: a moving
/// ladder's lower sprites on the lines its pies crowd).
#[test]
fn the_limit_drops_the_lines_a_later_body_shares() {
    let p = limited(false);
    let win = window(&p.screen, [0, 0], None, None, false);
    // Four bodies, eight sprites, on lines 100 to 115; a fifth from 92.
    let mut s = row(5, 100);
    s.bodies[4].as_mut().unwrap().y = 92;
    let cuts = sprite_cuts(&p, &s, &win, 0);
    // Its two columns on lines 100 to 107 of the screen (8 to 15 of its
    // picture) are past the limit; its top half shows.
    assert_eq!(cuts[4], [[0, 8, 8, 8], [8, 8, 8, 8]]);
    assert!(cuts[..4].iter().all(Vec::is_empty));
    assert!(over_limit(&p, &s, &win, 0).iter().all(|h| !h));
    // First in the table, it keeps its lines and the last of the row
    // loses those.
    s.rank = vec![10, 11, 12, 13, 0];
    let cuts = sprite_cuts(&p, &s, &win, 0);
    assert!(cuts[4].is_empty());
    assert_eq!(cuts[3], [[0, 0, 8, 8], [8, 0, 8, 8]]);
    // The scene keeps the cuts and hides nothing whole.
    let mut limited_scene = s.clone();
    limit(&p, &mut limited_scene, &win, 0);
    assert_eq!(limited_scene.cut, cuts);
    assert!(limited_scene.bodies.iter().all(Option::is_some));
}

#[test]
fn a_flickering_limit_takes_turns() {
    let p = limited(true);
    let s = row(6, 100);
    let win = window(&p.screen, [0, 0], None, None, false);
    let a = over_limit(&p, &s, &win, 0);
    let b = over_limit(&p, &s, &win, 1);
    assert_ne!(a, b);
    // Over six frames every body is shown on some and hidden on some.
    for i in 0..6 {
        let shown = (0..6).filter(|f| !over_limit(&p, &s, &win, *f)[i]).count();
        assert!(shown > 0 && shown < 6, "body {i} shown on {shown} frames");
    }
}

#[test]
fn easing_limits_a_flash_to_a_small_step() {
    assert_eq!(ease([10, 10, 10], [20, 20, 20]), [20, 20, 20]);
    let c = ease([0, 0, 0], [255, 255, 255]);
    assert!(c[0] > 0 && c[0] <= DIM_STEP as u8 + 1, "{c:?}");
}

fn spec() -> Spec {
    let mut s = Spec {
        name: "test".into(),
        actions: Vec::new(),
        kinds: Vec::new().into(),
        levels: Vec::new().into(),
        start_level: 0,
        cameras: Vec::new().into(),
        machines: Vec::new().into(),
        counters: Vec::new().into(),
        events: Vec::new(),
        own_events: 0,
        order: Vec::new(),
        seed: 0,
        profile: None,
        keep_direction: false,
        turn: None,
        stages: None,
        cost: None,
    };
    s.kinds.push(crate::tests::kind("hero"));
    s
}

fn body(x: i64, vx: i64, active: bool) -> Body {
    Body {
        kind: 0,
        x: Fx::from_int(x),
        y: Fx::from_int(100),
        vx: Fx::from_int(vx),
        alive: active,
        ..Body::default()
    }
}

fn at(frame: u64, bodies: Vec<Body>, camera: i64) -> World {
    World {
        frame,
        bodies,
        cameras: vec![[Fx::from_int(camera), Fx::ZERO]],
        ..World::default()
    }
}

fn scene_of(_p: &Present, w: &World) -> Scene {
    Scene {
        camera: [w.cameras[0][0].floor(), 0],
        clear: [0, 0, 0],
        bodies: w
            .bodies
            .iter()
            .map(|b| {
                b.active().then_some(Draw {
                    pic: 0,
                    x: b.x.floor(),
                    y: b.y.floor(),
                    flip_x: false,
                    flip_y: false,
                })
            })
            .collect(),
        ..Scene::default()
    }
}

fn smooth_viewer() -> Viewer {
    Viewer::new(Options {
        smooth: true,
        ..Options::default()
    })
}

#[test]
fn smooth_motion_draws_part_way_between_two_ticks() {
    let p = limited(false);
    let spec = spec();
    let mut v = smooth_viewer();
    let w0 = at(10, vec![body(40, 2, true)], 0);
    let w1 = at(11, vec![body(42, 2, true)], 0);
    v.step(&p, &w0, &scene_of(&p, &w0));
    v.step(&p, &w1, &scene_of(&p, &w1));
    let s = scene_of(&p, &w1);
    // Half way: a pixel back from where it is.
    assert_eq!(v.frame(&p, &spec, &w1, &s, SUB / 2).nudge[0], [-SUB, 0]);
    assert_eq!(v.frame(&p, &spec, &w1, &s, 0).nudge[0], [-2 * SUB, 0]);
    // At the tick, and with smooth motion off, it is where it is.
    assert_eq!(v.frame(&p, &spec, &w1, &s, SUB).nudge[0], [0, 0]);
    v.options.smooth = false;
    assert_eq!(v.frame(&p, &spec, &w1, &s, 0).nudge[0], [0, 0]);
}

#[test]
fn a_jump_in_state_is_drawn_at_once() {
    let p = limited(false);
    let spec = spec();
    // Moved 50 pixels at a speed of 2: a pipe or a respawn.
    let mut v = smooth_viewer();
    let w0 = at(10, vec![body(40, 2, true)], 0);
    let w1 = at(11, vec![body(90, 2, true)], 0);
    v.step(&p, &w0, &scene_of(&p, &w0));
    v.step(&p, &w1, &scene_of(&p, &w1));
    assert_eq!(
        v.frame(&p, &spec, &w1, &scene_of(&p, &w1), 0).nudge[0],
        [0, 0]
    );
    // Ticks that don't follow each other (a rewind, a load) aren't blended.
    let mut v = smooth_viewer();
    let w2 = at(30, vec![body(42, 2, true)], 0);
    v.step(&p, &w0, &scene_of(&p, &w0));
    v.step(&p, &w2, &scene_of(&p, &w2));
    assert_eq!(
        v.frame(&p, &spec, &w2, &scene_of(&p, &w2), 0).nudge[0],
        [0, 0]
    );
    // The camera's cut likewise; a small scroll blends.
    let mut v = smooth_viewer();
    let c0 = at(10, vec![body(40, 0, true)], 0);
    let c1 = at(11, vec![body(40, 0, true)], 3);
    v.step(&p, &c0, &scene_of(&p, &c0));
    v.step(&p, &c1, &scene_of(&p, &c1));
    assert_eq!(
        v.frame(&p, &spec, &c1, &scene_of(&p, &c1), 0).camera_nudge,
        [-3 * SUB, 0]
    );
    let c2 = at(12, vec![body(40, 0, true)], 200);
    v.step(&p, &c2, &scene_of(&p, &c2));
    assert_eq!(
        v.frame(&p, &spec, &c2, &scene_of(&p, &c2), 0).camera_nudge,
        [0, 0]
    );
}

#[test]
fn a_body_appearing_or_going_outside_the_view_fades() {
    let p = limited(false);
    let spec = spec();
    let mut v = Viewer::new(Options::default());
    // Body 0 waits past the view's right edge, body 1 inside the view.
    let w0 = at(10, vec![body(300, 0, false), body(100, 0, false)], 0);
    let w1 = at(11, vec![body(300, 0, true), body(100, 0, true)], 0);
    v.step(&p, &w0, &scene_of(&p, &w0));
    v.step(&p, &w1, &scene_of(&p, &w1));
    let f = v.frame(&p, &spec, &w1, &scene_of(&p, &w1), SUB);
    assert_eq!(f.alpha, [0, 255]);
    let mut w = w1.clone();
    let mut last = 0;
    for k in 2..=FADE as u64 + 1 {
        w.frame = 10 + k;
        v.step(&p, &w, &scene_of(&p, &w));
        let a = v.frame(&p, &spec, &w, &scene_of(&p, &w), SUB).alpha[0];
        assert!(a >= last);
        last = a;
    }
    assert_eq!(last, 255);
    // Body 0 goes while outside the view: a ghost fades out where it was.
    w.frame += 1;
    w.bodies[0].alive = false;
    v.step(&p, &w, &scene_of(&p, &w));
    let f = v.frame(&p, &spec, &w, &scene_of(&p, &w), SUB);
    assert_eq!(f.ghosts.len(), 1);
    assert_eq!((f.ghosts[0].0.x, f.ghosts[0].1), (300, 255));
    // Body 1 dies inside the view: it goes at once, as in the original.
    w.frame += 1;
    w.bodies[1].alive = false;
    v.step(&p, &w, &scene_of(&p, &w));
    let f = v.frame(&p, &spec, &w, &scene_of(&p, &w), SUB);
    assert_eq!(f.ghosts.len(), 1);
    assert!(f.ghosts[0].1 < 255);
}

#[test]
fn dimming_turns_a_strobe_into_small_steps() {
    let p = limited(false);
    let spec = spec();
    let mut v = Viewer::new(Options {
        dim_flashes: true,
        ..Options::default()
    });
    let mut w = at(0, Vec::new(), 0);
    let mut shown = Vec::new();
    for f in 0..12 {
        w.frame = f;
        let mut s = scene_of(&p, &w);
        s.clear = if f % 2 == 0 {
            [0, 0, 0]
        } else {
            [255, 255, 255]
        };
        v.step(&p, &w, &s);
        shown.push(v.frame(&p, &spec, &w, &s, SUB).scene.clear);
    }
    for pair in shown.windows(2) {
        let d = (i32::from(pair[1][0]) - i32::from(pair[0][0])).abs();
        assert!(d <= DIM_STEP + 1, "{shown:?}");
    }
    v.options.dim_flashes = false;
    let s = Scene {
        clear: [255, 255, 255],
        ..scene_of(&p, &w)
    };
    assert_eq!(v.frame(&p, &spec, &w, &s, SUB).scene.clear, [255, 255, 255]);
}

#[test]
fn the_viewer_never_changes_the_world() {
    let p = limited(true);
    let spec = spec();
    let mut v = Viewer::new(Options::all());
    let w0 = at(10, vec![body(40, 2, true), body(300, 0, false)], 0);
    let w1 = at(11, vec![body(42, 2, true), body(300, 0, true)], 1);
    let (h0, h1) = (w0.hash(), w1.hash());
    for w in [&w0, &w1] {
        v.step(&p, w, &scene_of(&p, w));
        let _ = v.frame(&p, &spec, w, &scene_of(&p, w), SUB / 3);
    }
    assert_eq!((w0.hash(), w1.hash()), (h0, h1));
}

#[test]
fn a_body_in_the_name_table_takes_no_sprites() {
    let p = limited(false);
    let mut s = row(6, 100);
    // The first two are background tiles (Donkey Kong): the rest fit.
    s.name_table = vec![true, true, false, false, false, false];
    let win = window(&p.screen, [0, 0], None, None, false);
    assert!(over_limit(&p, &s, &win, 0).iter().all(|h| !h));
}
