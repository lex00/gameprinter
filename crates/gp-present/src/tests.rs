use gp_sim::Fx;
use gp_sim::spec::{Kind, Machine, Rect, Role, State};
use gp_sim::world::{Body, MachineState};

use super::*;

fn n(v: i64) -> Fx {
    Fx::from_int(v)
}

pub(crate) fn kind(name: &str) -> Kind {
    let rect = Rect {
        x: n(0),
        y: n(0),
        w: n(16),
        h: n(16),
    };
    Kind {
        name: name.into(),
        role: Role::Player,
        size: [n(16), n(16)],
        hitbox: rect,
        tile_hitbox: rect,
        tile_collision: false,
        run: None,
        jump: None,
        patrol: None,
        contact: None,
        walk: None,
        fall: None,
        fly: None,
        hop: None,
        oscillate: None,
        leave_view: None,
        leap: None,
        launcher: None,
        launched: false,
        shuttle: None,
        stream: None,
        orbit: None,
        orbit_part: None,
        springboard: None,
        path: None,
        climb: None,
        health: None,
        knockback: None,
        hits: None,
        shoot: None,
        carry: None,
        carryable: None,
        move8: None,
        machine: None,
        ext: None,
        ext_params: Default::default(),
        ext_tables: Default::default(),
        no_slot: false,
        hook_through_stops: false,
        hook_frame_start: false,
        hook_first: false,
        hook_last: false,
        extra_slots: 0,
        despawn_behind: None,
        despawn_behind_wrap: None,
        despawn_ahead: None,
        invincible: None,
        locks: None,
        bounce: None,
        touch_points: Vec::new(),
    }
}

/// One entity type and a game machine with a frozen state.
fn spec() -> Spec {
    Spec {
        name: "test".into(),
        actions: Vec::new(),
        kinds: vec![kind("hero")].into(),
        levels: Vec::new().into(),
        start_level: 0,
        cameras: Vec::new().into(),
        machines: vec![Machine {
            name: "game".into(),
            initial: 0,
            states: vec![
                State {
                    name: "playing".into(),
                    freeze: false,
                    part: Default::default(),
                    transitions: Vec::new(),
                },
                State {
                    name: "dying".into(),
                    freeze: true,
                    part: Default::default(),
                    transitions: Vec::new(),
                },
            ],
        }]
        .into(),
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
    }
}

fn world(vx: i64) -> World {
    World {
        bodies: vec![Body {
            kind: 0,
            x: n(40),
            y: n(100),
            vx: Fx::from_raw(vx),
            on_ground: true,
            alive: true,
            ..Body::default()
        }],
        cameras: vec![[n(8), n(0)]],
        machines: vec![MachineState::default()],
        counters: vec![42, 7],
        ..World::default()
    }
}

fn frame(id: u32) -> Frame {
    Frame {
        id,
        ..Frame::default()
    }
}

/// A walk cycle of three pictures stepping every 4 frames below 1.5
/// pixels a frame and every 2 at or above it, a jump picture in the air,
/// and a two-picture idle on the game clock every 8 frames.
fn present() -> Present {
    Present {
        screen: Screen {
            size: [256, 240],
            aspect: [8, 7],
            overscan: [0, 8, 0, 8],
            sprite_limit: Some(SpriteLimit {
                per_line: 8,
                width: 8,
                flicker: true,
            }),
            x_wraps: false,
        },
        pictures: (0..12)
            .map(|i| Picture {
                file: format!("p{i}.png"),
                size: [16, if i == 11 { 24 } else { 16 }],
            })
            .collect(),
        // Six frames, each in two palette sets.
        frames: (0..6).map(|f| vec![f * 2, f * 2 + 1]).collect(),
        // Frame 5 is laid out from the position.
        origins: vec![None, None, None, None, None, Some([-4, -20])],
        kinds: vec![vec![Some(Look {
            base: frame(0),
            facing_flip: true,
            copy: None,
            mirror: 16,
            blink: None,
            behind: false,
            behind_when: None,
            split: None,
            replaced: None,
            leaves: None,
            front: false,
            name_table: false,
            in_place: false,
            clips: vec![
                Clip {
                    when: When {
                        ground: Some(false),
                        ..When::default()
                    },
                    frames: vec![frame(4)],
                    ..Clip::default()
                },
                Clip {
                    when: When {
                        moving: Some(true),
                        ..When::default()
                    },
                    frames: vec![frame(1), frame(2), frame(3)],
                    timing: vec![(Some(0x18000), 2), (None, 4)],
                    ..Clip::default()
                },
                Clip {
                    frames: vec![
                        frame(0),
                        Frame {
                            id: 0,
                            flip_x: true,
                            flip_y: false,
                        },
                    ],
                    timing: vec![(None, 8)],
                    game_clock: true,
                    ..Clip::default()
                },
            ],
        })]],
        levels: vec![LevelLook {
            picture: Some(11),
            clear: [1, 2, 3],
            set: 1,
            cycles: vec![
                Cycle {
                    mask: Some(10),
                    hud_mask: None,
                    colors: vec![[9, 9, 9], [8, 8, 8], [7, 7, 7]],
                    every: 8,
                    counter: None,
                },
                // Picked by counter 1 (7), less one.
                Cycle {
                    mask: Some(12),
                    hud_mask: None,
                    colors: vec![[1, 1, 1], [2, 2, 2], [3, 3, 3]],
                    every: 8,
                    counter: Some((1, -1)),
                },
            ],
            label: "1-1".into(),
            hud_layer: Some(9),
            cells: Some(CellLook {
                kinds: vec![(0, 20, 21), (1, 22, 23)],
                held: Some(24),
                own: vec![(3, 25)],
            }),
            order: vec![
                None,
                Some(crate::SpriteOrder {
                    at: 20,
                    step: 4,
                    word: Some(3),
                    turn: Some((1, 7)),
                }),
            ],
            effect_order: vec![Some((48, 2))],
        }],
        hud: Some(Hud {
            fields: vec![
                Field {
                    at: [2, 3],
                    source: Source::Counter(0),
                    digits: 6,
                    divide: 1,
                    pad: '0',
                    palette: 0,
                    when: Vec::new(),
                },
                Field {
                    at: [19, 3],
                    source: Source::Level,
                    digits: 0,
                    divide: 1,
                    pad: '0',
                    palette: 0,
                    when: Vec::new(),
                },
                Field {
                    at: [0, 0],
                    source: Source::Text("x y".into()),
                    digits: 0,
                    divide: 1,
                    pad: ' ',
                    palette: 1,
                    when: Vec::new(),
                },
            ],
            glyphs: "0123456789-"
                .chars()
                .map(|ch| Glyph {
                    set: 0,
                    palette: 0,
                    ch,
                    pic: 8,
                })
                .collect(),
            sprites: vec![HudSprite {
                at: [88, 25],
                pics: vec![30],
            }],
            sets: vec![0],
        }),
        bump: None,
        effects: Vec::new(),
        screens: Vec::new(),
        sprites_off: Vec::new(),
        hidden: Vec::new(),
    }
}

