//! A small model that uses every part of core 0.3.0 and platformer 0.3.0 (0.4.0 models)
//! the interpreter runs beyond Moss Hop's: enemies spawned by the view, a
//! walking and falling enemy with its own state machine that removes
//! itself, and a player with skids, jump bands, coyote time and a jump
//! buffer. Its lifted assets are made here, so nothing comes from a ROM.

use gp_runtime::{InputFrame, Runtime, Value};

use crate::{Interp, Lifted};

const MODEL: &str = r#"
[model]
schema = "0.4.0"
name = "tiny"
title = "Tiny"
view = "side"
frame_rate = "60"
systems = ["rules", "entity_states", "counters", "spawn", "platformer.player", "platformer.enemies", "platformer.contact", "exits", "camera"]

[model.extensions]
platformer = "0.4.0"

[input]
device = "nes-pad"

[input.actions]
jump = "a"
left = "left"
right = "right"
run = "b"

[assets.area]
kind = "level-grid"
role = "layout"

[assets.area.from]
source = "tiny"
asset = "grid"

[assets.blocks]
kind = "metatiles"
role = "tileset"
item_roles = ["sky", "terrain"]

[assets.blocks.from]
source = "tiny"
asset = "blocks"

[assets.foes]
kind = "table"
role = "spawns"

[assets.foes.from]
source = "tiny"
asset = "foes"

[entities.hero]
role = "player"
size = [16, 16]
tile_collision = true

[entities.hero.hitbox]
x = 0
y = 0
w = 16
h = 16

[entities.hero.run]
walk_speed = 1
run_speed = 3
run_action = "run"
acceleration = "0.25"
deceleration = "0.25"
skid_deceleration = "0.5"
air_control = true

[entities.hero.jump]
action = "jump"
speed = -6
gravity = "0.5"
max_fall = 4
coyote_frames = 3
buffer_frames = 4

[[entities.hero.jump.bands]]
below = 2
speed = -4
gravity = "0.5"

[entities.walker]
role = "enemy"
size = [16, 16]
tile_collision = true

[entities.walker.hitbox]
x = 0
y = 0
w = 16
h = 16

[entities.walker.walk]
speed = 1

[entities.walker.fall]
gravity = "0.5"
max_fall = 4

[entities.walker.contact]
stomp_depth = 8
bounce = -3
keep_on_stomp = true
otherwise = "kill-player"

[entities.walker.machine]
initial = "walking"

[[entities.walker.machine.states.walking.transition]]
on = "enemy.stomped"
to = "flat"

[entities.walker.machine.states.flat.set.walk]
speed = 0

[entities.walker.machine.states.flat.set.contact]
otherwise = "nothing"

[[entities.walker.machine.states.flat.transition]]
after = 10
to = "flat"
do = ["remove"]

[levels.one]
size = [64, 15]
tile_size = 16
grid = "area"
tileset = "blocks"
solid_property = "solid"
kill_below = 240
spawn_ahead = 32
despawn_behind = 64

[[levels.one.spawn]]
entity = "hero"
x = 40
y = 192

[[levels.one.spawn]]
entity = "walker"
table = "foes"
y = 184

[levels.one.spawn.columns]
x = "x"

[cameras.main]
follow = "hero"
axis = "x"
size = [256, 240]
lead = [112, 0]
clamp_to_level = true
scroll_back = false

[rules.game]
initial = "playing"

[[rules.game.states.playing.transition]]
on = "player.died"
to = "dead"

[rules.game.states.dead]
freeze = true
"#;

/// The asset map: a grid, its two metatiles, and `foes` rows of walkers.
fn map_text(foes: usize) -> String {
    format!(
        r#"
schema = 1
source = "tiny"
sha256 = "0000000000000000000000000000000000000000000000000000000000000000"

[[asset]]
name = "blocks"
format = "metatiles"
region = "prg"
offset = 0x100
count = 2
names = ["sky", "floor"]
property = [{{ name = "solid", region = "prg", offset = 0x108 }}]

[[asset]]
name = "grid"
format = "level-grid"
region = "prg"
offset = 0x200
width = 64
height = 15
legend = ".#"
metatiles = "blocks"

[[asset]]
name = "foes"
format = "table"
count = {foes}
column = [{{ name = "x", region = "prg", offset = 0xF00, hi_offset = 0xF10 }}]
"#
    )
}

/// The lifted files: floor 2 everywhere, a pipe of height 2 at column 20,
/// a hole of 2 at column 40; a walker at x 128 and one at x 768. A test
/// can leave out the pipe and the near walker, and without the pipe the
/// hole is 3 wide.
fn lifted() -> Lifted {
    lifted_with(true, true)
}

fn lifted_with(pipe: bool, near: bool) -> Lifted {
    let mut rows = vec![vec![b'.'; 64]; 15];
    for row in &mut rows[13..] {
        row.fill(b'#');
    }
    if pipe {
        for row in &mut rows[11..13] {
            row[20..22].fill(b'#');
        }
    }
    let hole = if pipe { 2 } else { 3 };
    for row in &mut rows[13..] {
        row[40..40 + hole].fill(b'.');
    }
    let rows: Vec<String> = rows
        .iter()
        .map(|r| format!("  \"{}\",", String::from_utf8_lossy(r)))
        .collect();
    let xs: Vec<u32> = if near { vec![128, 768] } else { vec![768] };
    let foes: String = xs.iter().map(|x| format!("[[row]]\nx = {x}\n")).collect();
    let mut l = Lifted::default();
    l.files.insert("assets.toml".into(), map_text(xs.len()));
    l.files.insert(
        "grid.toml".into(),
        format!(
            "schema = 1\nasset = \"grid\"\nrows = [\n{}\n]\n",
            rows.join("\n")
        ),
    );
    l.files.insert(
        "blocks.toml".into(),
        "schema = 1\nasset = \"blocks\"\n[[metatile]]\ntiles = [0, 0, 0, 0]\nsolid = 0\n[[metatile]]\ntiles = [1, 1, 1, 1]\nsolid = 1\n".into(),
    );
    l.files.insert(
        "foes.toml".into(),
        format!("schema = 1\nasset = \"foes\"\n{foes}"),
    );
    l
}

pub(crate) fn load() -> Interp {
    load_with(lifted())
}

fn load_with(l: Lifted) -> Interp {
    let m = gp_model::Model::parse(MODEL).unwrap();
    Interp::load(&m, &l).unwrap()
}

fn pads(buttons: u8, n: usize) -> Vec<InputFrame> {
    vec![InputFrame::pads([buttons, 0, 0, 0]); n]
}

fn get(rt: &Interp, p: &str) -> Value {
    let e = rt.query(p).unwrap();
    assert_eq!(e.len(), 1, "{p}");
    e[0].value.clone()
}

fn num(rt: &Interp, p: &str) -> i64 {
    match get(rt, p) {
        Value::Fixed(raw) => raw >> 16,
        Value::Int(n) => n,
        v => panic!("{p} is {v}"),
    }
}

const RIGHT: u8 = 0x80;
const LEFT: u8 = 0x40;
const A: u8 = 0x01;
const B: u8 = 0x02;

#[test]
fn the_grid_builds_the_level() {
    let rt = load();
    let level = &rt.spec().levels[0];
    let solid = |col: i64, row: i64| level.solid_at(col * 16, row * 16);
    assert!(solid(0, 13) && solid(0, 14) && !solid(0, 12));
    assert!(solid(20, 11) && solid(21, 12), "the pipe");
    assert!(
        !solid(40, 13) && !solid(41, 14) && solid(42, 13),
        "the hole"
    );
}

#[test]
fn enemies_spawn_by_the_view_walk_toward_the_player_and_fall() {
    let mut rt = load();
    assert_eq!(rt.spec().levels[0].spawns.len(), 3);
    assert_eq!(get(&rt, "entities.walker[0].waiting"), Value::Flag(false));
    assert_eq!(get(&rt, "entities.walker[1].waiting"), Value::Flag(true));
    assert_eq!(num(&rt, "entities.walker[1].x"), 768);
    // The near walker faces the player, falls to the floor and walks left.
    assert_eq!(
        get(&rt, "entities.walker[0].facing"),
        Value::Text("left".into())
    );
    rt.step(&pads(0, 20)).unwrap();
    assert_eq!(num(&rt, "entities.walker[0].y"), 192);
    assert_eq!(get(&rt, "entities.walker[0].on_ground"), Value::Flag(true));
    assert!(num(&rt, "entities.walker[0].x") < 128);
    // The far one stays waiting while the view is far from it.
    assert_eq!(get(&rt, "entities.walker[1].waiting"), Value::Flag(true));
}

#[test]
fn an_enemy_stands_on_its_tile_box_not_its_contact_box() {
    // A small contact box inside the frame, as Super Mario Bros.' goomba
    // has, and the box the game looks for ground and walls with: its foot
    // 24 below the position. Without a tile box the contact box's bottom
    // touches the floor, and the body stands 4 lower than the game's
    // position, so a frame captured from that position draws 4 into the
    // floor.
    let small = MODEL.replacen(
        "[entities.walker.hitbox]\nx = 0\ny = 0\nw = 16\nh = 16",
        "[entities.walker.hitbox]\nx = 3\ny = 14\nw = 10\nh = 6",
        1,
    );
    assert_ne!(small, MODEL);
    let feet = small.replacen(
        "[entities.walker.walk]",
        "[entities.walker.tile_hitbox]\nx = 0\ny = 20\nw = 17\nh = 4\n\n[entities.walker.walk]",
        1,
    );
    for (model, y) in [(&small, 208 - 20), (&feet, 208 - 24)] {
        let m = gp_model::Model::parse(model).unwrap();
        let mut rt = Interp::load(&m, &lifted()).unwrap();
        rt.step(&pads(0, 20)).unwrap();
        assert_eq!(get(&rt, "entities.walker[0].on_ground"), Value::Flag(true));
        assert_eq!(num(&rt, "entities.walker[0].y"), y);
    }
}

#[test]
fn a_stomp_flattens_the_walker_and_its_machine_removes_it() {
    // Jump as the walker comes near, trying each distance until one lands
    // on it.
    let stomped = (56..100).find_map(|at| {
        let mut rt = load();
        let cond = format!("entities.walker[0].x <= {at}");
        assert!(rt.run_until(&cond, &pads(0, 1), 200).unwrap().met);
        for f in 0..40u64 {
            rt.step(&pads(if f < 3 { A } else { 0 }, 1)).unwrap();
            if get(&rt, "entities.walker[0].state") == Value::Text("flat".into()) {
                return Some(rt);
            }
        }
        None
    });
    let mut rt = stomped.expect("some timing stomps the walker");
    let flat = rt.frame();
    assert_eq!(get(&rt, "rules.game.state"), Value::Text("playing".into()));
    assert_eq!(
        get(&rt, "entities.walker[0].alive"),
        Value::Flag(true),
        "keep_on_stomp"
    );
    rt.step(&pads(0, 1)).unwrap();
    assert_eq!(
        get(&rt, "entities.walker[0].vx"),
        Value::Fixed(0),
        "flat: walk speed 0"
    );
    rt.step(&pads(0, 8)).unwrap();
    assert_eq!(get(&rt, "entities.walker[0].alive"), Value::Flag(true));
    rt.step(&pads(0, 1)).unwrap();
    assert_eq!(rt.frame(), flat + 10);
    assert_eq!(
        get(&rt, "entities.walker[0].alive"),
        Value::Flag(false),
        "removed after 10"
    );
}

#[test]
fn running_skids_and_jump_bands() {
    let mut rt = load();
    // Accelerating at a quarter pixel a frame to the walk speed.
    rt.step(&pads(RIGHT, 2)).unwrap();
    assert_eq!(get(&rt, "entities.hero[0].vx"), Value::Fixed(0x8000));
    rt.step(&pads(RIGHT | B, 12)).unwrap();
    assert_eq!(num(&rt, "entities.hero[0].vx"), 3);
    // Holding left skids at half a pixel a frame.
    rt.step(&pads(LEFT, 1)).unwrap();
    assert_eq!(get(&rt, "entities.hero[0].vx"), Value::Fixed(0x2_8000));
    // A jump at speed 2 or more uses the default speed; slower, the band.
    let mut fast = load();
    fast.step(&pads(RIGHT | B, 12)).unwrap();
    fast.step(&pads(RIGHT | B | A, 1)).unwrap();
    assert_eq!(get(&fast, "entities.hero[0].vy"), Value::Fixed(-0x5_8000));
    let mut slow = load();
    slow.step(&pads(RIGHT, 4)).unwrap();
    slow.step(&pads(RIGHT | A, 1)).unwrap();
    assert_eq!(get(&slow, "entities.hero[0].vy"), Value::Fixed(-0x3_8000));
}

#[test]
fn coyote_time_and_the_jump_buffer() {
    // Walk off into the hole at column 40 and press A one frame late.
    let mut rt = load_with(lifted_with(false, false));
    let until = rt
        .run_until("entities.hero[0].on_ground == 0", &pads(RIGHT | B, 1), 400)
        .unwrap();
    assert!(until.met, "the hero reaches the hole");
    assert!(
        num(&rt, "entities.hero[0].vy") >= 0,
        "walked off, didn't jump"
    );
    rt.step(&pads(RIGHT | B | A, 1)).unwrap();
    assert!(
        num(&rt, "entities.hero[0].vy") < 0,
        "a jump within the coyote frames"
    );

    // A press in the air up to buffer_frames (4) frames before the frame
    // it lands jumps on the next frame; one 5 frames before doesn't.
    let fresh = || load_with(lifted_with(false, false));
    let mut probe = fresh();
    probe.step(&pads(A, 1)).unwrap();
    let mut landing = 0;
    for _ in 0..100 {
        probe.step(&pads(0, 1)).unwrap();
        if get(&probe, "entities.hero[0].on_ground") == Value::Flag(true) {
            landing = probe.frame();
            break;
        }
    }
    assert!(landing > 10, "the jump lands");
    let jumped_after = |press: u64| {
        let mut rt = fresh();
        rt.step(&pads(A, 1)).unwrap();
        while rt.frame() < landing + 1 {
            let a = if rt.frame() + 1 == press { A } else { 0 };
            rt.step(&pads(a, 1)).unwrap();
        }
        num(&rt, "entities.hero[0].vy") < 0
    };
    assert!(jumped_after(landing - 4), "4 frames before landing");
    assert!(jumped_after(landing), "on the landing frame");
    assert!(!jumped_after(landing - 5), "5 frames before landing");
}

#[test]
fn entities_far_behind_the_view_go() {
    let mut rt = load_with(lifted_with(false, false));
    // Run right, jumping the hole, until the walker appears.
    let mut appeared = None;
    for _ in 0..600 {
        let x = num(&rt, "entities.hero[0].x");
        let jump =
            (620..640).contains(&x) && get(&rt, "entities.hero[0].on_ground") == Value::Flag(true);
        rt.step(&pads(RIGHT | B | if jump { A } else { 0 }, 1))
            .unwrap();
        if get(&rt, "entities.walker[0].waiting") == Value::Flag(false) {
            appeared = Some(num(&rt, "cameras.main.x"));
            break;
        }
    }
    let camera = appeared.unwrap_or_else(|| {
        panic!(
            "the walker appears as the view comes near; hero at {} {}, {}",
            num(&rt, "entities.hero[0].x"),
            num(&rt, "entities.hero[0].y"),
            get(&rt, "rules.game.state")
        )
    });
    // The spawn system runs before the camera moves this frame, so the
    // camera has moved on by up to the hero's speed since.
    let ahead = 768 - (camera + 256);
    assert!(
        (32 - 3..32).contains(&(ahead + 3)),
        "within spawn_ahead: {camera}"
    );
    // Put it just inside the despawn line, then one pixel past it.
    let cam = num(&rt, "cameras.main.x");
    rt.set("entities.walker[0].x", Value::Int(cam - 64 - 14))
        .unwrap();
    rt.set("entities.walker[0].vx", Value::Int(0)).unwrap();
    rt.step(&pads(0, 1)).unwrap();
    assert_eq!(get(&rt, "entities.walker[0].alive"), Value::Flag(true));
    let cam = num(&rt, "cameras.main.x");
    rt.set("entities.walker[0].x", Value::Int(cam - 64 - 18))
        .unwrap();
    rt.step(&pads(0, 1)).unwrap();
    assert_eq!(
        get(&rt, "entities.walker[0].alive"),
        Value::Flag(false),
        "gone"
    );
}

// Core 0.3.0 and platformer 0.3.0: exits, running acceleration, flying,
// hopping, paths and `become`.

/// The small model with `from` replaced by `to` (once) and `extra` added.
pub(crate) fn edited(from: &str, to: &str, extra: &str) -> Interp {
    assert!(MODEL.contains(from), "{from}");
    let mut text = format!("{}\n{extra}", MODEL.replacen(from, to, 1));
    if extra.contains("[levels.two]") {
        text = text.replace(
            "frame_rate = \"60\"\n",
            "frame_rate = \"60\"\nstart_level = \"one\"\n",
        );
    }
    let m = gp_model::Model::parse(&text).unwrap();
    // The small model has no evidence table; everything else must hold.
    let problems: Vec<_> = gp_model::validate::validate(&m)
        .into_iter()
        .filter(|p| !p.message.contains("no evidence"))
        .collect();
    assert!(problems.is_empty(), "{problems:?}");
    Interp::load(&m, &lifted()).unwrap()
}