/// The picture body 0 shows after each of `frames` steps.
fn pictures(p: &Present, spec: &Spec, w: &mut World, frames: usize) -> Vec<(u32, bool)> {
    let mut a = Animator::new();
    let mut out = Vec::new();
    for _ in 0..frames {
        w.frame += 1;
        a.step(p, spec, w);
        let d = a.scene(p, spec, w).bodies[0].unwrap();
        out.push((d.pic, d.flip_x));
    }
    out
}

#[test]
fn a_walk_cycle_steps_by_speed() {
    let (p, spec) = (present(), spec());
    // Slow: a new picture every 4 frames, in the level's palette set 1.
    let slow = pictures(&p, &spec, &mut world(0x8000), 9);
    let ids: Vec<u32> = slow.iter().map(|(pic, _)| *pic).collect();
    assert_eq!(ids, [3, 3, 3, 3, 5, 5, 5, 5, 7]);
    // Fast: every 2.
    let fast = pictures(&p, &spec, &mut world(0x20000), 5);
    let ids: Vec<u32> = fast.iter().map(|(pic, _)| *pic).collect();
    assert_eq!(ids, [3, 3, 5, 5, 7]);
}

/// The test presentation with its walk counted by the body's timer.
fn body_clock_present() -> Present {
    let mut p = present();
    let look = p.kinds[0][0].as_mut().unwrap();
    look.clips[1].body_clock = true;
    p
}

/// The picture body 0 shows after each step, with the body on the ground
/// or not by `ground`.
fn pictures_with(
    p: &Present,
    spec: &Spec,
    w: &mut World,
    a: &mut Animator,
    ground: &[bool],
) -> Vec<u32> {
    ground
        .iter()
        .map(|g| {
            w.bodies[0].on_ground = *g;
            w.frame += 1;
            a.step(p, spec, w);
            a.scene(p, spec, w).bodies[0].unwrap().pic
        })
        .collect()
}

#[test]
fn a_body_clock_draws_then_steps_when_its_timer_runs_out() {
    let (p, spec) = (body_clock_present(), spec());
    let mut w = world(0x8000);
    let mut a = Animator::new();
    // A new body's timer has run out: it draws the first picture, then
    // steps and sets the timer to 4, as Super Mario Bros.'
    // AnimationControl does.
    let got = pictures_with(&p, &spec, &mut w, &mut a, &[true; 10]);
    assert_eq!(got, [3, 5, 5, 5, 5, 7, 7, 7, 7, 3]);
}

#[test]
fn a_body_clock_keeps_running_while_another_clip_shows() {
    let (p, spec) = (body_clock_present(), spec());
    let mut w = world(0x8000);
    let mut a = Animator::new();
    // Two frames of walk (timer 4, then 3) and a jump of two frames (2,
    // then 1). The timer runs out on landing, so the walk draws its first
    // picture and steps at once: the timer is the body's, not the clip's.
    let walk_jump_walk = [true, true, false, false, true, true, true];
    let got = pictures_with(&p, &spec, &mut w, &mut a, &walk_jump_walk);
    assert_eq!(got, [3, 5, 9, 9, 3, 5, 5]);
    // A jump of one frame leaves time on it: the walk draws its first
    // picture until it runs out.
    let mut a = Animator::new();
    let got = pictures_with(&p, &spec, &mut w, &mut a, &[true, false, true, true, true]);
    assert_eq!(got, [3, 9, 3, 3, 3]);
}

#[test]
fn a_seeded_body_clock_starts_where_the_game_is() {
    let (p, spec) = (body_clock_present(), spec());
    let mut w = world(0x8000);
    let mut a = Animator::new();
    w.frame += 1;
    a.step(&p, &spec, &w);
    // The game is at the walk's third picture with 2 frames left: it shows
    // it until the timer runs out, then the first.
    a.seed(&p, &w, 0, Some(2), Some(2));
    assert_eq!(a.scene(&p, &spec, &w).bodies[0].unwrap().pic, 7);
    let got = pictures_with(&p, &spec, &mut w, &mut a, &[true; 3]);
    assert_eq!(got, [7, 7, 3]);
    // A timer just set again (4, the clip's time) means the game stepped
    // after drawing: the picture before is the one on screen.
    a.seed(&p, &w, 0, Some(2), Some(4));
    assert_eq!(a.scene(&p, &spec, &w).bodies[0].unwrap().pic, 5);
    // A body or clip that isn't there is left alone.
    a.seed(&p, &w, 7, Some(1), Some(1));
}

#[test]
fn facing_left_mirrors_and_a_frame_flip_mirrors_again() {
    let (p, spec) = (present(), spec());
    let mut w = world(0);
    w.bodies[0].facing_left = true;
    let a = Animator::new();
    // Standing still: the game-clock clip, frame 0 then its mirrored twin.
    let d = |w: &World| a.frame(&p, w, 0).unwrap();
    w.frame = 0;
    assert!(d(&w).flip_x, "facing left mirrors");
    let mut both = w.clone();
    (both.bodies[0].facing_left, both.bodies[0].faced_both) = (false, true);
    assert!(d(&both).flip_x, "facing both ways draws as left");
    let mut a2 = Animator::new();
    for _ in 0..8 {
        w.frame += 1;
        a2.step(&p, &spec, &w);
    }
    let f = a2.frame(&p, &w, 0).unwrap();
    assert!(!f.flip_x, "the clip's flip undoes the facing flip");
}

#[test]
fn the_air_clip_wins_and_bodies_stand_on_their_bottom_edge() {
    let (p, spec) = (present(), spec());
    let mut w = world(0x20000);
    w.bodies[0].on_ground = false;
    let mut a = Animator::new();
    a.step(&p, &spec, &w);
    let s = a.scene(&p, &spec, &w);
    let d = s.bodies[0].unwrap();
    assert_eq!(d.pic, 9, "frame 4 in set 1");
    assert_eq!((d.x, d.y), (40, 100));
    assert_eq!(s.camera, [8, 0]);
    assert_eq!(s.level, Some(11));
    assert_eq!(s.clear, [1, 2, 3]);
}