#[test]
fn running_accelerates_by_run_acceleration() {
    let mut rt = edited(
        "acceleration = \"0.25\"\n",
        "acceleration = \"0.25\"\nrun_acceleration = \"0.75\"\n",
        "",
    );
    rt.step(&pads(RIGHT | B, 1)).unwrap();
    assert_eq!(get(&rt, "entities.hero[0].vx"), Value::Fixed(0xC000));
    rt.step(&pads(0, 8)).unwrap();
    rt.step(&pads(RIGHT, 1)).unwrap();
    assert_eq!(
        get(&rt, "entities.hero[0].vx"),
        Value::Fixed(0x4000),
        "walking"
    );
}

const EXIT: &str = r#"
[levels.two]
size = [64, 15]
tile_size = 16
grid = "area"
tileset = "blocks"
solid_property = "solid"
kill_below = 240

[[levels.two.spawn]]
entity = "hero"
x = 24
y = 192
"#;

#[test]
fn an_exit_is_taken_holding_its_action_and_keeps_counters() {
    let exit = "[[levels.one.exit]]\nto = \"two\"\nvia = \"pipe\"\ntake = { at = { x = 80, y = 0, w = 8, h = 240 }, hold = \"run\", standing = true }\nenter = { x = 300, y = 192 }\n";
    let mut rt = edited(
        "[[levels.one.spawn]]",
        &format!("{exit}\n[[levels.one.spawn]]"),
        &format!("{EXIT}\n[counters.steps]\nchange = [{{ every = 1 }}]\n"),
    );
    // Walking into the box without the action held takes nothing.
    let until = rt
        .run_until("entities.hero[0].x >= 70", &pads(RIGHT, 1), 200)
        .unwrap();
    assert!(until.met);
    rt.step(&pads(RIGHT, 2)).unwrap();
    assert_eq!(get(&rt, "level"), Value::Text("one".into()));
    let steps = num(&rt, "counters.steps");
    rt.step(&pads(RIGHT | B, 1)).unwrap();
    assert_eq!(get(&rt, "level"), Value::Text("two".into()));
    assert_eq!(num(&rt, "entities.hero[0].x"), 300);
    assert_eq!(get(&rt, "entities.hero[0].vx"), Value::Fixed(0));
    assert!(num(&rt, "counters.steps") > steps, "counters keep counting");
    // The camera is placed on the player: 300 - 112.
    assert_eq!(num(&rt, "cameras.main.x"), 188);
}

#[test]
fn fliers_hoppers_and_paths() {
    let extra = r#"
[entities.bullet]
role = "enemy"
size = [16, 16]
hitbox = { x = 0, y = 0, w = 16, h = 16 }
fly = { speed = 2, heading = "left" }

[entities.hopper]
role = "enemy"
size = [16, 16]
hitbox = { x = 0, y = 0, w = 16, h = 16 }
tile_collision = true
fall = { gravity = "0.5", max_fall = 4 }
hop = { speed = -3 }

[entities.plant]
role = "enemy"
size = [16, 16]
hitbox = { x = 0, y = 0, w = 16, h = 16 }
path = { axis = "y", distance = -8, speed = 1, pause = 3 }
"#;
    let spawns = "[[levels.one.spawn]]\nentity = \"bullet\"\nx = 200\ny = 64\n\n[[levels.one.spawn]]\nentity = \"hopper\"\nx = 120\ny = 192\n\n[[levels.one.spawn]]\nentity = \"plant\"\nx = 160\ny = 100\n\n[[levels.one.spawn]]";
    let mut rt = edited("[[levels.one.spawn]]", spawns, extra);
    rt.step(&pads(0, 1)).unwrap();
    // Flying left 2 pixels a frame, through the air with no gravity.
    let x0 = num(&rt, "entities.bullet[0].x");
    rt.step(&pads(0, 5)).unwrap();
    assert_eq!(num(&rt, "entities.bullet[0].x"), x0 - 10);
    assert_eq!(num(&rt, "entities.bullet[0].y"), 64);
    // The hopper lands and jumps again at -3.
    let mut hopped = false;
    for _ in 0..40 {
        rt.step(&pads(0, 1)).unwrap();
        if get(&rt, "entities.hopper[0].vy") == Value::Fixed(-3 << 16) {
            hopped = true;
        }
    }
    assert!(hopped, "the hopper jumps again on landing");
    // The plant goes up 8 pixels a pixel a frame, waits 3, comes back.
    let mut rt = edited("[[levels.one.spawn]]", spawns, extra);
    let mut ys = Vec::new();
    for _ in 0..24 {
        rt.step(&pads(0, 1)).unwrap();
        ys.push(num(&rt, "entities.plant[0].y"));
    }
    assert_eq!(&ys[..12], &[99, 98, 97, 96, 95, 94, 93, 92, 92, 92, 92, 93]);
    assert_eq!(*ys.iter().min().unwrap(), 92);
    // Platformer 0.4.7: with every = 2 and phase = 1 it moves on odd
    // frames only, and its wait at the top counts every frame.
    let odd = extra.replace("pause = 3 }", "pause = 3, every = 2, phase = 1 }");
    let mut rt = edited("[[levels.one.spawn]]", spawns, &odd);
    let mut ys = Vec::new();
    for _ in 0..24 {
        rt.step(&pads(0, 1)).unwrap();
        ys.push((num(&rt, "frame"), num(&rt, "entities.plant[0].y")));
    }
    let moves: Vec<i64> = ys
        .windows(2)
        .filter(|w| w[1].1 != w[0].1)
        .map(|w| w[1].0)
        .collect();
    assert!(moves.iter().all(|f| f % 2 == 1), "{ys:?}");
    // Up from 100 on frame 1, at the top on 15, 3 frames' wait, down on 19.
    assert_eq!(ys[0], (1, 99));
    assert_eq!(&moves[..9], &[3, 5, 7, 9, 11, 13, 15, 19, 21], "{ys:?}");
}

// Platformer 0.4.1: hops by a pattern and on a beat, swinging around a
// point, and leaving the view.
#[test]
fn hop_patterns_beats_oscillation_and_leaving_the_view() {
    let extra = r#"
[entities.tripper]
role = "enemy"
size = [16, 16]
hitbox = { x = 0, y = 0, w = 16, h = 16 }
tile_collision = true
fall = { gravity = "0.5", max_fall = 4 }
hop = { speed = -1, pattern = [-1, -1, -2] }

[entities.drummer]
role = "enemy"
size = [16, 16]
hitbox = { x = 0, y = 0, w = 16, h = 16 }
tile_collision = true
fall = { gravity = "0.5", max_fall = 4 }
hop = { speed = -1, every = 32 }

[entities.bobber]
role = "enemy"
size = [16, 16]
hitbox = { x = 0, y = 0, w = 16, h = 16 }
oscillate = { axis = "y", center = 16, force = "0.25", max_speed = 1, low_at = 150, low_center = -16 }

[entities.dart]
role = "enemy"
size = [8, 8]
hitbox = { x = 0, y = 0, w = 8, h = 8 }
fly = { speed = 4, heading = "left" }
leave_view = { margin = 8 }
"#;
    let spawns = "[[levels.one.spawn]]\nentity = \"tripper\"\nx = 120\ny = 192\n\n[[levels.one.spawn]]\nentity = \"drummer\"\nx = 160\ny = 192\n\n[[levels.one.spawn]]\nentity = \"bobber\"\nx = 200\ny = 64\n\n[[levels.one.spawn]]\nentity = \"dart\"\nx = 64\ny = 64\n\n[[levels.one.spawn]]";
    let mut rt = edited("[[levels.one.spawn]]", spawns, extra);
    let (mut hops, mut beats, mut ys) = (Vec::new(), Vec::new(), Vec::new());
    let mut dart_gone = None;
    for f in 0..160u64 {
        rt.step(&pads(0, 1)).unwrap();
        let vy = get(&rt, "entities.tripper[0].vy");
        if vy == Value::Fixed(-1 << 16) || vy == Value::Fixed(-2 << 16) {
            hops.push(vy);
        }
        if get(&rt, "entities.drummer[0].vy") == Value::Fixed(-1 << 16) {
            beats.push(f);
        }
        ys.push(num(&rt, "entities.bobber[0].y"));
        if dart_gone.is_none() && get(&rt, "entities.dart[0].alive") == Value::Flag(false) {
            dart_gone = Some(f);
        }
    }
    // Two small hops and a big one, in turn.
    assert_eq!(
        &hops[..4],
        &[
            Value::Fixed(-1 << 16),
            Value::Fixed(-1 << 16),
            Value::Fixed(-2 << 16),
            Value::Fixed(-1 << 16)
        ]
    );
    // Hops start only on frames whose number is a multiple of 32.
    assert!(!beats.is_empty());
    assert!(beats.iter().all(|f| (f + 1) % 32 == 0), "{beats:?}");
    // The bobber swings around 80 (placed at 64, centered 16 below): it
    // goes past the center and comes back, never more than 1 a frame.
    assert!(ys.iter().any(|&y| y > 80) && ys.iter().any(|&y| y < 80));
    assert!(ys.windows(2).all(|w| (w[1] - w[0]).abs() <= 1));
    // The dart flies left from 64 at 4 a frame and is gone once its x is
    // under 8 inside the view's left edge.
    assert!(dart_gone.is_some_and(|f| f < 20), "{dart_gone:?}");
}

// Platformer 0.4.1: a shuttle with a sway, and an orbit's pivot spawning
// its parts and turning.
#[test]
fn shuttles_sway_and_orbits_turn() {
    let extra = r#"
[entities.flier]
role = "enemy"
size = [16, 16]
hitbox = { x = 0, y = 0, w = 16, h = 16 }
shuttle = { axis = "x", step = 1, every = 2, turn_after = 3, sway = { step = 1, half = 4 } }

[entities.spark]
role = "enemy"
size = [8, 8]
hitbox = { x = 0, y = 0, w = 8, h = 8 }

[entities.pivot]
role = "enemy"
size = [8, 8]
hitbox = { x = 0, y = 0, w = 8, h = 8 }
orbit = { spin = "0.5", parts = 3, part = "spark", place = [4, 4] }
"#;
    let spawns = "[[levels.one.spawn]]\nentity = \"flier\"\nx = 100\ny = 64\n\n[[levels.one.spawn]]\nentity = \"pivot\"\nx = 150\ny = 64\n\n[[levels.one.spawn]]";
    let mut rt = edited("[[levels.one.spawn]]", spawns, extra);
    let mut xs = Vec::new();
    for _ in 0..16 {
        rt.step(&pads(0, 1)).unwrap();
        xs.push(num(&rt, "entities.flier[0].x"));
    }
    // A pixel every other frame: three moves right, then turning.
    let moves: Vec<i64> = xs
        .windows(2)
        .map(|w| w[1] - w[0])
        .filter(|d| *d != 0)
        .collect();
    assert_eq!(&moves[..6], &[1, 1, 1, -1, -1, -1], "{xs:?}");
    // The pivot moved by its place and made its three parts there.
    assert_eq!(num(&rt, "entities.pivot[0].x"), 154);
    assert_eq!(rt.query("entities.spark[*].x").unwrap().len(), 3);
}

#[test]
fn become_replaces_the_instance() {
    let mut rt = edited(
        "[[entities.walker.machine.states.walking.transition]]\non = \"enemy.stomped\"\nto = \"flat\"",
        "[[entities.walker.machine.states.walking.transition]]\non = \"enemy.stomped\"\nto = \"flat\"\ndo = [\"become:bullet\"]",
        "[entities.bullet]\nrole = \"enemy\"\nsize = [16, 16]\nhitbox = { x = 0, y = 0, w = 16, h = 16 }\nfly = { speed = 1, heading = \"right\" }\n",
    );
    let stomped = (56..100).any(|at| {
        let cond = format!("entities.walker[0].x <= {at}");
        let mut probe = rt.clone();
        assert!(probe.run_until(&cond, &pads(0, 1), 200).unwrap().met);
        for f in 0..40u64 {
            probe.step(&pads(if f < 3 { A } else { 0 }, 1)).unwrap();
            if probe
                .query("entities.bullet[0].alive")
                .is_ok_and(|e| !e.is_empty())
            {
                assert_eq!(get(&probe, "entities.walker[0].alive"), Value::Flag(false));
                assert_eq!(get(&probe, "entities.bullet[0].alive"), Value::Flag(true));
                return true;
            }
        }
        false
    });
    assert!(stomped, "some timing stomps the walker into a bullet");
    rt.step(&pads(0, 1)).unwrap();
}

/// An original made of RAM a script writes: each frame, slot `slot`'s
/// drifter (ID 9) moves 2 pixels right on even clock ticks, so its speed is
/// 1 a frame, and turns back every 16 ticks.
struct Scripted {
    ram: Vec<u8>,
    frame: u64,
    /// From this frame slot 1's state byte (0x15) has d5 set.
    defeated_from: Option<u64>,
}

const SLOT_MAPPING: &str = r#"
schema = 1
model = "tiny"
source = "tiny"
align = "polled"

[[field]]
path = "entities.hero[0].x"
ram = [0x40]

[[field]]
path = "entities.hero[0].y"
ram = [0x41]

[slots]
count = 2
active = 0x10
id = 0x12
clock = 0x09
moving = "ram[0x0020] == 0"

[[slots.level]]
level = "one"
when = "ram[0x0021] == 0"

[[slots.field]]
path = "x"
ram = [0x30, 0x32]

[[slots.field]]
path = "y"
ram = [0x34]

[[slots.kind]]
entity = "drifter"
ids = [9]
"#;

impl Scripted {
    fn byte(&self, a: usize) -> u8 {
        self.ram[a]
    }
}

impl Runtime for Scripted {
    fn capabilities(&self) -> gp_runtime::Capabilities {
        gp_runtime::Capabilities::empty()
    }
    fn step(&mut self, inputs: &[InputFrame]) -> Result<Vec<gp_runtime::Event>, String> {
        for _ in inputs {
            self.frame += 1;
            // The original pauses (no clock tick) on frames 30 to 34.
            if (30..35).contains(&self.frame) {
                continue;
            }
            self.ram[0x09] = self.ram[0x09].wrapping_add(1);
            let t = self.ram[0x09];
            // Slot 1 holds the drifter from frame 3; slot 0 something else.
            self.ram[0x10] = 1;
            self.ram[0x12] = 4;
            if self.defeated_from.is_some_and(|f| self.frame >= f) {
                self.ram[0x15] = 0x20;
            }
            if self.frame >= 3 {
                self.ram[0x11] = 1;
                self.ram[0x13] = 9;
                if self.frame > 3 && t.is_multiple_of(2) {
                    let left = (t / 16) % 2 == 1;
                    let x = self.ram[0x31] as i16 + if left { -2 } else { 2 };
                    self.ram[0x31] = x as u8;
                }
            }
        }
        Ok(Vec::new())
    }
    fn run_until(
        &mut self,
        _: &str,
        _: &[InputFrame],
        _: u32,
    ) -> Result<gp_runtime::Until, String> {
        Err("no".into())
    }
    fn frame(&self) -> u64 {
        self.frame
    }
    fn snapshot(&self) -> Vec<u8> {
        self.ram.clone()
    }
    fn restore(&mut self, s: &[u8]) -> Result<(), String> {
        self.ram = s.to_vec();
        Ok(())
    }
    fn load_state(&mut self, _: &[u8]) -> Result<Vec<gp_runtime::LoadNote>, String> {
        Err("no".into())
    }
    fn fork(&self) -> Self {
        Scripted {
            ram: self.ram.clone(),
            frame: self.frame,
            defeated_from: self.defeated_from,
        }
    }
    fn canonical_state(&self) -> Vec<u8> {
        self.ram.clone()
    }
    fn state_hash(&self) -> u64 {
        0
    }
    fn query(&self, sel: &str) -> Result<Vec<gp_runtime::Entry>, String> {
        let a = sel
            .strip_prefix("ram[0x")
            .and_then(|s| s.strip_suffix(']'))
            .and_then(|h| usize::from_str_radix(h, 16).ok())
            .ok_or(format!("{sel}?"))?;
        Ok(vec![gp_runtime::Entry {
            path: sel.into(),
            value: Value::Int(i64::from(self.byte(a))),
        }])
    }
    fn sample(
        &mut self,
        _: &[String],
        _: &[InputFrame],
        _: u32,
    ) -> Result<Vec<Vec<gp_runtime::Entry>>, String> {
        Err("no".into())
    }
    fn entities_at(&self, _: gp_runtime::Rect) -> Vec<u64> {
        Vec::new()
    }
    fn patch(&mut self, _: &str) -> gp_runtime::PatchResult {
        gp_runtime::PatchResult::NeedsReload
    }
    fn set(&mut self, _: &str, _: Value) -> Result<(), String> {
        Err("no".into())
    }
    fn explain(&self, _: &str) -> Vec<String> {
        Vec::new()
    }
}

/// The drifter's hook: a pixel a frame by its `speed`, turning every 16
/// frames of the clock, as the scripted original does on average.
struct Drift;

impl gp_sim::Ext for Drift {
    fn step(&self, cx: &mut gp_sim::ExtCx<'_>) {
        let speed = cx.param("speed");
        let left = (cx.frame / 16) % 2 == 1;
        cx.me.x = if left {
            cx.me.x.sub(speed)
        } else {
            cx.me.x.add(speed)
        };
    }
}

/// Each slot life gets one body of its entity, placed where the slot is
/// on its first frame and stepped on the original's clock ticks while it
/// moves its objects; both sides' speed and period come out alike.
#[test]
fn slot_lives_are_compared_with_one_body_each() {
    let interp = edited(
        "[cameras.main]",
        "[entities.drifter]\nrole = \"enemy\"\nsize = [16, 16]\next = \"drift\"\next_params = { speed = \"1\" }\n\n[entities.drifter.hitbox]\nx = 0\ny = 0\nw = 16\nh = 16\n\n[cameras.main]",
        "",
    );
    let mapping = crate::mapping::Mapping::parse(SLOT_MAPPING).unwrap();
    let mut ram = vec![0u8; 0x800];
    ram[0x31] = 100;
    ram[0x35] = 50;
    let mut original = Scripted {
        ram,
        frame: 0,
        defeated_from: None,
    };
    let mut exts = gp_sim::Exts::new();
    exts.insert("drift".into(), Box::new(Drift));
    let mut rows = 0;
    let reports = crate::compare::compare_slots(
        &mut original,
        &mapping,
        &pads(0, 200),
        interp.spec(),
        &exts,
        &mut |r| {
            assert_eq!(r.entity, "drifter");
            rows += 1;
        },
    )
    .unwrap();
    assert_eq!(reports.len(), 1);
    let r = &reports[0];
    assert_eq!(
        (r.lives, r.frames),
        (1, 192),
        "200 frames, less 3 before and 5 paused"
    );
    let x = &r.fields[0];
    assert_eq!(x.path, "x");
    // The model moves a pixel every tick, the original two every other:
    // within a pixel on every frame.
    assert_eq!(x.agreed, x.compared);
    let m = r.measures();
    let speed = m
        .iter()
        .find(|m| m.field == "x" && m.what == "speed")
        .unwrap();
    assert_eq!(speed.original, gp_sim::Fx::from_int(1));
    assert_eq!(speed.model, gp_sim::Fx::from_int(1));
    let period = m
        .iter()
        .find(|m| m.field == "x" && m.what == "period")
        .unwrap();
    assert_eq!(period.original, gp_sim::Fx::from_int(32));
    assert_eq!(period.off_bp(), Some(0), "{period:?}");
    assert!(rows > 0);
}

/// A start carries the original's slot objects into the model: each
/// living one of a kind the slots name becomes a body at the slot's
/// place, and the model's own entities that have appeared go.
#[test]
fn a_start_carries_the_originals_slot_objects() {
    let interp = edited(
        "[cameras.main]",
        "[entities.drifter]\nrole = \"enemy\"\nsize = [16, 16]\next = \"drift\"\next_params = { speed = \"1\" }\n\n[entities.drifter.hitbox]\nx = 0\ny = 0\nw = 16\nh = 16\n\n[cameras.main]",
        "",
    );
    let mapping = crate::mapping::Mapping::parse(SLOT_MAPPING).unwrap();
    let mut ram = vec![0u8; 0x800];
    ram[0x31] = 100;
    ram[0x35] = 50;
    ram[0x40] = 20;
    ram[0x41] = 30;
    let mut original = Scripted {
        ram,
        frame: 0,
        defeated_from: None,
    };
    original.step(&pads(0, 3)).unwrap();
    let spec = interp.spec().clone();
    let mut base = gp_sim::World::power_on(&spec);
    let enemy = |w: &gp_sim::World| {
        w.bodies
            .iter()
            .filter(|b| {
                b.alive && !b.waiting && spec.kinds[b.kind].role != gp_sim::spec::Role::Player
            })
            .map(|b| (spec.kinds[b.kind].name.clone(), b.x.floor(), b.y.floor()))
            .collect::<Vec<_>>()
    };
    // The model's own enemies, all appeared.
    for b in &mut base.bodies {
        b.waiting = false;
    }
    assert!(
        enemy(&base).iter().any(|e| e.0 != "drifter"),
        "{:?}",
        enemy(&base)
    );
    let t = crate::translate::from_original(&original, &spec, &base, &mapping).unwrap();
    // Slot 0 holds an ID the slots don't name; slot 1 the drifter.
    assert_eq!(enemy(&t.world), [("drifter".to_owned(), 100, 50)]);
    let note = t
        .notes
        .iter()
        .find(|n| n.path == "entities.drifter[*]")
        .unwrap();
    assert_eq!(note.how, gp_runtime::LoadHow::Approximated);
    assert!(
        note.why
            .starts_with("1 from the original's slots, at x 100"),
        "{}",
        note.why
    );
    // A resync keeps the model's own.
    let kept = crate::translate::keeping_entities(&original, &spec, &base, &mapping).unwrap();
    assert_eq!(enemy(&kept.world), enemy(&base));
}

/// A kind with `holds_slot`: a start carries the object, and a run's slot
/// objects leave it out. The numbers are made up.
#[test]
fn an_object_that_holds_its_slot_is_carried_and_not_measured() {
    let text = SLOT_MAPPING.replace(
        "moving = \"ram[0x0020] == 0\"\n",
        "moving = \"ram[0x0020] == 0\"\nstate = 0x14\nends = 0x20\n",
    ) + "\n[[slots.kind]]\nentity = \"stump\"\nids = [9]\nends = 0\nholds_slot = true\n";
    let e =
        crate::mapping::Mapping::parse(&text.replace("schema = 1\n", "schema = 18\n")).unwrap_err();
    assert!(e.contains("holds_slot needs schema 19"), "{e}");
    let text = text.replace("schema = 1\n", "schema = 19\n");
    let mapping = crate::mapping::Mapping::parse(&text).unwrap();
    let mut ram = vec![0u8; 0x800];
    ram[0x11] = 1;
    ram[0x13] = 9;
    ram[0x15] = 0x20;
    let original = Scripted {
        ram,
        frame: 0,
        defeated_from: None,
    };
    let slots = mapping.slots.as_ref().unwrap();
    assert!(
        crate::bodies::slot_objects(&original, slots)
            .unwrap()
            .is_empty()
    );
    let carried = crate::bodies::carried_slot_objects(&original, slots).unwrap();
    assert_eq!(carried.len(), 1);
    assert_eq!((carried[0].slot, carried[0].kind), (1, 1));
}

/// Mapping schema 5: a carried body takes every slot field the mapping
/// names, its facing, its x fraction and a path's spawn end, way and wait,
/// and the model's frame count takes the slots' clock as its low byte, so a
/// path that moves on odd frames moves on the original's. The numbers are
/// made up.
#[test]
fn a_carried_body_takes_its_facing_fraction_and_path_state() {
    let interp = edited(
        "[cameras.main]",
        "[entities.plant]\nrole = \"enemy\"\nsize = [16, 16]\nhitbox = { x = 0, y = 0, w = 16, h = 16 }\npath = { axis = \"y\", distance = -8, speed = 1, pause = 5, every = 2, phase = 1 }\n\n[cameras.main]",
        "",
    );
    let kinds = r#"[[slots.kind]]
entity = "walker"
ids = [7]

[[slots.kind.field]]
path = "x"
ram = [0x30, 0x32]
frac = 0x50

[[slots.kind.field]]
path = "facing"
ram = [0x52]
values = ["right", "right", "left"]

[[slots.kind]]
entity = "plant"
ids = [8]

[[slots.kind.field]]
path = "home_y"
ram = [0x54]

[[slots.kind.field]]
path = "path_back"
ram = [0x56]
bits = [7, 8]
flag = true
invert = true

[[slots.kind.field]]
path = "wait"
ram = [0x58]
offset = -1
"#;
    let text = SLOT_MAPPING
        .replace("schema = 1", "schema = 5")
        .replace("[[slots.kind]]\nentity = \"drifter\"\nids = [9]\n", kinds);
    let mapping = crate::mapping::Mapping::parse(&text).unwrap();
    // A schema 4 mapping can't name them.
    let old = crate::mapping::Mapping::parse(&text.replace("schema = 5", "schema = 4"));
    assert!(old.unwrap_err().contains("needs schema 5"));
    let mut ram = vec![0u8; 0x800];
    ram[0x09] = 0x41;
    (ram[0x10], ram[0x11], ram[0x12], ram[0x13]) = (1, 1, 7, 8);
    // Slot 0: a walker at x 100.5, facing left.
    (ram[0x30], ram[0x50], ram[0x34], ram[0x52]) = (100, 0x80, 50, 2);
    // Slot 1: a plant at the top of its path (home 100, out 8), its last
    // move up, 4 frames on its timer.
    (ram[0x31], ram[0x35], ram[0x55], ram[0x57], ram[0x59]) = (140, 92, 100, 0xff, 4);
    (ram[0x40], ram[0x41]) = (20, 30);
    let original = Scripted {
        ram,
        frame: 0,
        defeated_from: None,
    };
    let spec = interp.spec().clone();
    let base = gp_sim::World::power_on(&spec);
    let t = crate::translate::from_original(&original, &spec, &base, &mapping).unwrap();
    let of = |name: &str| {
        let k = spec.kinds.index_of(name).unwrap();
        *t.world
            .bodies
            .iter()
            .rfind(|b| b.kind == k && b.alive)
            .unwrap()
    };
    let walker = of("walker");
    assert_eq!(walker.x, gp_sim::Fx::from_raw(100 << 16 | 0x8000));
    assert!(walker.facing_left);
    let plant = of("plant");
    assert_eq!(plant.home[1], gp_sim::Fx::from_int(100));
    assert!(plant.path_back, "at the far end it heads back");
    assert_eq!(plant.wait, 3);
    assert_eq!(t.world.frame as u8, 0x41);
    assert!(t.world.frame >= base.frame);
    // The state names them.
    let mut rt = interp.clone();
    rt.load_state(&t.state()).unwrap();
    assert_eq!(num(&rt, "entities.plant[0].home_y"), 100);
    assert_eq!(get(&rt, "entities.plant[0].path_back"), Value::Flag(true));
    assert_eq!(num(&rt, "entities.plant[0].wait"), 3);
    // It waits 3 frames, then moves down on the next odd one, 0x45.
    let mut ys = Vec::new();
    for _ in 0..6 {
        rt.step(&pads(0, 1)).unwrap();
        ys.push((num(&rt, "frame") as u8, num(&rt, "entities.plant[0].y")));
    }
    assert_eq!(
        ys,
        [
            (0x42, 92),
            (0x43, 92),
            (0x44, 92),
            (0x45, 93),
            (0x46, 93),
            (0x47, 94)
        ]
    );
}

/// A life ends on a state bit `ends` names, or a flag the kind doesn't
/// list, and the body steps through its components too.
#[test]
fn slot_lives_end_on_a_state_bit_and_take_only_their_flags() {
    let interp = edited(
        "[cameras.main]",
        "[entities.drifter]\nrole = \"enemy\"\nsize = [16, 16]\next = \"drift\"\next_params = { speed = \"1\" }\n\n[entities.drifter.hitbox]\nx = 0\ny = 0\nw = 16\nh = 16\n\n[cameras.main]",
        "",
    );
    let mut exts = gp_sim::Exts::new();
    exts.insert("drift".into(), Box::new(Drift));
    let run = |mapping: &str, defeated_from| {
        let mapping = crate::mapping::Mapping::parse(mapping).unwrap();
        let mut ram = vec![0u8; 0x800];
        ram[0x31] = 100;
        let mut original = Scripted {
            ram,
            frame: 0,
            defeated_from,
        };
        crate::compare::compare_slots(
            &mut original,
            &mapping,
            &pads(0, 200),
            interp.spec(),
            &exts,
            &mut |_| {},
        )
        .unwrap()
        .remove(0)
    };
    let ends = SLOT_MAPPING.replace("count = 2", "count = 2\nstate = 0x14\nends = 0x20");
    let r = run(&ends, Some(100));
    assert_eq!(r.lives, 1);
    assert!(r.frames < 100, "{}", r.frames);
    // A kind's own `ends` replaces the slots'.
    let none = ends.replace("ids = [9]", "ids = [9]\nends = 0x01");
    assert_eq!(run(&none, Some(100)).frames, 192);
    // Slot 1's flag is 1: a kind that takes only flag 2 has no lives.
    let flags = SLOT_MAPPING.replace("ids = [9]", "ids = [9]\nflags = [2]");
    assert_eq!(run(&flags, None).lives, 0);
}

#[test]
fn slots_are_checked_when_parsed() {
    let bad = |from: &str, to: &str, why: &str| {
        let text = SLOT_MAPPING.replace(from, to);
        let e = crate::mapping::Mapping::parse(&text).unwrap_err();
        assert!(e.contains(why), "{e}");
    };
    bad("path = \"y\"", "path = \"speed\"", "isn't one of");
    bad("path = \"y\"", "path = \"facing\"", "needs schema 5");
    bad(
        "ids = [9]",
        "ids = [9]\n\n[[slots.kind]]\nentity = \"walker\"\nids = [9]",
        "names two entities",
    );
    bad("count = 2", "count = 0", "at least 1");
    bad("count = 2", "count = 2\nends = 0x20", "ends needs state");
    assert!(
        crate::mapping::Mapping::parse(
            &SLOT_MAPPING.replace("count = 2", "count = 2\nstate = 0x1e\nends = 0x20")
        )
        .is_ok()
    );
    bad(
        "when = \"ram[0x0021] == 0\"",
        "when = \"ram[\"",
        "slots.level one",
    );
    // A slot's rest (schema 7): what an uncarried slot holds.
    let rest = "\n[[slots.rest]]\npath = \"vy\"\nram = [0x36]\nsigned = true\n\n[[slots.level]]";
    let with_rest = SLOT_MAPPING.replacen("\n[[slots.level]]", rest, 1);
    let e =
        crate::mapping::Mapping::parse(&with_rest.replace("schema = 1", "schema = 6")).unwrap_err();
    assert!(e.contains("rest needs schema 7"), "{e}");
    let seven = with_rest.replace("schema = 1", "schema = 7");
    let m = crate::mapping::Mapping::parse(&seven).unwrap();
    assert_eq!(m.slots.unwrap().rest[0].path, "vy");
    let e = crate::mapping::Mapping::parse(&seven.replace("path = \"vy\"", "path = \"x\""))
        .unwrap_err();
    assert!(e.contains("isn't one of"), "{e}");
    // The height's fraction (schema 15).
    let y_frac = seven.replace("path = \"vy\"", "path = \"y_frac\"");
    let e = crate::mapping::Mapping::parse(&y_frac).unwrap_err();
    assert!(e.contains("rest y_frac needs schema 15"), "{e}");
    let fifteen = y_frac.replace("schema = 7", "schema = 15");
    let m = crate::mapping::Mapping::parse(&fifteen).unwrap();
    assert_eq!(m.slots.unwrap().rest[0].path, "y_frac");
    // An object that keeps its slot (schema 15).
    let held = "\n[slots.held]\nentity = \"walker\"\nids = [9]\nstate = 0x20\n\n[[slots.level]]";
    let with_held = SLOT_MAPPING.replacen("\n[[slots.level]]", held, 1);
    let e = crate::mapping::Mapping::parse(&with_held.replace("schema = 1", "schema = 14"))
        .unwrap_err();
    assert!(e.contains("held needs schema 15"), "{e}");
    let fifteen = with_held.replace("schema = 1", "schema = 15");
    let e = crate::mapping::Mapping::parse(&fifteen).unwrap_err();
    assert!(e.contains("held needs state"), "{e}");
    let m =
        crate::mapping::Mapping::parse(&fifteen.replace("count = 2", "count = 2\nstate = 0x1e"))
            .unwrap();
    assert_eq!(m.slots.unwrap().held.unwrap().entity, "walker");
}

/// A camera's `move_x` (schema 8, core 0.4.12's `keep_move`) is refused
/// before schema 8 and read from it.
#[test]
fn a_cameras_move_needs_schema_8() {
    let with = |schema: u32| {
        SLOT_MAPPING
            .replace("schema = 1", &format!("schema = {schema}"))
            .replacen(
                "[[field]]\npath = \"entities.hero[0].y\"",
                "[[field]]\npath = \"cameras.main.move_x\"\nram = [0x42]\nsigned = true\n\n[[field]]\npath = \"entities.hero[0].y\"",
                1,
            )
    };
    let e = crate::mapping::Mapping::parse(&with(7)).unwrap_err();
    assert!(e.contains("`move_x` needs schema 8"), "{e}");
    let m = crate::mapping::Mapping::parse(&with(8)).unwrap();
    assert!(m.fields.iter().any(|f| f.path == "cameras.main.move_x"));
}