#[test]
fn a_frozen_world_stops_own_clocks_and_cycles() {
    let (p, spec) = (present(), spec());
    let mut w = world(0x8000);
    let mut a = Animator::new();
    for _ in 0..4 {
        w.frame += 1;
        a.step(&p, &spec, &w);
    }
    let before = (a.frame(&p, &w, 0), a.clock());
    w.machines[0].state = 1;
    for _ in 0..20 {
        w.frame += 1;
        a.step(&p, &spec, &w);
    }
    assert_eq!((a.frame(&p, &w, 0), a.clock()), before);
}

#[test]
fn cycles_tint_masks_by_the_clock() {
    let (p, spec) = (present(), spec());
    let mut w = world(0);
    let mut a = Animator::new();
    let mut colors = Vec::new();
    for _ in 0..24 {
        w.frame += 1;
        a.step(&p, &spec, &w);
        colors.push(a.scene(&p, &spec, &w).tints[0].1[0]);
    }
    assert_eq!(colors[..7], [9; 7]);
    assert_eq!(colors[7..15], [8; 8]);
    assert_eq!(colors[15..23], [7; 8]);
    assert_eq!(colors[23], 9);
}

#[test]
fn hud_fields_show_counters_and_the_level_label() {
    let (p, spec) = (present(), spec());
    let w = world(0);
    let a = Animator::new();
    let s = a.scene(&p, &spec, &w);
    let hud = p.hud.as_ref().unwrap();
    assert_eq!(hud.fields[0].text(&w, &p.levels[0]), "000042");
    assert_eq!(hud.fields[1].text(&w, &p.levels[0]), "1-1");
    // The layer, then six digits and three characters of the label; the
    // text field's palette has no glyphs, so it draws nothing.
    assert_eq!(s.hud.len(), 1 + 6 + 3 + 1);
    assert_eq!(s.hud_sprites, 1);
    assert_eq!((s.hud[10].pic, s.hud[10].x, s.hud[10].y), (30, 88, 25));
    assert_eq!((s.hud[1].x, s.hud[1].y), (16, 24));
    assert_eq!((s.hud[7].x, s.hud[7].y), (19 * 8, 24));
    let mut big = world(0);
    big.counters[0] = 12_345_678;
    assert_eq!(hud.fields[0].text(&big, &p.levels[0]), "345678");
    let mut tens = hud.fields[0].clone();
    tens.divide = 10;
    assert_eq!(tens.text(&w, &p.levels[0]), "000004");
}

#[test]
fn the_screen_crops_overscan_and_shapes_pixels() {
    let p = present();
    assert_eq!(p.screen.visible(), [0, 8, 256, 224]);
    assert_eq!(p.screen.window(3), [877, 672]);
    assert_eq!(Screen::default().window(2), [512, 480]);
}

#[test]
fn present_text_round_trips() {
    let mut p = present();
    p.screens.push(crate::ScreenLook {
        states: vec![(0, 1)],
        counters: vec![(0, Some(16), Some(20))],
        picture: 2,
        clear: [1, 2, 3],
        sprites: vec![crate::ScreenSprite {
            pic: 3,
            at: [56, -8],
            counter: Some(0),
            step: [0, 16],
        }],
        from_line: Some(34),
    });
    let t = text::write(&p);
    assert!(t.starts_with("gp-present 23\n"));
    assert_eq!(text::read(&t).unwrap(), p);
    assert!(text::read("gp-present 4\n256").is_err());
    assert!(text::read(&format!("{t} 9")).is_err());
}

#[test]
fn presentation_never_changes_the_world() {
    // Stepping the animator and drawing reads the world only: its
    // canonical bytes are the same before and after.
    let (p, spec) = (present(), spec());
    let w = world(0x20000);
    let before = w.clone();
    let mut a = Animator::new();
    for _ in 0..50 {
        a.step(&p, &spec, &w);
        let _ = a.scene(&p, &spec, &w);
    }
    assert_eq!(w, before);
}

#[test]
fn a_held_clip_keeps_the_walk_picture_and_blinks_need_invulnerability() {
    let (mut p, spec) = (present(), spec());
    // In the air without a jump: hold the walk's pictures. Invulnerable:
    // blink between frame 0 and frame 3.
    p.kinds[0][0].as_mut().unwrap().clips.insert(
        0,
        Clip {
            when: When {
                ground: Some(false),
                jumped: Some(false),
                ..When::default()
            },
            frames: vec![frame(1), frame(2), frame(3)],
            timing: vec![(None, 4)],
            hold: true,
            ..Clip::default()
        },
    );
    p.kinds[0][0].as_mut().unwrap().clips.insert(
        0,
        Clip {
            when: When {
                invulnerable: Some(true),
                ..When::default()
            },
            frames: vec![frame(0), frame(3)],
            timing: vec![(None, 2)],
            ..Clip::default()
        },
    );
    let mut w = world(0x8000);
    let mut a = Animator::new();
    for _ in 0..5 {
        w.frame += 1;
        a.step(&p, &spec, &w);
    }
    let walking = a.frame(&p, &w, 0).unwrap().id;
    assert_eq!(walking, 2);
    // Walks off a ledge: the picture stays.
    w.bodies[0].on_ground = false;
    w.bodies[0].air = 1;
    for _ in 0..12 {
        w.frame += 1;
        a.step(&p, &spec, &w);
        assert_eq!(a.frame(&p, &w, 0).unwrap().id, walking);
    }
    // A jump's air clip is the old one.
    w.bodies[0].air = u32::MAX;
    a.step(&p, &spec, &w);
    assert_eq!(a.frame(&p, &w, 0).unwrap().id, 4);
    // Hurt: blinks.
    w.bodies[0].invuln = 10;
    let mut ids = Vec::new();
    for _ in 0..4 {
        a.step(&p, &spec, &w);
        ids.push(a.frame(&p, &w, 0).unwrap().id);
    }
    assert_eq!(ids, [0, 0, 3, 3]);
}

#[test]
fn a_frame_with_an_origin_is_placed_from_the_position_and_mirrors_about_the_axis() {
    let (mut p, spec) = (present(), spec());
    p.kinds[0][0].as_mut().unwrap().clips.clear();
    p.kinds[0][0].as_mut().unwrap().base = frame(5);
    let mut w = world(0);
    let a = Animator::new();
    let d = a.scene(&p, &spec, &w).bodies[0].unwrap();
    assert_eq!((d.x, d.y), (40 - 4, 100 - 20));
    // Facing left: the 16-wide picture turns about x + 8, so its left is
    // at 16 - (-4) - 16 = 4 from the position.
    w.bodies[0].facing_left = true;
    let d = a.scene(&p, &spec, &w).bodies[0].unwrap();
    assert!(d.flip_x);
    assert_eq!((d.x, d.y), (40 + 4, 100 - 20));
}