/// A slot group, a kind's `up_down` and an input path (schema 9) are
/// refused before schema 9 and read from it; a group with levels of its
/// own, or a kind in two groups, is refused.
#[test]
fn slot_groups_and_input_need_schema_9() {
    let group = "\n[[slots.group]]\ncount = 2\nactive = 0x50\nid = 0x50\nclock = 0x09\n\n[[slots.group.field]]\npath = \"x\"\nram = [0x52]\n\n[[slots.group.field]]\npath = \"y\"\nram = [0x54]\n\n[[slots.group.kind]]\nentity = \"shot\"\nids = [1]\nup_down = true\n";
    let with = |schema: u32, extra: &str| {
        SLOT_MAPPING
            .replace("schema = 1", &format!("schema = {schema}"))
            .replacen(
                "[[field]]\npath = \"entities.hero[0].y\"",
                "[[field]]\npath = \"input.jump\"\nram = [0x0d]\nbits = [7, 8]\nflag = true\npresentation = true\n\n[[field]]\npath = \"entities.hero[0].y\"",
                1,
            )
            + extra
    };
    let e = crate::mapping::Mapping::parse(&with(8, "")).unwrap_err();
    assert!(e.contains("needs schema 9"), "{e}");
    let m = crate::mapping::Mapping::parse(&with(9, group)).unwrap();
    let groups = m.slot_groups();
    assert_eq!(groups.len(), 2);
    assert_eq!(groups[1].kinds[0].entity, "shot");
    assert!(groups[1].kinds[0].up_down);
    let e = crate::mapping::Mapping::parse(&with(8, &group.replace("[[field]]", ""))).unwrap_err();
    assert!(e.contains("needs schema 9"), "{e}");
    let twice = group.replace("entity = \"shot\"", "entity = \"drifter\"");
    let e = crate::mapping::Mapping::parse(&with(9, &twice)).unwrap_err();
    assert!(e.contains("in two groups"), "{e}");
    let levels =
        format!("{group}\n[[slots.group.level]]\nlevel = \"one\"\nwhen = \"ram[0x0021] == 0\"\n");
    let e = crate::mapping::Mapping::parse(&with(9, &levels)).unwrap_err();
    assert!(e.contains("no groups, levels or held"), "{e}");
}

/// An entity's `seen_speed` (schema 10, platformer 0.4.11's
/// `run.speed_before_walls`) is refused before schema 10 and read from it.
#[test]
fn an_entitys_seen_speed_needs_schema_10() {
    let with = |schema: u32| {
        SLOT_MAPPING
            .replace("schema = 1", &format!("schema = {schema}"))
            .replacen(
                "[[field]]\npath = \"entities.hero[0].y\"",
                "[[field]]\npath = \"entities.hero[0].seen_speed\"\nram = [0x42]\n\n[[field]]\npath = \"entities.hero[0].y\"",
                1,
            )
    };
    let e = crate::mapping::Mapping::parse(&with(9)).unwrap_err();
    assert!(e.contains("`seen_speed` needs schema 10"), "{e}");
    let m = crate::mapping::Mapping::parse(&with(10)).unwrap();
    assert!(
        m.fields
            .iter()
            .any(|f| f.path == "entities.hero[0].seen_speed")
    );
}

/// The value `both` of an entity's `facing` (schema 12, platformer 0.4.15's
/// `run.face_both`) is refused before schema 12 and read from it.
#[test]
fn facing_both_needs_schema_12() {
    let with = |schema: u32| {
        SLOT_MAPPING
            .replace("schema = 1", &format!("schema = {schema}"))
            .replacen(
                "[[field]]\npath = \"entities.hero[0].y\"",
                "[[field]]\npath = \"entities.hero[0].facing\"\nram = [0x42]\nvalues = [\"right\", \"right\", \"left\", \"both\"]\n\n[[field]]\npath = \"entities.hero[0].y\"",
                1,
            )
    };
    let e = crate::mapping::Mapping::parse(&with(11)).unwrap_err();
    assert!(e.contains("the value `both` needs schema 12"), "{e}");
    assert!(crate::mapping::Mapping::parse(&with(12)).is_ok());
}

/// A slot field's `shared` (schema 13) reads one address for every slot,
/// and is refused before schema 13 and on a field outside the slots.
#[test]
fn a_shared_slot_field_reads_one_address() {
    let base = "ram = [0x70]\nnote = \"a timer\"";
    let field = format!("[[slots.field]]\npath = \"ext[2]\"\n{base}\nshared = true\n");
    let with = |schema: u32| {
        format!(
            "{}\n{field}",
            SLOT_MAPPING.replace("schema = 1", &format!("schema = {schema}"))
        )
    };
    let e = crate::mapping::Mapping::parse(&with(12)).unwrap_err();
    assert!(e.contains("`shared` needs schema 13"), "{e}");
    let m = crate::mapping::Mapping::parse(&with(13)).unwrap();
    let slots = m.slots.as_ref().unwrap();
    let f = slots.fields.iter().find(|f| f.shared).unwrap();
    assert_eq!(f.slot_selectors(3), ["ram[0x0070]"]);
    let e = crate::mapping::Mapping::parse(&with(13).replacen(
        "[[field]]\n",
        "[[field]]\nshared = true\n",
        1,
    ))
    .unwrap_err();
    assert!(e.contains("is for a slot field"), "{e}");
}

/// A mapping's `[cells]` (schema 14), the original's level grid, is refused
/// before schema 14 and without a page. The grid is made up.
#[test]
fn cells_need_schema_14_and_a_page() {
    let cells = "[cells]\npages = [0x300]\npage_columns = 8\nrows = 4\ntop = 1\nview = 9\nkinds = [\"coin\"]\n";
    let with = |schema: u32, cells: &str| {
        SLOT_MAPPING
            .replace("schema = 1", &format!("schema = {schema}"))
            .replacen("[[field]]\n", &format!("{cells}\n[[field]]\n"), 1)
    };
    let e = crate::mapping::Mapping::parse(&with(13, cells)).unwrap_err();
    assert!(e.contains("`cells` needs schema 14"), "{e}");
    let m = crate::mapping::Mapping::parse(&with(14, cells)).unwrap();
    let c = m.cells.unwrap();
    assert_eq!((c.pages, c.page_columns, c.top), (vec![0x300], 8, 1));
    let e = crate::mapping::Mapping::parse(&with(14, &cells.replace("[0x300]", "[]"))).unwrap_err();
    assert!(e.contains("needs a page"), "{e}");
}

/// An entity's `ext_tables` are read from a lifted table's column, and
/// carried to its hook in the compiled kind. The table is made up.
#[test]
fn ext_tables_come_from_a_lifted_table() {
    let text = format!(
        "{}\n{}",
        MODEL.replace(
            "[cameras.main]",
            "[entities.spinner]\nrole = \"enemy\"\nsize = [8, 8]\next = \"spin\"\next_tables = { steps = \"spins.step\" }\n\n[entities.spinner.hitbox]\nx = 0\ny = 0\nw = 8\nh = 8\n\n[cameras.main]",
        ),
        "[assets.spins]\nkind = \"table\"\nrole = \"behavior\"\n\n[assets.spins.from]\nsource = \"tiny\"\nasset = \"spins\"\n"
    );
    let m = gp_model::Model::parse(&text).unwrap();
    let problems: Vec<_> = gp_model::validate::validate(&m)
        .into_iter()
        .filter(|p| !p.message.contains("no evidence"))
        .collect();
    assert!(problems.is_empty(), "{problems:?}");
    let mut l = lifted();
    let map = l.files["assets.toml"].clone()
        + "\n[[asset]]\nname = \"spins\"\nformat = \"table\"\ncount = 3\ncolumn = [{ name = \"step\", region = \"prg\", offset = 0x600 }]\n";
    l.files.insert("assets.toml".into(), map);
    l.files.insert(
        "spins.toml".into(),
        "schema = 1\nasset = \"spins\"\n\n[[row]]\nstep = 3\n\n[[row]]\nstep = 1\n\n[[row]]\nstep = 4\n".into(),
    );
    let rt = Interp::load(&m, &l).unwrap();
    let k = rt
        .spec()
        .kinds
        .iter()
        .find(|k| k.name == "spinner")
        .unwrap();
    assert_eq!(k.ext_tables["steps"], vec![3, 1, 4]);
    // A slot that isn't a table, or a name without a column, is refused.
    for (bad, why) in [
        ("\"area.step\"", "not table"),
        ("\"spins\"", "isn't `<slot>.<column>`"),
    ] {
        let m = gp_model::Model::parse(&text.replace("\"spins.step\"", bad)).unwrap();
        let p = gp_model::validate::validate(&m);
        assert!(p.iter().any(|p| p.message.contains(why)), "{bad}: {p:?}");
    }
}

/// Core `entity.ext_rows`: a `rows` slot is a table of its rows' numbers in
/// order, for a hook that reads a span by address. The rows are made up.
#[test]
fn ext_rows_are_a_rows_slot_as_one_table() {
    let text = format!(
        "{}\n{}",
        MODEL.replace(
            "[cameras.main]",
            "[entities.spinner]\nrole = \"enemy\"\nsize = [8, 8]\next = \"spin\"\next_rows = { runs = \"pieces\" }\n\n[entities.spinner.hitbox]\nx = 0\ny = 0\nw = 8\nh = 8\n\n[cameras.main]",
        ),
        "[assets.pieces]\nkind = \"rows\"\nrole = \"layout\"\n\n[assets.pieces.from]\nsource = \"tiny\"\nasset = \"pieces\"\n"
    );
    let m = gp_model::Model::parse(&text).unwrap();
    let problems: Vec<_> = gp_model::validate::validate(&m)
        .into_iter()
        .filter(|p| !p.message.contains("no evidence"))
        .collect();
    assert!(problems.is_empty(), "{problems:?}");
    let mut l = lifted();
    let map = l.files["assets.toml"].clone()
        + "\n[[asset]]\nname = \"pieces\"\nformat = \"byte-rows\"\nregion = \"prg\"\noffset = 0x600\ncount = 2\nwidth = 3\n";
    l.files.insert("assets.toml".into(), map);
    l.files.insert(
        "pieces.toml".into(),
        "schema = 1\nasset = \"pieces\"\nrows = [\n  [1, 2, 3],\n  [4, 5, 6],\n]\n".into(),
    );
    let rt = Interp::load(&m, &l).unwrap();
    let k = rt
        .spec()
        .kinds
        .iter()
        .find(|k| k.name == "spinner")
        .unwrap();
    assert_eq!(k.ext_tables["runs"], vec![1, 2, 3, 4, 5, 6]);
    // A slot that isn't rows is refused.
    let m = gp_model::Model::parse(&text.replace(
        "[assets.pieces]\nkind = \"rows\"",
        "[assets.pieces]\nkind = \"table\"",
    ))
    .unwrap();
    let p = gp_model::validate::validate(&m);
    assert!(p.iter().any(|p| p.message.contains("not rows")), "{p:?}");
}

#[test]
fn change_actions_exits_that_start_and_restart_after_a_room() {
    // A counter the rules change; the hero dies on a walker's touch and
    // the level restarts after 3 frames, taking a life.
    let rules = MODEL.split("[rules.game]").next().unwrap().to_owned();
    let tail = r#"
[counters.lives]
start = 3

[rules.game]
initial = "playing"

[[rules.game.states.playing.transition]]
on = "player.died"
to = "dead"

[rules.game.states.dead]
freeze = true

[[rules.game.states.dead.transition]]
after = 3
to = "playing"
do = ["change:lives:-1", "restart-level"]
"#;
    let room = "[[levels.one.exit]]\nto = \"two\"\nvia = \"pipe\"\ntake = { at = { x = 80, y = 0, w = 8, h = 240 }, hold = \"run\" }\nenter = { x = 300, y = 192 }\n";
    let text = format!(
        "{}\n{tail}\n{EXIT}",
        rules
            .replace(
                "frame_rate = \"60\"\n",
                "frame_rate = \"60\"\nstart_level = \"one\"\n"
            )
            .replace(
                "[[levels.one.spawn]]",
                &format!("{room}\n[[levels.one.spawn]]")
            )
    );
    let m = gp_model::Model::parse(&text).unwrap();
    let mut rt = Interp::load(&m, &lifted_with(false, false)).unwrap();
    rt.run_until("entities.hero[0].x >= 70", &pads(RIGHT, 1), 200)
        .unwrap();
    rt.step(&pads(RIGHT | B, 1)).unwrap();
    assert_eq!(get(&rt, "level"), Value::Text("two".into()));
    // Die in the room: the level last started, one, starts again, a life
    // gone by `change:lives:-1`.
    let lives = num(&rt, "counters.lives");
    let mut s = rt.world().clone();
    let p = s
        .bodies
        .iter()
        .position(|b| rt.spec().kinds[b.kind].name == "hero")
        .unwrap();
    s.bodies[p].y = gp_sim::Fx::from_int(400);
    rt.restore(&s.canonical()).unwrap();
    rt.step(&pads(0, 8)).unwrap();
    assert_eq!(get(&rt, "level"), Value::Text("one".into()));
    assert_eq!(num(&rt, "counters.lives"), lives - 1);

    // The same exit with `start` starts two as a level: its spawn, and
    // restarting starts two.
    let started = text.replace(
        "hold = \"run\" }\nenter = { x = 300, y = 192 }",
        "hold = \"run\" }\nstart = true",
    );
    let m = gp_model::Model::parse(&started).unwrap();
    let mut rt = Interp::load(&m, &lifted_with(false, false)).unwrap();
    rt.run_until("entities.hero[0].x >= 70", &pads(RIGHT, 1), 200)
        .unwrap();
    rt.step(&pads(RIGHT | B, 1)).unwrap();
    assert_eq!(get(&rt, "level"), Value::Text("two".into()));
    assert_eq!(num(&rt, "entities.hero[0].x"), 24, "two's spawn");
    assert_eq!(rt.world().home_level, rt.world().level);
}

#[test]
fn a_flier_heads_for_the_player_and_a_path_waits_while_it_is_near() {
    let extra = r#"
[entities.bullet]
role = "enemy"
size = [16, 16]
hitbox = { x = 0, y = 0, w = 16, h = 16 }
fly = { speed = 1 }

[entities.plant]
role = "enemy"
size = [16, 16]
hitbox = { x = 0, y = 0, w = 16, h = 16 }
path = { axis = "y", distance = -8, speed = 1, wait_near = 33 }
"#;
    // The hero is at x 40: a bullet at 10 goes right, one at 200 left; a
    // plant 32 pixels away waits, one 33 away rises.
    let spawns = "[[levels.one.spawn]]\nentity = \"bullet\"\nx = 10\ny = 64\n\n[[levels.one.spawn]]\nentity = \"bullet\"\nx = 200\ny = 64\n\n[[levels.one.spawn]]\nentity = \"plant\"\nx = 72\ny = 100\n\n[[levels.one.spawn]]\nentity = \"plant\"\nx = 73\ny = 100\n\n[[levels.one.spawn]]";
    let mut rt = edited("[[levels.one.spawn]]", spawns, extra);
    rt.step(&pads(0, 2)).unwrap();
    assert_eq!(
        get(&rt, "entities.bullet[0].facing"),
        Value::Text("right".into())
    );
    assert_eq!(
        get(&rt, "entities.bullet[1].facing"),
        Value::Text("left".into())
    );
    assert_eq!(num(&rt, "entities.plant[0].y"), 100, "32 away: waits");
    assert!(num(&rt, "entities.plant[1].y") < 100, "33 away: rises");
    // The canonical state keeps a path's place and wait.
    let snap = rt.world().canonical();
    let back = gp_sim::World::from_canonical(rt.spec(), &snap).unwrap();
    assert_eq!(back, *rt.world());
    assert!(
        back.bodies
            .iter()
            .any(|b| b.home[1] == gp_sim::Fx::from_int(100))
    );
}

#[test]
fn move_first_moves_by_the_old_speed_then_adds_gravity() {
    let mut late = load();
    late.step(&pads(A, 1)).unwrap();
    // The band at rest: speed -4, gravity 0.5.
    assert_eq!(
        get(&late, "entities.hero[0].y"),
        Value::Fixed((192 << 16) - 0x3_8000)
    );
    let mut first = edited(
        "coyote_frames = 3\n",
        "coyote_frames = 3\nmove_first = true\n",
        "",
    );
    first.step(&pads(A, 1)).unwrap();
    assert_eq!(get(&first, "entities.hero[0].y"), Value::Fixed(188 << 16));
    assert_eq!(get(&first, "entities.hero[0].vy"), Value::Fixed(-0x3_8000));
}