/// A `copy`, as DrawSmallPlatform's: a second picture 128 below, wrapped at
/// 256, and neither shows with its top above 32. Bodies keep their indices.
#[test]
fn a_copy_draws_a_second_picture_after_the_bodies() {
    let (mut p, spec) = (present(), spec());
    p.kinds[0][0].as_mut().unwrap().copy = Some([128, 256, 32]);
    let mut w = world(0);
    let a = Animator::new();
    let s = a.scene(&p, &spec, &w);
    assert_eq!(s.bodies.len(), w.bodies.len() + 1);
    assert_eq!(s.behind.len(), s.bodies.len());
    let (d, c) = (s.bodies[0].unwrap(), s.bodies[w.bodies.len()].unwrap());
    assert_eq!((c.x, c.y), (d.x, d.y + 128));
    // The copy wraps above the status bar's line: it doesn't show.
    w.bodies[0].y = n(150);
    let s = a.scene(&p, &spec, &w);
    assert_eq!(s.bodies.len(), w.bodies.len());
    assert!(s.bodies[0].is_some());
    // The first is above that line: only the copy shows.
    w.bodies[0].y = n(10);
    let s = a.scene(&p, &spec, &w);
    assert!(s.bodies[0].is_none());
    assert_eq!(s.bodies.len(), w.bodies.len() + 1);
}

#[test]
fn a_blink_hides_every_other_run_while_its_condition_holds() {
    let mut p = present();
    p.kinds[0][0].as_mut().unwrap().blink = Some((
        When {
            invulnerable: Some(true),
            ..When::default()
        },
        2,
    ));
    let mut w = world(0);
    let a = Animator::new();
    let shown = |w: &World| a.frame(&p, w, 0).is_some();
    let mut seen = Vec::new();
    for f in 0..6 {
        w.frame = f;
        seen.push(shown(&w));
    }
    assert_eq!(seen, [true; 6], "not invulnerable: always drawn");
    w.bodies[0].invuln = 30;
    seen.clear();
    for f in 0..6 {
        w.frame = f;
        seen.push(shown(&w));
    }
    assert_eq!(seen, [true, true, false, false, true, true]);
}

#[test]
fn a_cycle_with_a_counter_shows_the_color_the_counter_picks() {
    let (p, spec) = (present(), spec());
    let color = |c: i64| {
        let mut w = world(0);
        w.counters[1] = c;
        let a = Animator::new();
        a.scene(&p, &spec, &w).tints[1].1[0]
    };
    // Counter 7 less 1 is 6, which is 0 of 3: the clock plays no part.
    assert_eq!(color(7), 1);
    assert_eq!(color(8), 2);
    assert_eq!(color(9), 3);
    // Below zero it wraps, so a counter of 0 shows the last color.
    assert_eq!(color(0), 3);
    // The same clock gives the same color.
    let mut w = world(0);
    let mut a = Animator::new();
    for _ in 0..30 {
        w.frame += 1;
        a.step(&p, &spec, &w);
    }
    assert_eq!(a.scene(&p, &spec, &w).tints[1].1[0], 1);
}

#[test]
fn a_hud_sprite_is_drawn_over_the_fields_and_counted_as_a_sprite() {
    use crate::raster::{self, Image};
    let (p, spec) = (present(), spec());
    let w = world(0);
    let s = Animator::new().scene(&p, &spec, &w);
    // Every picture is a red pixel at its top left, but the sprite's (30)
    // is a 2 by 1 green strip.
    let mut pics: Vec<Option<Image>> = (0..40)
        .map(|_| {
            Some(Image {
                w: 1,
                h: 1,
                px: vec![[255, 0, 0, 255]],
            })
        })
        .collect();
    pics[30] = Some(Image {
        w: 2,
        h: 1,
        px: vec![[0, 255, 0, 255]; 2],
    });
    let f = raster::draw(&p, &w, &pics, &s, &["hero".to_owned()], &[]);
    let at = |x: usize, y: usize| y * f.w as usize + x;
    assert!(f.sprite[at(88, 25)] && f.sprite[at(89, 25)]);
    assert_eq!(f.rgb[at(88, 25)], [0, 255, 0]);
    // The HUD's other pictures aren't sprites.
    assert!(!f.sprite[at(16, 24)]);
}

#[test]
fn a_body_is_drawn_over_the_huds_fields_as_a_sprite_over_the_background() {
    // On the NES the status row is background and a sprite goes over it,
    // so a hero at the top of the stage stays whole over the score (the
    // user saw half of Rivet Rodeo's hero go behind it, 2026-11-04). A
    // body behind the background stays under the HUD's tiles; the HUD's
    // own sprites still go over the bodies.
    use crate::raster::{self, Image};
    let (p, spec) = (present(), spec());
    let w = world(0);
    let mut s = Animator::new().scene(&p, &spec, &w);
    let px = |c: [u8; 3]| {
        Some(Image {
            w: 1,
            h: 1,
            px: vec![[c[0], c[1], c[2], 255]],
        })
    };
    let mut pics: Vec<Option<Image>> = (0..40).map(|_| px([255, 0, 0])).collect();
    pics[31] = px([0, 0, 255]);
    let [cx, cy] = s.camera;
    // The HUD field's tile at (16, 24); the HUD's sprite at (88, 25).
    let on = |x: i64, y: i64| Draw {
        pic: 31,
        x: x + cx,
        y: y + cy,
        flip_x: false,
        flip_y: false,
    };
    s.bodies = vec![Some(on(16, 24)), Some(on(88, 25)), Some(on(19 * 8, 24))];
    s.behind = vec![false, false, true];
    s.front = vec![false; 3];
    s.name_table = vec![false; 3];
    let f = raster::draw(&p, &w, &pics, &s, &[], &[]);
    let at = |x: usize, y: usize| y * f.w as usize + x;
    assert_eq!(
        f.rgb[at(16, 24)],
        [0, 0, 255],
        "the body over the field's tile"
    );
    assert!(f.sprite[at(16, 24)]);
    assert_eq!(
        f.rgb[at(88, 25)],
        [255, 0, 0],
        "the HUD's sprite over the body"
    );
    assert_eq!(
        f.rgb[at(19 * 8, 24)],
        [255, 0, 0],
        "a body behind the background stays under the HUD's tile"
    );
}