/// A recording keeps a trace by model path and an event stream; a replay
/// regenerates both and finds them equal, and a read through the trace's
/// own field table gives the interpreter's values.
#[test]
fn recordings_keep_a_trace_by_model_path_and_events() {
    use crate::recording::{RecordRequest, check, record};
    use gp_evidence::manifest::Provenance;
    use gp_evidence::model_events::decode_chunk_covering;
    use gp_evidence::trace::FieldTable;

    let m = gp_model::Model::parse(MODEL).unwrap();
    let bundle = crate::Bundle::new(&m, lifted());
    let dir = tempfile::tempdir().unwrap();
    let store = gp_evidence::Store::open(dir.path()).unwrap();
    let input = gp_evidence::input_log::generate(150, &["right@0-100", "a+right@20-40"]).unwrap();
    let done = record(
        &store,
        RecordRequest {
            bundle: &bundle,
            input: &input,
            checkpoint_interval: 60,
            provenance: Provenance::Scripted,
            name: "trace".into(),
            created_at: "2026-10-28T00:00:00Z".into(),
            notes: None,
        },
    )
    .unwrap();
    let t = done.manifest.trace.as_ref().expect("a trace");
    let e = done.manifest.events.as_ref().expect("events");
    assert_eq!(t.chunks.len(), 3);
    assert_eq!(e.schema, 2);
    let table = FieldTable::decode(&store.get(&t.fields).unwrap()).unwrap();
    assert_eq!(&table.state_magic, b"GPSP");
    assert!(table.fields.iter().any(|f| f.name == "entities.hero[0].x"));
    assert!(
        table
            .fields
            .iter()
            .any(|f| f.name == "entities.hero[0].present")
    );

    // The trace's state at frame 90 reads as the interpreter's at 90.
    let state = gp_evidence::trace::decode_state(
        &store.get(&t.chunks[1]).unwrap(),
        gp_evidence::chunk::chunk_range(1, t.chunk_frames, done.manifest.frames),
        &table,
        90,
    )
    .unwrap();
    let entries = crate::modeltrace::entries(&table, &state).unwrap();
    let mut rt = load();
    for f in input.frames.iter().take(90) {
        rt.advance(&crate::recording::input_frame(f)).unwrap();
    }
    let x = entries
        .iter()
        .find(|e| e.path == "entities.hero[0].x")
        .unwrap();
    assert_eq!(x.value, get(&rt, "entities.hero[0].x"));
    let level = entries.iter().find(|e| e.path == "level").unwrap();
    assert_eq!(level.value, get(&rt, "level"));

    // The events decode, and the replay finds the whole recording equal.
    let chunk = decode_chunk_covering(
        &store.get(&e.chunks[0]).unwrap(),
        gp_evidence::chunk::chunk_range(0, e.chunk_frames, done.manifest.frames),
    )
    .unwrap();
    assert_eq!(chunk.first_frame, 0);
    let checked = check(&store, &done.manifest).unwrap();
    assert_eq!(checked.result(), "ok", "{:?}", checked.divergence);
}

/// An original made from a model: another interpreter whose hero's state
/// it shows as RAM, the way a game keeps it, for the lockstep's tests.
/// `x` at 0x40-0x41 with its fraction at 0x42, `y` at 0x43-0x44 and 0x45,
/// `vx` and `vy` signed at 0x46 and 0x48 with fractions at 0x47 and 0x49,
/// standing at 0x4A.
#[derive(Clone)]
struct AsRam(Interp);

const RAM_MAPPING: &str = r#"
schema = 1
model = "tiny"
source = "tiny"
align = "polled"

[[field]]
path = "entities.hero[0].x"
ram = [0x40, 0x41]
frac = 0x42

[[field]]
path = "entities.hero[0].y"
ram = [0x43, 0x44]
frac = 0x45

[[field]]
path = "entities.hero[0].vx"
ram = [0x46]
frac = 0x47
signed = true

[[field]]
path = "entities.hero[0].vy"
ram = [0x48]
frac = 0x49
signed = true

[[field]]
path = "entities.hero[0].on_ground"
ram = [0x4A]
flag = true

[[field]]
path = "cameras.main.x"
ram = [0x4B, 0x4C]

[[field]]
path = "rules.game.state"
ram = [0x4D]
values = ["playing", "dead"]
"#;

impl AsRam {
    fn byte(&self, a: usize) -> Result<u8, String> {
        let raw = |p: &str| match get(&self.0, p) {
            Value::Fixed(r) => r,
            v => panic!("{p} is {v}"),
        };
        let fixed = |p: &str, i: usize| -> u8 {
            let r = raw(p);
            match i {
                0 => (r >> 16) as u8,
                1 => (r >> 24) as u8,
                _ => (r >> 8) as u8,
            }
        };
        Ok(match a {
            0x40..=0x42 => fixed("entities.hero[0].x", a - 0x40),
            0x43..=0x45 => fixed("entities.hero[0].y", a - 0x43),
            0x46 => fixed("entities.hero[0].vx", 0),
            0x47 => fixed("entities.hero[0].vx", 2),
            0x48 => fixed("entities.hero[0].vy", 0),
            0x49 => fixed("entities.hero[0].vy", 2),
            0x4A => u8::from(get(&self.0, "entities.hero[0].on_ground") == Value::Flag(true)),
            0x4B | 0x4C => fixed("cameras.main.x", a - 0x4B),
            0x4D => u8::from(get(&self.0, "rules.game.state") == Value::Text("dead".into())),
            _ => return Err(format!("no RAM at {a:#x}")),
        })
    }
}

impl Runtime for AsRam {
    fn capabilities(&self) -> gp_runtime::Capabilities {
        gp_runtime::Capabilities::FORK
    }
    fn step(&mut self, inputs: &[InputFrame]) -> Result<Vec<gp_runtime::Event>, String> {
        self.0.step(inputs)
    }
    fn run_until(
        &mut self,
        _: &str,
        _: &[InputFrame],
        _: u32,
    ) -> Result<gp_runtime::Until, String> {
        Err("no".into())
    }
    fn frame(&self) -> u64 {
        self.0.frame()
    }
    fn snapshot(&self) -> Vec<u8> {
        self.0.snapshot()
    }
    fn restore(&mut self, s: &[u8]) -> Result<(), String> {
        self.0.restore(s)
    }
    fn load_state(&mut self, _: &[u8]) -> Result<Vec<gp_runtime::LoadNote>, String> {
        Err("an original has no model state".into())
    }
    fn fork(&self) -> Self {
        self.clone()
    }
    fn canonical_state(&self) -> Vec<u8> {
        self.0.canonical_state()
    }
    fn state_hash(&self) -> u64 {
        self.0.state_hash()
    }
    fn query(&self, sel: &str) -> Result<Vec<gp_runtime::Entry>, String> {
        let a = sel
            .strip_prefix("ram[0x")
            .and_then(|s| s.strip_suffix(']'))
            .and_then(|h| usize::from_str_radix(h, 16).ok())
            .ok_or(format!("{sel}?"))?;
        Ok(vec![gp_runtime::Entry {
            path: sel.into(),
            value: Value::Int(i64::from(self.byte(a)?)),
        }])
    }
    fn sample(
        &mut self,
        _: &[String],
        _: &[InputFrame],
        _: u32,
    ) -> Result<Vec<Vec<gp_runtime::Entry>>, String> {
        Err("no".into())
    }
    fn entities_at(&self, _: gp_runtime::Rect) -> Vec<u64> {
        Vec::new()
    }
    fn patch(&mut self, _: &str) -> gp_runtime::PatchResult {
        gp_runtime::PatchResult::NeedsReload
    }
    fn set(&mut self, _: &str, _: Value) -> Result<(), String> {
        Err("no".into())
    }
    fn explain(&self, _: &str) -> Vec<String> {
        Vec::new()
    }
}

/// Run right, jump twice, keep running.
fn lockstep_input() -> Vec<InputFrame> {
    let mut out = pads(RIGHT, 20);
    out.extend(pads(RIGHT | A, 6));
    out.extend(pads(RIGHT, 30));
    out.extend(pads(RIGHT | A, 6));
    out.extend(pads(RIGHT, 40));
    out
}

fn run_lockstep(
    original: Interp,
    mode: crate::lockstep::Mode,
    from: Option<u64>,
) -> crate::lockstep::Lockstep {
    let mut model = load();
    let spec = model.spec().clone();
    let mapping = crate::mapping::Mapping::parse(RAM_MAPPING).unwrap();
    let opts = crate::lockstep::Options {
        mode,
        tolerance: Vec::new(),
        from,
        start_state: None,
        values: false,
        until_start: false,
        sound: false,
        stop_at_split: false,
    };
    crate::lockstep::lockstep(
        &mut AsRam(original),
        &mut model,
        &spec,
        &mapping,
        &lockstep_input(),
        &opts,
    )
    .unwrap()
}

/// A lockstep's start sets a draws script's base to the original's frame
/// there, so the script's frames line up from where the run started, not
/// from where the start was asked for.
#[test]
fn a_lockstep_starts_the_draws_at_the_original_frame_it_starts_on() {
    let mut model = load().with_draws(gp_sim::DrawScript::default());
    let spec = model.spec().clone();
    let mapping = crate::mapping::Mapping::parse(RAM_MAPPING).unwrap();
    let opts = crate::lockstep::Options {
        mode: crate::lockstep::Mode::Free,
        tolerance: Vec::new(),
        from: Some(5),
        start_state: None,
        values: false,
        until_start: false,
        sound: false,
        stop_at_split: false,
    };
    let ls = crate::lockstep::lockstep(
        &mut AsRam(load()),
        &mut model,
        &spec,
        &mapping,
        &lockstep_input(),
        &opts,
    )
    .unwrap();
    let start = ls.start.unwrap();
    let d = model.draws().unwrap();
    assert_eq!(d.base, start.original_frame);
    assert_eq!(d.original_frame(start.frame), start.original_frame);
    // Set by hand, before a load: the load starts the script there.
    let mut rt = load().with_draws(gp_sim::DrawScript::default());
    rt.step(&pads(0, 4)).unwrap();
    let state = rt.canonical_state();
    rt.set(crate::runtime::DRAWS_BASE, Value::Int(7)).unwrap();
    rt.load_state(&state).unwrap();
    assert_eq!(rt.draws().unwrap().original_frame(rt.frame()), 7);
}

/// The model beside itself: no distance on any path, no split, in either
/// mode, and translating a state it already has changes nothing.
#[test]
fn lockstep_with_itself_never_splits() {
    use crate::lockstep::Mode;
    for mode in [Mode::Resync, Mode::Free] {
        let ls = run_lockstep(load(), mode, None);
        assert_eq!(ls.frames.len(), lockstep_input().len() + 1);
        assert!(ls.splits.is_empty(), "{mode:?}: {:?}", ls.splits.first());
        assert!(ls.paths.iter().all(|p| p.max == gp_sim::Fx::ZERO));
    }
    let mut rt = load();
    rt.step(&lockstep_input()[..30]).unwrap();
    let mapping = crate::mapping::Mapping::parse(RAM_MAPPING).unwrap();
    let original = AsRam(rt.clone());
    let t = crate::translate::from_original(&original, rt.spec(), rt.world(), &mapping).unwrap();
    assert_eq!(t.world, *rt.world());
    let loaded: Vec<&str> = t
        .with(gp_runtime::LoadHow::Loaded)
        .map(|n| n.path.as_str())
        .collect();
    assert!(loaded.contains(&"entities.hero[0].x") && loaded.contains(&"rules.game.state"));
    // The walkers aren't mapped, so they're named, not guessed.
    assert!(
        t.with(gp_runtime::LoadHow::Missing)
            .any(|n| n.path == "entities.walker[*]"),
        "{:?}",
        t.notes
    );
}

/// Feature `field.case.number`: while its case holds, a number field reads
/// as the case's number, and a case can't give a number to a flag.
#[test]
fn a_number_case_gives_the_field_its_value() {
    let text = RAM_MAPPING
        .replacen("schema = 1", "schema = 3", 1)
        .replacen(
            "frac = 0x42\n",
            "frac = 0x42\n\n[[field.case]]\nwhen = \"ram[0x0040] >= 0\"\nnumber = -1000\n",
            1,
        );
    let mapping = crate::mapping::Mapping::parse(&text).unwrap();
    let ram = vec![7u8; 0x100];
    assert_eq!(
        mapping.fields[0].read_ram(&ram),
        gp_runtime::Value::Fixed(-1000 << 16)
    );
}

/// Feature `field.case.unmeasured`: on frames its case holds, a field
/// isn't compared, so a drift there makes no distance and no split, and a
/// case that gives nothing at all is refused.
#[test]
fn an_unmeasured_case_leaves_its_field_uncompared() {
    use crate::lockstep::Mode;
    let original = edited(
        "acceleration = \"0.25\"\n",
        "acceleration = \"0.125\"\n",
        "",
    );
    let text = RAM_MAPPING.replacen("schema = 1", "schema = 3", 1).replacen(
        "frac = 0x42\n",
        "frac = 0x42\n\n[[field.case]]\nwhen = \"ram[0x0040] >= 0\"\nunmeasured = \"never compared here\"\n",
        1,
    );
    let mapping = crate::mapping::Mapping::parse(&text).unwrap();
    let mut model = load();
    let spec = model.spec().clone();
    let opts = crate::lockstep::Options {
        mode: Mode::Free,
        tolerance: Vec::new(),
        from: None,
        start_state: None,
        values: false,
        until_start: false,
        sound: false,
        stop_at_split: false,
    };
    let ls = crate::lockstep::lockstep(
        &mut AsRam(original),
        &mut model,
        &spec,
        &mapping,
        &lockstep_input(),
        &opts,
    )
    .unwrap();
    let x = ls
        .paths
        .iter()
        .find(|p| p.path == "entities.hero[0].x")
        .unwrap();
    assert_eq!(x.max, gp_sim::Fx::ZERO);
    assert!(x.first_split.is_none());
    let empty = text.replacen("unmeasured = \"never compared here\"\n", "", 1);
    let err = crate::mapping::Mapping::parse(&empty).unwrap_err();
    assert!(err.contains("unmeasured reason"), "{err}");
}

/// An original that accelerates at half the model's rate: in free mode
/// the hero drifts apart and splits once a path; in resync mode every
/// split is put back, so the distances stay small and every split is a
/// frame the model parts on its own. The resync run is deterministic: two
/// runs reach the same state hashes, and loading a resync's state into a
/// fresh model reaches the hash the run had there.
#[test]
fn resync_puts_the_model_back_and_free_run_drifts() {
    use crate::lockstep::Mode;
    let slow = || {
        edited(
            "acceleration = \"0.25\"\n",
            "acceleration = \"0.125\"\n",
            "",
        )
    };
    let free = run_lockstep(slow(), Mode::Free, None);
    let resync = run_lockstep(slow(), Mode::Resync, None);
    let x = |ls: &crate::lockstep::Lockstep| ls.paths[0].clone();
    assert_eq!(x(&free).first_split, x(&resync).first_split);
    assert!(x(&free).first_split.is_some());
    assert!(
        x(&free).max > x(&resync).max,
        "free {} resync {}",
        x(&free).max,
        x(&resync).max
    );
    assert!(resync.splits.len() > free.splits.len());
    // Free mode lists a path's split once while it stays apart.
    assert!(
        free.splits
            .iter()
            .filter(|s| s.paths.iter().any(|p| p.path == "entities.hero[0].x"))
            .count()
            < 4
    );
    assert!(!resync.resync_notes.is_empty());

    let again = run_lockstep(slow(), Mode::Resync, None);
    assert_eq!(again.hashes, resync.hashes);
    assert_eq!(again.splits, resync.splits);

    // Replaying from a resync: the original's state at the split, loaded
    // into a fresh model, then stepped, reaches the run's next hash.
    let split = &resync.splits[resync.splits.len() / 2];
    let mut original = AsRam(slow());
    let mut model = load();
    let input = lockstep_input();
    let mapping = crate::mapping::Mapping::parse(RAM_MAPPING).unwrap();
    // No lag and no start condition: original frame n is model frame n.
    original.step(&input[..split.frame as usize]).unwrap();
    let mut probe = model.clone();
    let opts = crate::lockstep::Options {
        mode: Mode::Resync,
        tolerance: Vec::new(),
        from: None,
        start_state: None,
        values: false,
        until_start: false,
        sound: false,
        stop_at_split: false,
    };
    let spec = model.spec().clone();
    let upto = crate::lockstep::lockstep(
        &mut AsRam(slow()),
        &mut probe,
        &spec,
        &mapping,
        &input[..split.frame as usize],
        &opts,
    )
    .unwrap();
    assert_eq!(upto.hashes.last(), Some(&resync.hashes[split.at]));
    let base = gp_sim::World::from_canonical(&spec, &probe.canonical_state()).unwrap();
    let t = crate::translate::from_original(&original, &spec, &base, &mapping).unwrap();
    model.load_state(&t.state()).unwrap();
    assert_eq!(model.state_hash(), resync.hashes[split.at]);
    model
        .step(&input[split.frame as usize..=split.frame as usize])
        .unwrap();
    // The next frame's hash, unless the next frame split again and was
    // put back too.
    let next = split.at + 1;
    if resync.splits.iter().all(|s| s.at != next) {
        assert_eq!(model.state_hash(), resync.hashes[next]);
    }
}

/// Starting somewhere: the original's state at a frame is translated in,
/// and the paths agree there.
#[test]
fn lockstep_starts_from_the_originals_state() {
    use crate::lockstep::Mode;
    let slow = edited(
        "acceleration = \"0.25\"\n",
        "acceleration = \"0.125\"\n",
        "",
    );
    let ls = run_lockstep(slow, Mode::Free, Some(40));
    let start = ls.start.as_ref().expect("started");
    assert_eq!(start.original_frame, 40);
    assert_eq!(ls.frames[0], (40, 40));
    assert!(ls.paths.iter().all(|p| p.distance[0] == gp_sim::Fx::ZERO));
    assert!(
        start
            .notes
            .iter()
            .any(|n| n.path == "entities.hero[0].x" && n.how == gp_runtime::LoadHow::Loaded)
    );
}