/// The picture body 0 shows after each step at each speed.
fn pictures_at(p: &Present, spec: &Spec, speeds: &[i64]) -> Vec<u32> {
    let mut w = world(speeds[0]);
    let mut a = Animator::new();
    speeds
        .iter()
        .map(|&vx| {
            w.bodies[0].vx = Fx::from_raw(vx);
            w.frame += 1;
            a.step(p, spec, &w);
            a.scene(p, spec, &w).bodies[0].unwrap().pic
        })
        .collect()
}

#[test]
fn a_clip_timed_by_the_speed_before_picks_its_time_a_frame_late() {
    let spec = spec();
    // Fast for two frames, then slow: the walk's picture changes on the
    // third frame and takes the slow time there, or the fast one when it
    // reads the speed of the frame before (Super Mario Bros.'
    // GetPlayerAnimSpeed runs before ImposeFriction).
    let (fast, slow) = (0x20000, 0x8000);
    let speeds = [fast, fast, slow, slow, slow, slow, slow];
    let p = present();
    assert_eq!(pictures_at(&p, &spec, &speeds), [3, 3, 5, 5, 5, 5, 7]);
    let mut before = present();
    before.kinds[0][0].as_mut().unwrap().clips[1].speed_before = true;
    assert_eq!(pictures_at(&before, &spec, &speeds), [3, 3, 5, 5, 7, 7, 7]);
}

#[test]
fn a_game_clock_starts_at_the_worlds_frame() {
    // A world loaded partway through a game shows its game-clock clips in
    // the game's phase: the idle clip flips on frames 8 to 15.
    let (p, spec) = (present(), spec());
    let flip_at = |frame: u64| {
        let mut w = world(0);
        w.frame = frame;
        let mut a = Animator::new();
        a.step(&p, &spec, &w);
        a.scene(&p, &spec, &w).bodies[0].unwrap().flip_x
    };
    assert!(!flip_at(1));
    assert!(flip_at(8));
    assert!(flip_at(15));
    assert!(!flip_at(16));
}

#[test]
fn a_body_behind_the_level_shows_only_where_the_level_draws_nothing() {
    use crate::raster::{self, Image};
    let p = Present {
        screen: Screen {
            size: [4, 2],
            ..Screen::default()
        },
        pictures: vec![
            Picture {
                file: "level.png".into(),
                size: [4, 2],
            },
            Picture {
                file: "body.png".into(),
                size: [4, 2],
            },
        ],
        ..Present::default()
    };
    // The level draws its left half; the body is red all over.
    let level = Image {
        w: 4,
        h: 2,
        px: (0..8)
            .map(|i| {
                if i % 4 < 2 {
                    [0, 200, 0, 255]
                } else {
                    [0, 0, 0, 0]
                }
            })
            .collect(),
    };
    let body = Image {
        w: 4,
        h: 2,
        px: vec![[255, 0, 0, 255]; 8],
    };
    let pics = vec![Some(level), Some(body)];
    let scene = |behind: bool| Scene {
        clear: [1, 2, 3],
        level: Some(0),
        bodies: vec![Some(Draw {
            pic: 1,
            x: 0,
            y: 0,
            flip_x: false,
            flip_y: false,
        })],
        behind: vec![behind],
        ..Scene::default()
    };
    let w = World::default();
    let names = ["plant".to_owned()];
    let f = raster::draw(&p, &w, &pics, &scene(true), &names, &[]);
    assert_eq!(f.rgb[0], [0, 200, 0], "the level covers it");
    assert_eq!(f.rgb[3], [255, 0, 0], "it shows past the level");
    // Its pixels are a sprite's everywhere, as the NES's sprite layer
    // counts a sprite behind the background.
    assert!(f.sprite.iter().all(|s| *s));
    let front = raster::draw(&p, &w, &pics, &scene(false), &names, &[]);
    assert!(front.rgb.iter().all(|c| *c == [255, 0, 0]));
    // The scene says which bodies are behind, from their look.
    let mut q = present();
    q.kinds[0][0].as_mut().unwrap().behind = true;
    let s = Animator::new().scene(&q, &spec(), &world(0));
    assert_eq!(s.behind, vec![true]);
}

#[test]
fn present_text_keeps_behind_and_the_speed_before() {
    let mut p = present();
    let look = p.kinds[0][0].as_mut().unwrap();
    look.behind = true;
    look.clips[1].speed_before = true;
    assert_eq!(text::read(&text::write(&p)).unwrap(), p);
}

/// The test spec with a second kind, `defeated`, that the hero's defeat
/// leaves, and a present where `defeated` draws the body it replaced.
fn replacing(leaves: Option<u32>) -> (Spec, Present) {
    let mut spec = spec();
    spec.kinds[0].health = Some(gp_sim::spec::Health {
        counter: None,
        points: 1,
        invulnerable: 0,
        shield: false,
        damage_from: Vec::new(),
        defeat: Some(gp_sim::spec::Defeat {
            becomes: 1,
            rise: n(0),
            speed: n(0),
        }),
    });
    spec.kinds.push(kind("defeated"));
    let mut p = present();
    p.kinds[0][0].as_mut().unwrap().leaves = leaves;
    p.kinds.push(vec![Some(Look {
        base: frame(3),
        replaced: Some(true),
        ..Look::default()
    })]);
    (spec, p)
}

/// Steps the world one frame with the hero defeated and its successor
/// added at its place, as `health.defeat` does.
fn defeat(w: &mut World) {
    let old = w.bodies[0];
    w.bodies[0].alive = false;
    w.bodies.push(Body {
        kind: 1,
        y: old.y.add(n(-2)),
        alive: true,
        ..old
    });
}

#[test]
fn a_defeated_body_draws_the_one_it_replaced_turned_over() {
    let (spec, p) = replacing(None);
    let mut w = world(0);
    w.bodies[0].facing_left = true;
    let mut a = Animator::new();
    a.step(&p, &spec, &w);
    let before = a.scene(&p, &spec, &w).bodies[0].unwrap();
    defeat(&mut w);
    a.step(&p, &spec, &w);
    let s = a.scene(&p, &spec, &w);
    assert!(s.bodies[0].is_none(), "the old body is gone");
    let d = s.bodies[1].expect("the successor is drawn");
    assert_eq!(d.pic, before.pic, "the old body's picture");
    assert_eq!((d.flip_x, d.flip_y), (before.flip_x, !before.flip_y));
    assert_eq!(d.y, before.y - 2, "placed where the successor is");
    // Later frames keep it, whatever the successor does.
    w.bodies[1].vx = n(2);
    a.step(&p, &spec, &w);
    assert_eq!(a.scene(&p, &spec, &w).bodies[1].unwrap().pic, before.pic);
}

#[test]
fn a_kind_with_its_own_defeated_frame_leaves_that_unturned() {
    let (spec, p) = replacing(Some(4));
    let mut w = world(0);
    let mut a = Animator::new();
    a.step(&p, &spec, &w);
    defeat(&mut w);
    a.step(&p, &spec, &w);
    let d = a.scene(&p, &spec, &w).bodies[1].unwrap();
    assert!(p.frames[4].contains(&d.pic));
    assert!(!d.flip_y);
}

#[test]
fn a_successor_with_nothing_to_copy_draws_its_own_frame() {
    let (spec, p) = replacing(None);
    let mut w = world(0);
    w.bodies[0].kind = 1;
    let mut a = Animator::new();
    a.step(&p, &spec, &w);
    let d = a.scene(&p, &spec, &w).bodies[0].unwrap();
    assert!(p.frames[3].contains(&d.pic));
}

#[test]
fn a_body_added_keeps_the_others_animations() {
    let p = present();
    let spec = spec();
    let w = world(0x10000);
    let mut a = Animator::new();
    for _ in 0..5 {
        a.step(&p, &spec, &w);
    }
    let walking = a.frame(&p, &w, 0).unwrap();
    assert_ne!(walking.id, 1, "the walk has stepped past its first picture");
    let mut more = w.clone();
    more.bodies.push(w.bodies[0]);
    a.step(&p, &spec, &more);
    // One more frame of the same walk: not back at the first picture.
    let mut b = Animator::new();
    for _ in 0..6 {
        b.step(&p, &spec, &w);
    }
    assert_eq!(a.frame(&p, &more, 0), b.frame(&p, &w, 0));
}

#[test]
fn a_clip_can_test_a_hook_word() {
    let mut p = present();
    let look = p.kinds[0][0].as_mut().unwrap();
    look.clips.insert(
        0,
        Clip {
            when: When {
                word: Some((2, None, Some(16))),
                ..When::default()
            },
            frames: vec![frame(5)],
            ..Clip::default()
        },
    );
    let spec = spec();
    let mut w = world(0);
    w.bodies[0].ext[2] = 16;
    let mut a = Animator::new();
    a.step(&p, &spec, &w);
    assert_ne!(a.frame(&p, &w, 0).unwrap().id, 5);
    w.bodies[0].ext[2] = 15;
    a.step(&p, &spec, &w);
    assert_eq!(a.frame(&p, &w, 0).unwrap().id, 5);
}

#[test]
fn present_text_keeps_replaced_leaves_and_word_tests() {
    let (_, mut p) = replacing(Some(4));
    p.kinds[0][0].as_mut().unwrap().clips[0].when.word = Some((2, Some(1), Some(16)));
    let text = crate::text::write(&p);
    assert_eq!(crate::text::read(&text).unwrap(), p);
}

#[test]
fn a_body_in_front_is_drawn_over_the_others() {
    let mut p = present();
    p.kinds[0][0].as_mut().unwrap().front = true;
    let mut spec = spec();
    spec.kinds.push(kind("box"));
    p.kinds.push(vec![Some(Look {
        base: frame(3),
        ..Look::default()
    })]);
    let mut w = world(0);
    let mut other = w.bodies[0];
    other.kind = 1;
    w.bodies.push(other);
    let mut a = Animator::new();
    a.step(&p, &spec, &w);
    let s = a.scene(&p, &spec, &w);
    assert_eq!(s.front, vec![true, false]);
    let names = ["hero".to_string(), "box".to_string()];
    let pics: Vec<Option<raster::Image>> = (0..12)
        .map(|i| {
            Some(raster::Image {
                w: 16,
                h: if i == 11 { 24 } else { 16 },
                px: vec![[i as u8 * 10, 0, 0, 255]; if i == 11 { 384 } else { 256 }],
            })
        })
        .collect();
    let f = raster::draw(&p, &w, &pics, &s, &names, &[]);
    let d = s.bodies[0].unwrap();
    let at = ((d.y - s.camera[1]) as u32 * f.w + (d.x - s.camera[0]) as u32) as usize;
    assert_eq!(
        f.rgb[at][0],
        d.pic as u8 * 10,
        "the hero's picture is on top"
    );
}

/// A level of 4 by 2 cells of 16 pixels: a coin block at cell 1, a brick
/// at cell 3 and a coin at cell 6.
fn cell_spec() -> Spec {
    use gp_sim::spec::{Level, Tile};
    let mut s = spec();
    s.events = vec!["block.coin".into()];
    s.levels = vec![Level {
        name: "main".into(),
        tile_size: 16,
        size: [4, 2],
        tiles: vec![
            Tile::Empty,
            Tile::CoinBlock,
            Tile::Empty,
            Tile::Brick,
            Tile::Solid,
            Tile::Solid,
            Tile::Coin,
            Tile::Solid,
        ],
        surfaces: Vec::new(),
        ladders: Vec::new(),
        ladder_tops: Vec::new(),
        ladder_stops: Vec::new(),
        ground: Vec::new(),
        ground_feet: [0, 0],
        ground_land: None,
        ground_boxes: false,
        ground_land_rising: false,
        ground_bodies: Vec::new(),
        pickups: Vec::new(),
        spawns: Vec::new(),
        kill_below: None,
        pit_event: None,
        goal_x: None,
        goal_under: None,
        hazard: None,
        water: None,
        ice: None,
        origin: [Fx::ZERO; 2],
        camera: None,
        despawn_ahead: None,
        respawn: false,
        counters: Vec::new(),
        next: None,
        spawn_ahead: None,
        despawn_behind: None,
        spawn_slots: None,
        despawn_by_position: false,
        despawn_after_move: false,
        spawn_in_order: false,
        appear_still: false,
        slot_rest: false,
        hard: false,
        draw: None,
        exits: Vec::new(),
        hit_widths: Vec::new(),
        scroll_stop: None,
        loop_back: None,
        halfway: None,
    }]
    .into();
    s
}

/// The test presentation with a bump of speed -2 and pull 0.3125, and an
/// effect on event 0 at the hit: 4 frames rising at -5 until its speed is
/// 5, then a still picture for 3 frames.
fn cell_present() -> Present {
    let mut p = present();
    let fx = |v: i64| v << 16;
    p.bump = Some(Bump {
        motion: Motion {
            speed: fx(-2),
            gravity: 0x5000,
            max_fall: None,
        },
        offset: [0, 1],
    });
    p.effects = vec![Effect {
        event: 0,
        at_hit: true,
        offset: [5, -16],
        phases: vec![
            Phase {
                frames: vec![frame(0), frame(1)],
                every: 2,
                motion: Some(Motion {
                    speed: fx(-5),
                    gravity: 0x5000,
                    max_fall: Some(fx(6)),
                }),
                until_speed: Some(fx(5)),
                ..Phase::default()
            },
            Phase {
                frames: vec![frame(2)],
                every: 1,
                length: Some(3),
                ..Phase::default()
            },
        ],
    }];
    p
}