/// Distances: a number's in its units, whole parts only for `whole`, a
/// flag's or a name's 0 or 1.
#[test]
fn distances_are_in_the_paths_units() {
    use crate::lockstep::distance;
    use gp_sim::Fx;
    let mapping = crate::mapping::Mapping::parse(RAM_MAPPING).unwrap();
    let x = &mapping.fields[0];
    let half = Fx::ONE.raw() / 2;
    assert_eq!(
        distance(x, &Value::Fixed(3 * Fx::ONE.raw()), &Value::Fixed(half)),
        Fx::from_raw(5 * half)
    );
    let mut whole = x.clone();
    whole.whole = true;
    assert_eq!(
        distance(
            &whole,
            &Value::Fixed(3 << 16),
            &Value::Fixed((3 << 16) + half)
        ),
        Fx::ZERO
    );
    assert_eq!(
        distance(&mapping.fields[4], &Value::Flag(true), &Value::Flag(false)),
        Fx::ONE
    );
    assert_eq!(
        distance(
            &mapping.fields[6],
            &Value::Text("dead".into()),
            &Value::Text("dead".into())
        ),
        Fx::ZERO
    );
    assert_eq!(
        distance(x, &Value::Fixed(1 << 16), &Value::Int(1)),
        Fx::ZERO
    );
}

/// A counter with an `every` change, for the ticks tests: down by one
/// every 8 frames.
const CLOCK: &str =
    "[counters.clock]\nstart = 100\n\n[[counters.clock.change]]\nevery = 8\nby = -1\n";

/// The model with [`CLOCK`], as an original that keeps the clock the way
/// Super Mario Bros. keeps its timer: the value at 0x50, and at 0x51 a
/// countdown to the next change, 8 just after one and 1 on the frame
/// before the next.
#[derive(Clone)]
struct Clocked(Interp);

fn clocked() -> Interp {
    edited("name = \"tiny\"\n", "name = \"tiny\"\n", CLOCK)
}

impl Runtime for Clocked {
    fn capabilities(&self) -> gp_runtime::Capabilities {
        gp_runtime::Capabilities::FORK
    }
    fn step(&mut self, inputs: &[InputFrame]) -> Result<Vec<gp_runtime::Event>, String> {
        self.0.step(inputs)
    }
    fn run_until(
        &mut self,
        _: &str,
        _: &[InputFrame],
        _: u32,
    ) -> Result<gp_runtime::Until, String> {
        Err("no".into())
    }
    fn frame(&self) -> u64 {
        self.0.frame()
    }
    fn snapshot(&self) -> Vec<u8> {
        self.0.snapshot()
    }
    fn restore(&mut self, s: &[u8]) -> Result<(), String> {
        self.0.restore(s)
    }
    fn load_state(&mut self, _: &[u8]) -> Result<Vec<gp_runtime::LoadNote>, String> {
        Err("an original has no model state".into())
    }
    fn fork(&self) -> Self {
        self.clone()
    }
    fn canonical_state(&self) -> Vec<u8> {
        self.0.canonical_state()
    }
    fn state_hash(&self) -> u64 {
        self.0.state_hash()
    }
    fn query(&self, sel: &str) -> Result<Vec<gp_runtime::Entry>, String> {
        let v = match sel {
            "ram[0x0050]" => num(&self.0, "counters.clock"),
            "ram[0x0051]" => 8 - num(&self.0, "ticks.clock[0]"),
            // Two held-back changes' slots: the second holds amount 2, with
            // its timer 3 above the value that makes the change.
            "ram[0x0060]" | "ram[0x0070]" => 0,
            "ram[0x0061]" => 2,
            "ram[0x0071]" => 0x2e,
            _ => return Err(format!("{sel}?")),
        };
        Ok(vec![gp_runtime::Entry {
            path: sel.into(),
            value: Value::Int(v),
        }])
    }
    fn sample(
        &mut self,
        _: &[String],
        _: &[InputFrame],
        _: u32,
    ) -> Result<Vec<Vec<gp_runtime::Entry>>, String> {
        Err("no".into())
    }
    fn entities_at(&self, _: gp_runtime::Rect) -> Vec<u64> {
        Vec::new()
    }
    fn patch(&mut self, _: &str) -> gp_runtime::PatchResult {
        gp_runtime::PatchResult::NeedsReload
    }
    fn set(&mut self, _: &str, _: Value) -> Result<(), String> {
        Err("no".into())
    }
    fn explain(&self, _: &str) -> Vec<String> {
        Vec::new()
    }
}

/// The clock's mapping, with or without its count toward the next change.
fn clock_mapping(ticks: bool) -> crate::mapping::Mapping {
    let mut text = String::from(
        "schema = 2\nmodel = \"tiny\"\nsource = \"tiny\"\nalign = \"polled\"\n\n[[field]]\npath = \"counters.clock\"\nram = [0x50]\n",
    );
    if ticks {
        text.push_str(
            "\n[[field]]\npath = \"ticks.clock[0]\"\nram = [0x51]\nscale = -1\noffset = 8\n",
        );
    }
    crate::mapping::Mapping::parse(&text).unwrap()
}

/// The state view names each `every` change's count, 0 just after the
/// change and up to one less than its period.
#[test]
fn the_view_names_each_every_changes_count() {
    let mut rt = clocked();
    assert_eq!(num(&rt, "ticks.clock[0]"), 0);
    rt.step(&pads(0, 13)).unwrap();
    assert_eq!(num(&rt, "ticks.clock[0]"), 5);
    assert_eq!(num(&rt, "counters.clock"), 99);
    rt.step(&pads(0, 3)).unwrap();
    assert_eq!(num(&rt, "ticks.clock[0]"), 0);
    assert_eq!(num(&rt, "counters.clock"), 98);
}

/// A change the original holds back loads as a waiting change in place of
/// the base world's, and reaches the counter the frame after the timer
/// reads `at`, 4 frames on here.
#[test]
fn a_change_the_original_holds_back_comes_on_its_frame() {
    let held = "schema = 15\nmodel = \"tiny\"\nsource = \"tiny\"\nalign = \"polled\"\n\n[[field]]\npath = \"counters.clock\"\nram = [0x50]\n\n[[pending]]\ncounter = \"clock\"\ncount = 2\namount = 0x60\ntimer = 0x70\nat = 0x2b\nby = [0, 0, -7]\n";
    let mapping = crate::mapping::Mapping::parse(held).unwrap();
    let mut rt = clocked();
    let mut base = rt.world().clone();
    base.pending.push((0, -1, 9));
    let original = Clocked(clocked());
    let t = crate::translate::from_original(&original, rt.spec(), &base, &mapping).unwrap();
    assert_eq!(t.world.pending, vec![(0, -7, 4)]);
    rt.load_state(&t.state()).unwrap();
    rt.step(&pads(0, 3)).unwrap();
    assert_eq!(num(&rt, "counters.clock"), 100);
    rt.step(&pads(0, 1)).unwrap();
    assert_eq!(num(&rt, "counters.clock"), 93);
}

/// Started mid-count from the original's state, a counter with its count
/// mapped changes on the original's schedule; without it, on the model's
/// own, so it splits a frame or more off.
#[test]
fn an_every_counter_loaded_mid_count_steps_on_the_originals_schedule() {
    use crate::lockstep::{Mode, Options};
    let run = |ticks: bool| {
        let mut model = clocked();
        let spec = model.spec().clone();
        let opts = Options {
            mode: Mode::Free,
            from: Some(13),
            ..Options::default()
        };
        crate::lockstep::lockstep(
            &mut Clocked(clocked()),
            &mut model,
            &spec,
            &clock_mapping(ticks),
            &pads(0, 60),
            &opts,
        )
        .unwrap()
    };
    let with = run(true);
    let start = with.start.as_ref().expect("started");
    let note = |path: &str| start.notes.iter().find(|n| n.path == path).cloned();
    assert_eq!(
        note("ticks.clock[0]").map(|n| n.how),
        Some(gp_runtime::LoadHow::Loaded)
    );
    assert!(with.splits.is_empty(), "{:?}", with.splits.first());
    assert_eq!(with.paths[0].within, with.frames.len() as u64);

    let without = run(false);
    let start = without.start.as_ref().expect("started");
    assert!(
        start
            .notes
            .iter()
            .any(|n| n.path == "ticks.clock[0]" && n.how == gp_runtime::LoadHow::Missing),
        "{:?}",
        start.notes
    );
    // Loaded at 5 of 8 into the count, the model's own count starts at 0
    // and changes 5 frames late.
    let split = without.splits.first().expect("splits without the count");
    assert_eq!(split.paths[0].path, "counters.clock");
    assert_eq!(split.frame, 16);
}

/// A mapping's `[scope]` stops a run at the first frame out of scope: a
/// `leaves` selector's value changing from the first compared frame's, or
/// a `while` that stops holding. The frames from there are counted apart.
#[test]
fn a_run_stops_where_the_mappings_scope_ends() {
    use crate::lockstep::{Mode, Options};
    let run = |scope: &str| {
        let mut model = clocked();
        let spec = model.spec().clone();
        let mut text = String::from(
            "schema = 2\nmodel = \"tiny\"\nsource = \"tiny\"\nalign = \"polled\"\n\n[[field]]\npath = \"counters.clock\"\nram = [0x50]\n",
        );
        text.push_str(scope);
        let mapping = crate::mapping::Mapping::parse(&text).unwrap();
        let opts = Options {
            mode: Mode::Resync,
            from: Some(13),
            ..Options::default()
        };
        crate::lockstep::lockstep(
            &mut Clocked(clocked()),
            &mut model,
            &spec,
            &mapping,
            &pads(0, 60),
            &opts,
        )
        .unwrap()
    };
    let all = run("");
    assert_eq!((all.scope_end, all.out_of_scope), (None, 0));
    // The clock reads 99 from frame 8 and 98 from frame 16.
    let leaves = run("\n[scope]\nleaves = [\"ram[0x50]\"]\n");
    assert_eq!(leaves.scope_end, Some(16));
    assert_eq!(leaves.out_of_scope, 45);
    assert_eq!(leaves.frames.last().map(|f| f.1), Some(15));
    let held = run("\n[scope]\nwhile = \"ram[0x50] >= 98\"\n");
    assert_eq!(held.scope_end, Some(24));
    let empty = crate::mapping::Mapping::parse(
        "schema = 2\nmodel = \"tiny\"\nsource = \"tiny\"\nalign = \"polled\"\n[[field]]\npath = \"counters.clock\"\nram = [0x50]\n[scope]\n",
    )
    .unwrap_err();
    assert!(empty.contains("`[scope]` needs"), "{empty}");
}

/// A count read past the change's last frame changes on the next frame,
/// as the last frame's does; a ticks path needs schema 2 and names one
/// change.
#[test]
fn ticks_are_clamped_and_checked_when_parsed() {
    let rt = clocked();
    let spec = rt.spec().clone();
    let mapping = clock_mapping(true);
    let t = crate::translate::translate(
        &spec,
        rt.world(),
        &mapping,
        &[Value::Int(100), Value::Int(8)],
        None,
    )
    .unwrap();
    assert_eq!(t.world.ticks, vec![7]);
    let t = crate::translate::translate(
        &spec,
        rt.world(),
        &mapping,
        &[Value::Int(100), Value::Int(-1)],
        None,
    )
    .unwrap();
    assert_eq!(t.world.ticks, vec![0]);
    assert!(
        t.with(gp_runtime::LoadHow::Approximated)
            .any(|n| n.path == "ticks.clock[0]")
    );
    let one = "schema = 1\nmodel = \"tiny\"\nsource = \"tiny\"\nalign = \"polled\"\n\n[[field]]\npath = \"ticks.clock[0]\"\nram = [0x51]\n";
    assert!(
        crate::mapping::Mapping::parse(one)
            .unwrap_err()
            .contains("schema 2")
    );
    let bare = one
        .replace("schema = 1", "schema = 2")
        .replace("ticks.clock[0]", "ticks.clock");
    assert!(
        crate::mapping::Mapping::parse(&bare)
            .unwrap_err()
            .contains("ticks.<counter>[i]")
    );
    let missing = crate::translate::translate(
        &spec,
        rt.world(),
        &clock_mapping(false),
        &[Value::Int(100)],
        None,
    )
    .unwrap();
    assert!(
        missing
            .with(gp_runtime::LoadHow::Missing)
            .any(|n| n.path == "ticks.clock[0]")
    );
}

/// A two-state machine on the player, loaded from a mapped byte: the
/// state the byte names is entered with what its `set` changes, a case
/// over another byte gives a third state, and one that only approximates
/// says why.
#[test]
fn a_mapped_byte_enters_the_players_machine_state_with_its_set() {
    let interp = edited(
        "[entities.hero.run]",
        "[entities.hero.machine]\ninitial = \"small\"\n\n[entities.hero.machine.states.small]\n\n[entities.hero.machine.states.big.set.hitbox]\nx = 0\ny = 0\nw = 16\nh = 32\n\n[entities.hero.machine.states.hurt]\n\n[[entities.hero.machine.states.hurt.transition]]\nafter = 10\nto = \"small\"\n\n[entities.hero.run]",
        "",
    );
    let text = r#"
schema = 3
model = "tiny"
source = "tiny"
align = "polled"

[[field]]
path = "entities.hero[0].state"
ram = [0x50]
values = ["small", "big"]

[[field.case]]
when = "ram[0x0050] == 0 && ram[0x0051] != 0"
value = "hurt"

[[field.case]]
when = "ram[0x0052] != 0"
approximated = "the star, which the model hasn't"
"#;
    let mapping = crate::mapping::Mapping::parse(text).unwrap();
    let spec = interp.spec().clone();
    let base = gp_sim::World::power_on(&spec);
    let hero = spec.kinds.iter().position(|k| k.name == "hero").unwrap();
    let state = |w: &gp_sim::World| {
        let b = w.bodies.iter().find(|b| b.kind == hero).unwrap();
        let m = spec.kinds[hero].machine.as_ref().unwrap();
        (
            m.states[b.state].name.clone(),
            spec.kind(b).hitbox.h.floor(),
        )
    };
    assert_eq!(state(&base), ("small".to_owned(), 16));
    let start = |bytes: [u8; 3]| {
        let mut ram = vec![0u8; 0x800];
        ram[0x50..0x53].copy_from_slice(&bytes);
        let original = Scripted {
            ram,
            frame: 0,
            defeated_from: None,
        };
        crate::translate::from_original(&original, &spec, &base, &mapping).unwrap()
    };
    let how = |t: &crate::translate::Translation, p: &str| {
        t.notes
            .iter()
            .find(|n| n.path == p)
            .map(|n| (n.how, n.why.clone()))
            .unwrap()
    };
    let big = start([1, 0, 0]);
    assert_eq!(state(&big.world), ("big".to_owned(), 32));
    assert_eq!(
        how(&big, "entities.hero[0].state").0,
        gp_runtime::LoadHow::Loaded
    );
    let time = how(&big, "entities.hero[0].time");
    assert_eq!(time.0, gp_runtime::LoadHow::Approximated);
    assert!(time.1.contains("big entered now"), "{}", time.1);
    assert_eq!(
        big.notes
            .iter()
            .filter(|n| n.path == "entities.hero[0].time")
            .count(),
        1
    );

    let small = start([0, 0, 0]);
    assert_eq!(state(&small.world), ("small".to_owned(), 16));
    assert_eq!(
        how(&small, "entities.hero[0].state").0,
        gp_runtime::LoadHow::Loaded
    );
    assert_eq!(
        how(&small, "entities.hero[0].time").0,
        gp_runtime::LoadHow::Missing
    );

    let hurt = start([0, 8, 0]);
    assert_eq!(state(&hurt.world), ("hurt".to_owned(), 16));

    let star = start([1, 0, 0x23]);
    assert_eq!(state(&star.world), ("big".to_owned(), 32));
    let n = how(&star, "entities.hero[0].state");
    assert_eq!(n.0, gp_runtime::LoadHow::Approximated);
    assert!(n.1.ends_with("the star, which the model hasn't"), "{}", n.1);

    // The comparison reads the case too.
    let mut ram = vec![0u8; 0x800];
    ram[0x51] = 8;
    assert_eq!(mapping.fields[0].read_ram(&ram), Value::Text("hurt".into()));

    // Cases need schema 3, a named-value field for a name, and a reason
    // or a name.
    let two = text.replace("schema = 3", "schema = 2");
    assert!(
        crate::mapping::Mapping::parse(&two)
            .unwrap_err()
            .contains("schema 3")
    );
    let unnamed = text.replace("values = [\"small\", \"big\"]", "");
    assert!(
        crate::mapping::Mapping::parse(&unnamed)
            .unwrap_err()
            .contains("is a named value")
    );
    let bare = text.replace("approximated = \"the star, which the model hasn't\"", "");
    assert!(
        crate::mapping::Mapping::parse(&bare)
            .unwrap_err()
            .contains("gives a value")
    );
}

// Bodies as measures (crate::bodies): a model original shows its pacers in
// slots, the way a game reuses object slots, and the lockstep pairs each
// slot life with a model body.

/// The small model with pacers that walk back and forth: one at each x in
/// `at`, turning at its x plus `reach` and back at its spawn.
pub(crate) fn pacers(at: &[(i64, i64)]) -> Interp {
    let mut spawns = String::new();
    for (x, reach) in at {
        spawns.push_str(&format!(
            "[[levels.one.spawn]]\nentity = \"pacer\"\nx = {x}\ny = 120\nparams = {{ patrol_end = {} }}\n\n",
            x + reach
        ));
    }
    edited(
        "[cameras.main]",
        &format!(
            "{spawns}[entities.pacer]\nrole = \"enemy\"\nsize = [16, 16]\n\n[entities.pacer.hitbox]\nx = 0\ny = 0\nw = 16\nh = 16\n\n[entities.pacer.patrol]\nperiod = 1\nstep = 1\n\n[cameras.main]"
        ),
        "",
    )
}

const PACER_MAPPING: &str = r#"
schema = 1
model = "tiny"
source = "tiny"
align = "polled"

[[field]]
path = "entities.hero[0].x"
ram = [0x40, 0x41]
frac = 0x42

[slots]
count = 4
active = 0x10
id = 0x18
clock = 0x14

[[slots.field]]
path = "x"
ram = [0x20, 0x28]

[[slots.field]]
path = "y"
ram = [0x30]

[[slots.kind]]
entity = "pacer"
ids = [7]
"#;

/// An original made from a model whose pacers are in slots: slot `i`
/// holds its `i`-th pacer while it is alive and has appeared, with ID 7,
/// its whole x at 0x20 and 0x28 and its y at 0x30. The hero's x is at
/// 0x40 to 0x42, as [`AsRam`] has it.
#[derive(Clone)]
struct Slotted(Interp);

impl Slotted {
    fn byte(&self, a: usize) -> Result<u8, String> {
        let raw = |p: &str| match get(&self.0, p) {
            Value::Fixed(r) => r,
            v => panic!("{p} is {v}"),
        };
        let pacer = |i: usize, f: &str| self.0.query(&format!("entities.pacer[{i}].{f}"));
        let here = |i: usize| {
            pacer(i, "alive").is_ok_and(|e| e[0].value == Value::Flag(true))
                && pacer(i, "waiting").is_ok_and(|e| e[0].value == Value::Flag(false))
        };
        let x = |i: usize| raw(&format!("entities.pacer[{i}].x")) >> 16;
        Ok(match a {
            0x40 => (raw("entities.hero[0].x") >> 16) as u8,
            0x41 => (raw("entities.hero[0].x") >> 24) as u8,
            0x42 => (raw("entities.hero[0].x") >> 8) as u8,
            0x10..=0x13 => u8::from(here(a - 0x10)),
            0x18..=0x1B => 7,
            0x14 => self.0.frame() as u8,
            0x20..=0x23 if here(a - 0x20) => x(a - 0x20) as u8,
            0x28..=0x2B if here(a - 0x28) => (x(a - 0x28) >> 8) as u8,
            0x30..=0x33 if here(a - 0x30) => {
                (raw(&format!("entities.pacer[{}].y", a - 0x30)) >> 16) as u8
            }
            0x20..=0x33 => 0,
            _ => return Err(format!("no RAM at {a:#x}")),
        })
    }
}

impl Runtime for Slotted {
    fn capabilities(&self) -> gp_runtime::Capabilities {
        gp_runtime::Capabilities::FORK
    }
    fn step(&mut self, inputs: &[InputFrame]) -> Result<Vec<gp_runtime::Event>, String> {
        self.0.step(inputs)
    }
    fn run_until(
        &mut self,
        _: &str,
        _: &[InputFrame],
        _: u32,
    ) -> Result<gp_runtime::Until, String> {
        Err("no".into())
    }
    fn frame(&self) -> u64 {
        self.0.frame()
    }
    fn snapshot(&self) -> Vec<u8> {
        self.0.snapshot()
    }
    fn restore(&mut self, s: &[u8]) -> Result<(), String> {
        self.0.restore(s)
    }
    fn load_state(&mut self, _: &[u8]) -> Result<Vec<gp_runtime::LoadNote>, String> {
        Err("an original has no model state".into())
    }
    fn fork(&self) -> Self {
        self.clone()
    }
    fn canonical_state(&self) -> Vec<u8> {
        self.0.canonical_state()
    }
    fn state_hash(&self) -> u64 {
        self.0.state_hash()
    }
    fn query(&self, sel: &str) -> Result<Vec<gp_runtime::Entry>, String> {
        let a = sel
            .strip_prefix("ram[0x")
            .and_then(|s| s.strip_suffix(']'))
            .and_then(|h| usize::from_str_radix(h, 16).ok())
            .ok_or(format!("{sel}?"))?;
        Ok(vec![gp_runtime::Entry {
            path: sel.into(),
            value: Value::Int(i64::from(self.byte(a)?)),
        }])
    }
    fn sample(
        &mut self,
        _: &[String],
        _: &[InputFrame],
        _: u32,
    ) -> Result<Vec<Vec<gp_runtime::Entry>>, String> {
        Err("no".into())
    }
    fn entities_at(&self, _: gp_runtime::Rect) -> Vec<u64> {
        Vec::new()
    }
    fn patch(&mut self, _: &str) -> gp_runtime::PatchResult {
        gp_runtime::PatchResult::NeedsReload
    }
    fn set(&mut self, _: &str, _: Value) -> Result<(), String> {
        Err("no".into())
    }
    fn explain(&self, _: &str) -> Vec<String> {
        Vec::new()
    }
}

/// Both run free from power-on with nothing pressed for `frames` frames.
fn pacer_lockstep(original: Interp, mut model: Interp, frames: usize) -> crate::lockstep::Lockstep {
    let spec = model.spec().clone();
    let mapping = crate::mapping::Mapping::parse(PACER_MAPPING).unwrap();
    let opts = crate::lockstep::Options {
        mode: crate::lockstep::Mode::Free,
        values: true,
        ..crate::lockstep::Options::default()
    };
    crate::lockstep::lockstep(
        &mut Slotted(original),
        &mut model,
        &spec,
        &mapping,
        &pads(0, frames),
        &opts,
    )
    .unwrap()
}

/// The model beside itself: its pacer is paired with the original's slot
/// life on the first frame, its place, direction and being alive are
/// measured and never apart, and its turns are events on both sides on
/// the same frames.
#[test]
fn a_slot_body_is_paired_and_measured() {
    let ls = pacer_lockstep(pacers(&[(100, 40)]), pacers(&[(100, 40)]), 120);
    assert_eq!(ls.bodies.len(), 1, "{:?}", ls.bodies);
    let b = &ls.bodies[0];
    assert_eq!((b.kind.as_str(), b.nth, b.slot), ("pacer", 0, Some(0)));
    assert_eq!((b.from, b.paired), (0, Some(0)));
    let names: Vec<&str> = b.paths.iter().map(|&i| ls.paths[i].path.as_str()).collect();
    assert_eq!(
        names,
        [
            "bodies.pacer[0].x",
            "bodies.pacer[0].y",
            "bodies.pacer[0].moving",
            "bodies.pacer[0].alive"
        ]
    );
    assert!(ls.splits.is_empty(), "{:?}", ls.splits.first());
    let turns: Vec<(usize, bool)> = b
        .events
        .iter()
        .filter(|e| e.what == "turns left")
        .map(|e| (e.at, e.original))
        .collect();
    assert_eq!(turns.len(), 2, "{:?}", b.events);
    assert_eq!(turns[0].0, turns[1].0);
    assert!(crate::bodies::mismatches(b).is_empty());
    // Every frame's values hold the body's measures after the mapping's.
    assert!(
        ls.values
            .iter()
            .all(|(o, m)| o.len() == ls.paths.len() && m.len() == ls.paths.len())
    );
}

/// A model whose pacer walks 4 pixels further before it turns: the turn
/// splits on the pacer's track 4 frames late, and the clip score says so.
#[test]
fn a_broken_patrol_turn_splits_on_the_enemy_track() {
    let ls = pacer_lockstep(pacers(&[(100, 40)]), pacers(&[(100, 44)]), 120);
    let b = &ls.bodies[0];
    let m = crate::bodies::mismatches(b);
    let first = &m[0];
    assert_eq!(first.what, "turns left", "{m:?}");
    let (o, md) = (first.original.unwrap(), first.model.unwrap());
    assert_eq!(md - o, 4);
    let start = ls.frames[0].1;
    let tracks = crate::scenario::tracks(&ls, start);
    let t = tracks.iter().find(|t| t.track == "pacer").unwrap();
    assert_eq!(
        t.reason.as_deref(),
        Some("the pacer turns left 4 frames late")
    );
    assert!(
        t.says.starts_with(&format!(
            "Pacer: wrong from frame {} (the pacer turns left 4 frames late)",
            ls.frames[o].1 - start
        )),
        "{}",
        t.says
    );
    let scores = crate::scenario::bodies(&ls, start);
    assert_eq!(
        scores[0].first.as_deref(),
        Some("the pacer turns left 4 frames late")
    );
    assert_eq!(
        scores[0].says,
        format!(
            "the original's pacer turns left at frame {}, the model's at {}",
            ls.frames[o].1 - start,
            ls.frames[md].1 - start
        )
    );
    // The direction is wrong from the original's turn until the model's,
    // the one turn in the run.
    let moving = ls
        .paths
        .iter()
        .find(|p| p.path == "bodies.pacer[0].moving")
        .unwrap();
    assert_eq!(moving.first_split, Some(o));
    assert_eq!(moving.within, ls.frames.len() as u64 - 4, "{:?}", b.events);
}

/// `stop_at_split` ends a run on its first split: the same first split as
/// the whole run's, on the same frame, and no frame after it. A run that
/// never splits goes to its end.
#[test]
fn a_run_told_to_stop_at_its_first_split_splits_where_the_whole_run_does() {
    let run = |model: &[(i64, i64)], stop: bool| {
        let mut m = pacers(model);
        let spec = m.spec().clone();
        let mapping = crate::mapping::Mapping::parse(PACER_MAPPING).unwrap();
        let opts = crate::lockstep::Options {
            mode: crate::lockstep::Mode::Free,
            values: true,
            stop_at_split: stop,
            ..crate::lockstep::Options::default()
        };
        crate::lockstep::lockstep(
            &mut Slotted(pacers(&[(100, 40)])),
            &mut m,
            &spec,
            &mapping,
            &pads(0, 120),
            &opts,
        )
        .unwrap()
    };
    let whole = run(&[(100, 44)], false);
    let cut = run(&[(100, 44)], true);
    assert!(!whole.splits.is_empty());
    assert_eq!(cut.splits.len(), 1);
    assert_eq!(cut.splits[0], whole.splits[0]);
    assert_eq!(cut.frames.len(), whole.splits[0].at + 1);
    assert!(cut.frames.len() < whole.frames.len());
    let same = run(&[(100, 40)], true);
    assert!(same.splits.is_empty());
    assert_eq!(same.frames.len(), run(&[(100, 40)], false).frames.len());
}

/// Feature `slots.resync`: in resync mode a body's split puts the world
/// back with the original's slot objects carried into the model, and the
/// body pairs again with the carried one, so it stays one body, alive on
/// both sides. Without the feature a body's split loads nothing.
#[test]
fn a_resync_with_slots_resync_carries_the_bodies_back() {
    let run = |text: &str| {
        let mut model = pacers(&[(100, 44)]);
        let spec = model.spec().clone();
        let mapping = crate::mapping::Mapping::parse(text).unwrap();
        let opts = crate::lockstep::Options {
            mode: crate::lockstep::Mode::Resync,
            ..crate::lockstep::Options::default()
        };
        crate::lockstep::lockstep(
            &mut Slotted(pacers(&[(100, 40)])),
            &mut model,
            &spec,
            &mapping,
            &pads(0, 120),
            &opts,
        )
        .unwrap()
    };
    let kept = run(PACER_MAPPING);
    assert!(kept.resync_notes.is_empty(), "a body's split loads nothing");
    let carried = run(&PACER_MAPPING.replacen("[slots]\n", "[slots]\nresync = true\n", 1));
    assert!(
        carried
            .resync_notes
            .iter()
            .any(|n| n.path == "entities.pacer[*]"),
        "{:?}",
        carried.resync_notes
    );
    assert_eq!(carried.bodies.len(), 1, "{:?}", carried.bodies);
    let alive = carried
        .paths
        .iter()
        .find(|p| p.path == "bodies.pacer[0].alive")
        .unwrap();
    assert_eq!(alive.max, gp_sim::Fx::ZERO);
}

/// The pairing is the same on every run, nearest first: two pacers 10
/// pixels apart each go with the model's at the same place, and a pacer
/// only the model has is a body of its own whose being alive differs.
#[test]
fn the_pairing_is_deterministic_and_nearest_first() {
    let run = || {
        pacer_lockstep(
            pacers(&[(100, 40), (110, 40)]),
            pacers(&[(100, 40), (110, 40), (60, 20)]),
            60,
        )
    };
    let (a, b) = (run(), run());
    assert_eq!(a.bodies, b.bodies);
    assert_eq!(a.paths, b.paths);
    assert_eq!(a.hashes, b.hashes);
    let sides: Vec<(Option<u16>, bool, &str)> = a
        .bodies
        .iter()
        .map(|b| (b.slot, b.model.is_some(), b.kind.as_str()))
        .collect();
    assert_eq!(
        sides,
        [
            (Some(0), true, "pacer"),
            (Some(1), true, "pacer"),
            (None, true, "pacer")
        ]
    );
    // Each pair stands at the same place on its first frame.
    for b in &a.bodies[..2] {
        let x = b.paths[0];
        assert_eq!(a.paths[x].max, gp_sim::Fx::ZERO, "{}", a.paths[x].path);
    }
    let alone = &a.bodies[2];
    assert_eq!(alone.one_sided(), Some("model"));
    let alive = a
        .paths
        .iter()
        .find(|p| p.path == "bodies.pacer[2].alive")
        .unwrap();
    assert_eq!(alive.first_split, Some(0));
    let tracks = crate::scenario::tracks(&a, a.frames[0].1);
    let t = tracks.iter().find(|t| t.track == "pacer").unwrap();
    assert_eq!(
        t.reason.as_deref(),
        Some("the model has a pacer the original doesn't")
    );
}

/// A presentation field is in the mapping a translation reads and out of
/// the one a run compares, and needs schema 4.
#[test]
fn a_presentation_field_is_loaded_but_not_measured() {
    let text = RAM_MAPPING.replacen("schema = 1", "schema = 4", 1)
        + "\n[[field]]\npath = \"counters.phase\"\nram = [0x30]\npresentation = true\n";
    let mapping = crate::mapping::Mapping::parse(&text).unwrap();
    let n = mapping.fields.len();
    assert!(mapping.fields[n - 1].presentation);
    let measured = mapping.measured();
    assert_eq!(measured.fields.len(), n - 1);
    assert!(measured.fields.iter().all(|f| !f.presentation));
    let old = text.replacen("schema = 4", "schema = 3", 1);
    let err = crate::mapping::Mapping::parse(&old).unwrap_err();
    assert!(err.contains("needs schema 4"), "{err}");
}

/// What a screen measure is told: each load, and the animation values
/// that came with it.
#[derive(Default)]
struct AnimationSeen(Vec<Vec<(String, i64)>>);

impl crate::lockstep::Screens<AsRam, Interp> for AnimationSeen {
    fn model_changed(&mut self, _: &Interp, _: bool) {}
    fn shots(
        &mut self,
        _: &AsRam,
        _: &Interp,
    ) -> Result<Option<(gp_present::raster::Frame, gp_present::raster::Frame)>, String> {
        Ok(None)
    }
    fn options(&self) -> crate::screen::Options {
        crate::screen::Options::default()
    }
    fn animation(&mut self, _: &Interp, values: &[(String, i64)]) {
        self.0.push(values.to_vec());
    }
}

/// A `present.` path (schema 6) names a body's animation: a translation
/// notes it loaded and leaves the world alone, a run doesn't compare it,
/// and the screen measure gets the original's values after each load.
#[test]
fn a_present_path_loads_the_animation_not_the_world() {
    let text = RAM_MAPPING.replacen("schema = 1", "schema = 6", 1)
        + "\n[[field]]\npath = \"present.hero[0].frame\"\nram = [0x4A]\npresentation = true\n"
        + "\n[[field]]\npath = \"present.hero[0].timer\"\nram = [0x4D]\npresentation = true\n";
    let mapping = crate::mapping::Mapping::parse(&text).unwrap();
    assert_eq!(mapping.animation().fields.len(), 2);
    assert!(
        mapping
            .measured()
            .fields
            .iter()
            .all(|f| !f.path.starts_with("present."))
    );
    for (from, to, why) in [
        ("schema = 6", "schema = 5", "needs schema 6"),
        (
            "present.hero[0].timer",
            "present.hero.timer",
            "present.<kind>[i].frame or .timer",
        ),
        (
            "present.hero[0].timer",
            "present.hero[0].speed",
            "present.<kind>[i].frame or .timer",
        ),
    ] {
        let err = crate::mapping::Mapping::parse(&text.replacen(from, to, 1)).unwrap_err();
        assert!(err.contains(why), "{err}");
    }
    let unflagged = text.replacen(
        "path = \"present.hero[0].frame\"\nram = [0x4A]\npresentation = true",
        "path = \"present.hero[0].frame\"\nram = [0x4A]",
        1,
    );
    let err = crate::mapping::Mapping::parse(&unflagged).unwrap_err();
    assert!(err.contains("presentation = true"), "{err}");

    let mut rt = load();
    rt.step(&lockstep_input()[..30]).unwrap();
    let original = AsRam(rt.clone());
    let t = crate::translate::from_original(&original, rt.spec(), rt.world(), &mapping).unwrap();
    assert_eq!(t.world, *rt.world());
    assert!(
        t.with(gp_runtime::LoadHow::Loaded)
            .any(|n| n.path == "present.hero[0].frame")
    );
    let values = crate::lockstep::animation_of(&original, &mapping).unwrap();
    let on_ground = i64::from(rt.world().bodies[0].on_ground);
    assert_eq!(
        values,
        [
            ("present.hero[0].frame".to_string(), on_ground),
            ("present.hero[0].timer".to_string(), 0)
        ]
    );

    let mut model = load();
    let spec = model.spec().clone();
    let opts = crate::lockstep::Options {
        mode: crate::lockstep::Mode::Free,
        tolerance: Vec::new(),
        from: Some(10),
        start_state: None,
        values: false,
        until_start: false,
        sound: false,
        stop_at_split: false,
    };
    let mut seen = AnimationSeen::default();
    crate::lockstep::lockstep_with(
        &mut AsRam(load()),
        &mut model,
        &spec,
        &mapping,
        &lockstep_input(),
        &opts,
        Some(&mut seen),
    )
    .unwrap();
    assert_eq!(seen.0.len(), 1, "one load, at the start");
    assert_eq!(seen.0[0].len(), 2);
}