#[test]
fn a_bump_rises_and_falls_back_to_its_cell() {
    let m = Motion {
        speed: -2 << 16,
        gravity: 0x5000,
        max_fall: None,
    };
    // Super Mario Bros.' bouncing block (BumpBlock, ImposeGravityBlock):
    // back at its cell on the 14th frame.
    let path: Vec<i64> = (1..=14).map(|n| m.offset(n)).collect();
    assert_eq!(
        path,
        [-2, -4, -6, -7, -7, -8, -8, -8, -7, -6, -5, -4, -2, 0]
    );
}

#[test]
fn changed_cells_draw_their_new_kind_and_a_held_cell_bumps() {
    use gp_sim::spec::Tile;
    let (p, spec) = (cell_present(), cell_spec());
    let mut w = world(0);
    w.level = 0;
    let mut a = Animator::new();
    a.step(&p, &spec, &w);
    assert!(a.scene(&p, &spec, &w).cells.is_empty());
    // The coin taken: empty, drawn with the empty kind's picture.
    w.set_tile(&spec, 2 * 16, 16, Tile::Empty);
    // The coin block hit: solid while held, then solid for good.
    w.set_tile(&spec, 16, 0, Tile::Solid);
    w.held_cells.push((1, Tile::Solid, 15));
    // The brick bumped: held, back to a brick after.
    w.set_tile(&spec, 3 * 16, 0, Tile::Solid);
    w.held_cells.push((3, Tile::Brick, 15));
    a.step(&p, &spec, &w);
    let s = a.scene(&p, &spec, &w);
    let at = |d: &Draw| (d.pic, d.x, d.y);
    let cells: Vec<_> = s.cells.iter().map(at).collect();
    assert_eq!(cells, [(24, 16, 0), (24, 48, 0), (20, 32, 16)]);
    // The bumps, after the one body: the block shows what it becomes,
    // the brick its own picture, both 2 pixels up on the hit's frame (and
    // the bump's offset, a pixel down).
    let bumps: Vec<_> = s.bodies[1..].iter().flatten().map(at).collect();
    assert_eq!(bumps, [(23, 16, -1), (25, 48, -1)]);
    a.step(&p, &spec, &w);
    let s = a.scene(&p, &spec, &w);
    assert_eq!(s.bodies[1].map(|d| d.y), Some(-3));
    // Released: the block is drawn used, the brick as built.
    w.held_cells.clear();
    w.set_tile(&spec, 3 * 16, 0, Tile::Brick);
    a.step(&p, &spec, &w);
    let s = a.scene(&p, &spec, &w);
    let cells: Vec<_> = s.cells.iter().map(at).collect();
    assert_eq!(cells, [(22, 16, 0), (20, 32, 16)]);
    assert_eq!(s.bodies.len(), 1);
}

/// Platformer `level.pickup_cells`, on made-up numbers: a taken place's
/// cells show the empty kind's picture, and a place not taken shows none.
#[test]
fn a_taken_pickups_cells_draw_as_empty() {
    let p = cell_present();
    let mut spec = cell_spec();
    let mut w = world(0);
    w.level = 0;
    w.counters = vec![0, 0];
    spec.levels[0].pickups = vec![gp_sim::spec::PickupGroup {
        places: vec![[0, 0], [16, 0]],
        counters: vec![0, 1],
        offset: [0, 0],
        window: [0, 0],
        event: None,
        hole: None,
        cells: vec![vec![4, 5], vec![7]],
        by_hook: false,
    }];
    let mut a = Animator::new();
    a.step(&p, &spec, &w);
    assert!(a.scene(&p, &spec, &w).cells.is_empty());
    w.counters[1] = 1;
    a.step(&p, &spec, &w);
    let s = a.scene(&p, &spec, &w);
    let cells: Vec<_> = s.cells.iter().map(|d| (d.pic, d.x, d.y)).collect();
    assert_eq!(cells, [(20, 48, 16)]);
}

#[test]
fn an_event_starts_an_effect_at_the_hit_that_runs_its_phases_then_goes() {
    use gp_sim::spec::Tile;
    let (p, spec) = (cell_present(), cell_spec());
    let mut w = world(0);
    let mut a = Animator::new();
    a.step(&p, &spec, &w);
    w.set_tile(&spec, 16, 0, Tile::Solid);
    w.held_cells.push((1, Tile::Solid, 15));
    a.step(&p, &spec, &w);
    // Not an effect's event: nothing starts.
    a.fired_kinds(&p, &spec, &w, &[(1, None)]);
    assert_eq!(a.scene(&p, &spec, &w).bodies.len(), 2);
    a.fired_kinds(&p, &spec, &w, &[(0, Some(0))]);
    let s = a.scene(&p, &spec, &w);
    // The bump, then the effect: at the cell's x plus 5 and y less 16,
    // moved once by its speed.
    let e = s.bodies[2].unwrap();
    assert_eq!((e.x, e.y), (21, -21));
    let mut ys = vec![e.y];
    let mut pics = vec![e.pic];
    for _ in 0..40 {
        w.frame += 1;
        a.step(&p, &spec, &w);
        let s = a.scene(&p, &spec, &w);
        match s.bodies.last().copied().flatten() {
            Some(d) if s.bodies.len() > 1 && d.x == 21 => {
                ys.push(d.y);
                pics.push(d.pic);
            }
            _ => break,
        }
    }
    // Up and back down for 31 frames, from speed -5 by 0.3125 a frame;
    // the frame its speed reaches 5 shows the second phase's picture
    // (frame 2's), which shows 3 frames, then it's gone.
    assert_eq!(pics.len(), 31 + 3);
    let second = p.frames[2][1];
    assert!(pics[..31].iter().all(|&x| x != second));
    assert!(pics[31..].iter().all(|&x| x == second));
    assert_eq!(ys[..4], [-21, -26, -31, -35]);
}

#[test]
fn present_text_keeps_cells_bumps_and_effects() {
    let p = cell_present();
    assert_eq!(text::read(&text::write(&p)).unwrap(), p);
}