#[test]
fn slots_with_a_stride_and_no_id_or_clock_read_each_slot_a_record_apart() {
    let text = SLOT_MAPPING
        .replace("id = 0x12\n", "")
        .replace("clock = 0x09\n", "")
        .replace(
            "path = \"x\"\nram = [0x30, 0x32]\n",
            "path = \"x\"\nram = [0x100]\nstride = 16\n",
        )
        .replace("ids = [9]", "ids = [0]");
    let mapping = crate::mapping::Mapping::parse(&text).unwrap();
    let slots = mapping.slots.as_ref().unwrap();
    assert_eq!((slots.id, slots.clock), (None, None));
    let x = slots.fields.iter().find(|f| f.path == "x").unwrap();
    assert_eq!(x.slot_selectors(0), ["ram[0x0100]"]);
    assert_eq!(x.slot_selectors(1), ["ram[0x0110]"]);
    let used = gp_model::features::used(
        &toml::Value::Table(text.parse().unwrap()),
        crate::mapping_features::FEATURES,
    );
    assert!(used.contains("slots.field.stride"), "{used:?}");

    let mut ram = vec![0u8; 0x800];
    ram[0x11] = 1; // only slot 1 is taken
    ram[0x110] = 77;
    ram[0x35] = 40;
    let original = Scripted {
        ram,
        frame: 0,
        defeated_from: None,
    };
    let got = crate::bodies::carried_slot_objects(&original, slots).unwrap();
    assert_eq!(got.len(), 1, "{got:?}");
    assert_eq!(got[0].slot, 1);
    let x = got[0].values.iter().find(|(p, _)| p == "x").unwrap();
    let y = got[0].values.iter().find(|(p, _)| p == "y").unwrap();
    // x from the sprite record 16 bytes on, y from the byte one on.
    assert_eq!(format!("{:?}", x.1), format!("Fixed({})", 77 << 16));
    assert_eq!(format!("{:?}", y.1), format!("Fixed({})", 40 << 16));

    let zero = text.replace("stride = 16", "stride = 0");
    assert!(crate::mapping::Mapping::parse(&zero).is_err());
}

#[test]
fn slots_with_a_flag_stride_and_a_free_value_read_each_slot_a_record_apart() {
    let text = SLOT_MAPPING
        .replace("id = 0x12\n", "")
        .replace("clock = 0x09\n", "")
        .replace(
            "active = 0x10\n",
            "active = 0x200\nactive_stride = 16\nfree = 0xFF\n",
        )
        .replace("ids = [9]", "ids = [0]");
    let mapping = crate::mapping::Mapping::parse(&text).unwrap();
    let slots = mapping.slots.as_ref().unwrap();
    let used = gp_model::features::used(
        &toml::Value::Table(text.parse().unwrap()),
        crate::mapping_features::FEATURES,
    );
    assert!(used.contains("slots.active_stride"), "{used:?}");
    assert!(used.contains("slots.free"), "{used:?}");

    // Slot 0's flag byte reads $FF, free; slot 1's, 16 bytes on, holds a
    // sprite's y, so it is taken whatever the value.
    let mut ram = vec![0u8; 0x800];
    ram[0x200] = 0xFF;
    ram[0x210] = 0;
    ram[0x31] = 77;
    let original = Scripted {
        ram,
        frame: 0,
        defeated_from: None,
    };
    let got = crate::bodies::carried_slot_objects(&original, slots).unwrap();
    assert_eq!(got.len(), 1, "{got:?}");
    assert_eq!(got[0].slot, 1);
    assert_eq!(slots.active_at(1), 0x210);

    let zero = text.replace("active_stride = 16", "active_stride = 0");
    assert!(crate::mapping::Mapping::parse(&zero).is_err());
}

#[test]
fn an_index_slot_field_reads_the_slots_own_number() {
    let text = SLOT_MAPPING.replace(
        "[[slots.kind]]",
        "[[slots.field]]\npath = \"ext[3]\"\nram = []\nindex = true\noffset = 1\n\n[[slots.kind]]",
    );
    let mapping = crate::mapping::Mapping::parse(&text).unwrap();
    let slots = mapping.slots.as_ref().unwrap();
    let used = gp_model::features::used(
        &toml::Value::Table(text.parse().unwrap()),
        crate::mapping_features::FEATURES,
    );
    assert!(used.contains("slots.field.index"), "{used:?}");
    let f = slots.fields.iter().find(|f| f.index).unwrap();
    assert!(f.slot_selectors(1).is_empty());
    assert_eq!(
        format!("{:?}", f.slot_value(1, &[])),
        format!("Fixed({})", 2 << 16)
    );
    let with_ram = text.replace("ram = []\nindex", "ram = [0x50]\nindex");
    assert!(crate::mapping::Mapping::parse(&with_ram).is_err());
}

#[test]
fn draw_sites_parse_and_a_site_name_is_one_word() {
    let base = SLOT_MAPPING.replace(
        "[slots]",
        "[[draws]]\nsite = \"ladder\"\npc = 0xDB7B\nram = 0x19\n\n[slots]",
    );
    let m = crate::mapping::Mapping::parse(&base).unwrap();
    assert_eq!(m.draws[0].site, "ladder");
    assert_eq!((m.draws[0].pc, m.draws[0].ram), (0xDB7B, 0x19));
    assert!(crate::mapping::Mapping::parse(&base.replace("\"ladder\"", "\"two words\"")).is_err());
}

/// Features `field.nametable`, `field.oam` and `field.oam_tile`: a field
/// reads a name table cell through the mirroring, an OAM byte by index, or
/// a byte of the first sprite with one of its tiles; a case's `ram` reads
/// RAM in its place while it holds. Made-up tiles and addresses.
#[test]
fn a_field_reads_the_name_tables_and_sprite_memory() {
    use crate::mapping::{Mapping, nametable_index};
    assert_eq!(
        nametable_index(0x2085, 1),
        0x085,
        "horizontal: $2000 is bank 0"
    );
    assert_eq!(
        nametable_index(0x2885, 1),
        0x485,
        "horizontal: $2800 is bank 1"
    );
    assert_eq!(
        nametable_index(0x2485, 0),
        0x485,
        "vertical: $2400 is bank 1"
    );
    assert_eq!(nametable_index(0x2C85, 0), 0x485);
    assert_eq!(nametable_index(0x2C85, 2), 0x085, "one-screen lower");
    assert_eq!(nametable_index(0x2C85, 4), 0xC85, "four-screen");
    let text = r#"
schema = 22
model = "tiny"
source = "tiny"
align = "polled"

[[field]]
path = "counters.pose"
nametable = [0x2885]
lookup = [[0x31, 1], [0x32, 2]]
lookup_else = -1

[[field.case]]
when = "ram[0x0010] == 1"
ram = [0x0011]

[[field]]
path = "counters.shown"
oam = [0x09]

[[field]]
path = "counters.body_x"
oam_tile = { tiles = [0x50, 0x51], byte = 3, none = 7 }
"#;
    let m = Mapping::parse(text).unwrap();
    let mut ram = [0u8; 0x800];
    let mut oam = [0u8; 256];
    let mut nt = [0u8; 0x800];
    nt[0x485] = 0x32;
    oam[9] = 0x44;
    oam[4 * 5 + 1] = 0x51;
    oam[4 * 5 + 3] = 0x80;
    let read = |ram: &[u8], oam: &[u8], nt: &[u8], sel: &str| -> Result<i64, String> {
        let at =
            |s: &str| i64::from_str_radix(s.trim_start_matches("0x").trim_end_matches(']'), 16);
        if sel == "mapper.mirroring" {
            return Ok(1);
        }
        let (name, rest) = sel.split_once('[').ok_or("no index")?;
        let i = at(rest).map_err(|e| e.to_string())? as usize;
        Ok(i64::from(match name {
            "ram" => ram[i],
            "oam" => oam[i],
            "nametables" => nt[i],
            _ => return Err(sel.to_string()),
        }))
    };
    let values = |ram: &[u8], oam: &[u8], nt: &[u8]| -> Vec<gp_runtime::Value> {
        m.fields
            .iter()
            .map(|f| f.read(&mut |s| read(ram, oam, nt, s)).unwrap().0)
            .collect()
    };
    let fx = |n: i64| gp_runtime::Value::Fixed(n << 16);
    assert_eq!(values(&ram, &oam, &nt), vec![fx(2), fx(0x44), fx(0x80)]);
    // The case reads RAM; no sprite with the tiles gives `none`.
    ram[0x10] = 1;
    ram[0x11] = 0x31;
    oam[4 * 5 + 1] = 0;
    assert_eq!(values(&ram, &oam, &nt), vec![fx(1), fx(0x44), fx(7)]);
    for bad in [
        "nametable = [0x2885]\nram = [0x10]",
        "nametable = [0x1885]",
        "oam = [0x100]",
    ] {
        let t = text.replacen("nametable = [0x2885]", bad, 1);
        assert!(Mapping::parse(&t).is_err(), "{bad}");
    }
    // Feature `field.case.nametable`: while its case holds, the field
    // reads another cell, through the same mirroring.
    let t = text.replacen(
        "ram = [0x0011]",
        "ram = [0x0011]\n\n[[field.case]]\nwhen = \"ram[0x0010] == 2\"\nnametable = [0x288E]",
        1,
    );
    let m2 = Mapping::parse(&t).unwrap();
    ram[0x10] = 2;
    nt[0x48E] = 0x31;
    let pose = m2.fields[0]
        .read(&mut |s| read(&ram, &oam, &nt, s))
        .unwrap()
        .0;
    assert_eq!(pose, fx(1));
    for bad in [
        "nametable = [0x288E, 0x288F]",
        "nametable = [0x288E]\nram = [0x11]",
    ] {
        let b = t.replacen("nametable = [0x288E]", bad, 1);
        assert!(Mapping::parse(&b).is_err(), "{bad}");
    }
}

/// Feature `slots.vacant_from`: a group's slots from one on read free
/// while a condition holds; the slot must be below the count and the
/// condition well formed.
#[test]
fn slots_vacant_from_names_a_slot_below_the_count() {
    let text = r#"
schema = 22
model = "tiny"
source = "tiny"
align = "polled"

[[field]]
path = "counters.time"
ram = [0x10]

[slots]
count = 4
active = 0x30
vacant_from = { slot = 2, when = "ram[0x0053] != 4" }

[[slots.field]]
path = "x"
ram = [0x40]

[[slots.kind]]
entity = "foe"
ids = [0]
"#;
    let m = crate::mapping::Mapping::parse(text).unwrap();
    let v = m.slots.as_ref().unwrap().vacant_from.as_ref().unwrap();
    assert_eq!((v.slot, v.when.as_str()), (2, "ram[0x0053] != 4"));
    for bad in ["slot = 4", "slot = 2, when = \"ram[\""] {
        let t = text.replacen("slot = 2", bad, 1);
        let t = if bad.contains("when") {
            t.replacen(", when = \"ram[0x0053] != 4\"", "", 1)
        } else {
            t
        };
        assert!(crate::mapping::Mapping::parse(&t).is_err(), "{bad}");
    }
}

/// Feature `field.machine`: a field reads a runtime protocol selector's
/// number, with `bits` applied, and takes no `ram` beside it.
#[test]
fn a_machine_field_reads_a_selector() {
    let text = r#"
schema = 22
model = "tiny"
source = "tiny"
align = "polled"

[[field]]
path = "counters.resume"
machine = "nmi.return_pc"

[[field]]
path = "counters.carry"
machine = "nmi.status"
bits = [0, 1]
"#;
    let m = crate::mapping::Mapping::parse(text).unwrap();
    let read = |sel: &str| -> Result<i64, String> {
        match sel {
            "nmi.return_pc" => Ok(0xF505),
            "nmi.status" => Ok(0xA5),
            _ => Err(sel.to_string()),
        }
    };
    let values: Vec<gp_runtime::Value> = m
        .fields
        .iter()
        .map(|f| f.read(&mut |s| read(s)).unwrap().0)
        .collect();
    let fx = |n: i64| gp_runtime::Value::Fixed(n << 16);
    assert_eq!(values, vec![fx(0xF505), fx(1)]);
    let t = text.replacen(
        "machine = \"nmi.return_pc\"",
        "machine = \"nmi.return_pc\"\nram = [0x10]",
        1,
    );
    assert!(crate::mapping::Mapping::parse(&t).is_err());
}

/// A lockstep resumed from a prefix ([`crate::lockstep::prefix`] then
/// [`crate::lockstep::lockstep_resumed`], the original restored from a
/// snapshot at the prefix's end) is the run from power-on, in both modes,
/// with and without a start condition, for prefixes from the first frame
/// to the one before the start.
#[test]
fn a_resumed_lockstep_is_the_run_from_power_on() {
    let started = RAM_MAPPING.replacen(
        "align = \"polled\"\n",
        "align = \"polled\"\nstart = \"ram[0x40] >= 3\"\n",
        1,
    );
    for text in [RAM_MAPPING.to_owned(), started] {
        let mapping = crate::mapping::Mapping::parse(&text).unwrap();
        let input = lockstep_input();
        for mode in [crate::lockstep::Mode::Free, crate::lockstep::Mode::Resync] {
            for from in [8u64, 25, 60] {
                let opts = crate::lockstep::Options {
                    mode,
                    from: Some(from),
                    values: true,
                    sound: true,
                    ..Default::default()
                };
                let mut model = load();
                let spec = model.spec().clone();
                let plain = crate::lockstep::lockstep(
                    &mut AsRam(load()),
                    &mut model,
                    &spec,
                    &mapping,
                    &input,
                    &opts,
                )
                .unwrap();
                assert!(plain.start.is_some());
                for k in [0, 1, 4, from as usize - 2] {
                    let mut o = AsRam(load());
                    let p = crate::lockstep::prefix(
                        &mut o,
                        &mut load(),
                        &spec,
                        &mapping,
                        &input,
                        &opts,
                        k,
                    )
                    .unwrap();
                    assert_eq!(p.inputs, k);
                    // Restored into a fresh original, as from the cache.
                    let mut again = AsRam(load());
                    again.restore(&o.snapshot()).unwrap();
                    let mut m = load();
                    m.step(&pads(RIGHT, 3)).unwrap();
                    let resumed = crate::lockstep::lockstep_resumed(
                        &mut again, &mut m, &spec, &mapping, &input, &opts, None, p,
                    )
                    .unwrap();
                    assert_eq!(resumed, plain, "from {from}, prefix {k}");
                }
                // A prefix past the start is refused.
                assert!(
                    crate::lockstep::prefix(
                        &mut AsRam(load()),
                        &mut load(),
                        &spec,
                        &mapping,
                        &input,
                        &opts,
                        from as usize + 5,
                    )
                    .is_err()
                );
            }
        }
    }
}

/// The start scan resumed from a prefix ([`crate::corpus::scan::at_frame_prefix`]
/// then [`crate::corpus::scan::at_frame_from`] on a restored original)
/// finds the same frame and model frame as from power-on.
#[test]
fn a_resumed_start_scan_is_the_scan_from_power_on() {
    use crate::corpus::scan::{at_frame, at_frame_from, at_frame_prefix};
    let started = RAM_MAPPING.replacen(
        "align = \"polled\"\n",
        "align = \"polled\"\nstart = \"ram[0x40] >= 3\"\n",
        1,
    );
    let input = lockstep_input();
    for text in [RAM_MAPPING.to_owned(), started] {
        let mapping = crate::mapping::Mapping::parse(&text).unwrap();
        for frame in [6u64, 30, 70] {
            let (o, mf) = at_frame(&AsRam(load()), &mapping, &input, frame).unwrap();
            for k in [0usize, 2, frame as usize - 4] {
                let mut p = AsRam(load());
                let count = at_frame_prefix(&mut p, &mapping, &input, k, frame).unwrap();
                let mut again = AsRam(load());
                again.restore(&p.snapshot()).unwrap();
                let (o2, mf2) = at_frame_from(again, &mapping, &input, k, count, frame).unwrap();
                assert_eq!(
                    (o2.frame(), mf2),
                    (o.frame(), mf),
                    "frame {frame}, prefix {k}"
                );
                assert_eq!(o2.state_hash(), o.state_hash());
            }
        }
    }
}