#[test]
fn an_effect_stops_while_frozen_unless_it_goes_on() {
    use gp_sim::spec::Tile;
    let spec = cell_spec();
    let ys = |on: bool| {
        let mut p = cell_present();
        p.effects[0].phases[0].while_frozen = on;
        let mut w = world(0);
        let mut a = Animator::new();
        a.step(&p, &spec, &w);
        w.set_tile(&spec, 16, 0, Tile::Solid);
        w.held_cells.push((1, Tile::Solid, 15));
        a.step(&p, &spec, &w);
        a.fired_kinds(&p, &spec, &w, &[(0, None)]);
        // The game machine's frozen state, as Mario's death holds it.
        w.machines[0].state = 1;
        let mut out = Vec::new();
        for _ in 0..3 {
            w.frame += 1;
            a.step(&p, &spec, &w);
            out.push(
                a.scene(&p, &spec, &w)
                    .bodies
                    .last()
                    .copied()
                    .flatten()
                    .unwrap()
                    .y,
            );
        }
        out
    };
    assert_eq!(ys(false), [-21, -21, -21]);
    assert_eq!(ys(true), [-26, -31, -35]);
}

#[test]
fn a_clip_can_test_counters_and_more_words() {
    let mut p = present();
    let look = p.kinds[0][0].as_mut().unwrap();
    look.clips.insert(
        0,
        Clip {
            when: When {
                word: Some((2, Some(1), None)),
                words: vec![(3, None, Some(5))],
                counters: vec![(1, Some(7), Some(8)), (0, Some(40), None)],
                ..When::default()
            },
            frames: vec![frame(5)],
            ..Clip::default()
        },
    );
    let spec = spec();
    let mut w = world(0);
    w.bodies[0].ext[2] = 1;
    let mut a = Animator::new();
    a.step(&p, &spec, &w);
    // Counters 42 and 7 and both words hold.
    assert_eq!(a.frame(&p, &w, 0).unwrap().id, 5);
    w.counters[1] = 8;
    a.step(&p, &spec, &w);
    assert_ne!(a.frame(&p, &w, 0).unwrap().id, 5);
    w.counters[1] = 7;
    w.bodies[0].ext[3] = 5;
    a.step(&p, &spec, &w);
    assert_ne!(a.frame(&p, &w, 0).unwrap().id, 5);
}

#[test]
fn a_clip_frame_shows_for_its_own_length() {
    let mut p = present();
    let look = p.kinds[0][0].as_mut().unwrap();
    look.clips.insert(
        0,
        Clip {
            frames: vec![frame(1), frame(2)],
            lengths: vec![3, 1],
            ..Clip::default()
        },
    );
    let spec = spec();
    let mut w = world(0);
    let mut a = Animator::new();
    let mut ids = Vec::new();
    for _ in 0..9 {
        w.frame += 1;
        a.step(&p, &spec, &w);
        ids.push(a.frame(&p, &w, 0).unwrap().id);
    }
    assert_eq!(ids, [1, 1, 1, 2, 1, 1, 1, 2, 1]);
    // On the game's clock the lengths add up to the cycle.
    let c = &mut p.kinds[0][0].as_mut().unwrap().clips[0];
    c.game_clock = true;
    assert_eq!(
        (0..5).map(|t| c.at_clock(t)).collect::<Vec<_>>(),
        [0, 0, 0, 1, 0]
    );
}

#[test]
fn present_text_keeps_counter_tests_lengths_and_the_name_table() {
    let (_, mut p) = replacing(Some(4));
    let look = p.kinds[0][0].as_mut().unwrap();
    look.name_table = true;
    look.clips[0].when.words = vec![(1, Some(2), None)];
    look.clips[0].when.counters = vec![(0, None, Some(3))];
    look.clips[0].lengths = vec![4];
    let text = crate::text::write(&p);
    assert_eq!(crate::text::read(&text).unwrap(), p);
}

/// Donkey Kong's points ($CFC6, $CFA8): a smash's 500 waits, drawing
/// nothing, until the barrel goes, then shows until the frame after its
/// timer (counter 1 here) reads 0.
#[test]
fn an_effect_waits_for_its_entity_to_go_and_ends_after_its_counter_reads_0() {
    let spec = cell_spec();
    let mut p = cell_present();
    p.effects[0].at_hit = false;
    p.effects[0].offset = [0, 1];
    p.effects[0].phases = vec![
        Phase {
            frames: vec![frame(0)],
            every: 1,
            until_gone: true,
            while_frozen: true,
            ..Phase::default()
        },
        Phase {
            frames: vec![frame(2)],
            every: 1,
            until_counter: Some(1),
            while_frozen: true,
            ..Phase::default()
        },
    ];
    let first = p.frames[0][1];
    let second = p.frames[2][1];
    let mut w = world(0);
    let mut a = Animator::new();
    a.step(&p, &spec, &w);
    a.fired_kinds(&p, &spec, &w, &[(0, Some(0))]);
    let shown = |a: &Animator, w: &World| {
        let s = a.scene(&p, &spec, w);
        (s.bodies.len() > 1)
            .then(|| s.bodies.last().copied().flatten())
            .flatten()
            .map(|d| (d.pic, d.x, d.y))
    };
    assert_eq!(shown(&a, &w), Some((first, 40, 101)));
    let mut seen = Vec::new();
    for t in 1..=6 {
        w.frame += 1;
        if t == 2 {
            w.bodies[0].alive = false;
            w.counters[1] = 1;
        }
        if t == 4 {
            w.counters[1] = 0;
        }
        a.step(&p, &spec, &w);
        seen.push(shown(&a, &w).map(|d| d.0));
    }
    // Gone on frame 2: the second picture from then; the counter at 0 on
    // frame 4 still shows, and frame 5 doesn't.
    assert_eq!(
        seen,
        [
            Some(first),
            Some(second),
            Some(second),
            Some(second),
            None,
            None
        ]
    );
    assert_eq!(text::read(&text::write(&p)).unwrap(), p);
}

#[test]
fn no_sprite_shows_while_the_sprite_layer_is_off() {
    let mut p = present();
    p.sprites_off = vec![(0, 1)];
    let spec = spec();
    let mut w = world(0);
    let mut a = Animator::new();
    a.step(&p, &spec, &w);
    assert!(a.scene(&p, &spec, &w).bodies[0].is_some());
    w.machines[0].state = 1;
    assert!(a.scene(&p, &spec, &w).bodies[0].is_none());
    // A body drawn as name-table tiles is the background's, and stays.
    p.kinds[0][0].as_mut().unwrap().name_table = true;
    assert!(a.scene(&p, &spec, &w).bodies[0].is_some());
    let text = crate::text::write(&p);
    assert_eq!(crate::text::read(&text).unwrap(), p);
}
