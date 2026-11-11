//! The model interpreter as a browser preview, for Milestone 3's playable
//! web build (docs/web.md).
//!
//! [`Preview`] holds one running model, loaded from a model bundle (the
//! bytes `gp record --model` stores), and steps it a frame at a time. Built
//! for `wasm32-unknown-unknown`, the [`abi`] module exports it to
//! JavaScript as a handful of plain functions over linear memory, with no
//! bindings generator, so the module needs no JavaScript glue beyond
//! `scripts/web/page/preview.js`.

use std::fmt::Write as _;

use gp_interp::Interp;
use gp_interp::lifted::Bundle;
use gp_runtime::InputFrame;

/// A running model, and what it shows: the presentation, once loaded,
/// and its animation state, stepped after every frame.
pub struct Preview {
    interp: Interp,
    /// The hooks' label, as `gp` records it (`gp_exts::registry`), empty
    /// for a model with no hook crate.
    hooks: String,
    present: Option<gp_present::Present>,
    anim: gp_present::Animator,
    /// The picture options (`gp_present::view`) and their memory.
    viewer: gp_present::view::Viewer,
    sound: Option<Sound>,
    /// The rewind ring, the runtime shell standard's (`gp_present::shell`,
    /// docs/shell.md): it keeps nothing until [`Preview::ring_reset`] gives
    /// the frame rate.
    ring: gp_present::shell::Ring,
    /// Whether the last tick started a level, so the shell autosaves
    /// (`gp_present::shell::autosaves`).
    autosave_due: bool,
    /// A large-art look's rules (`gp_present::art`) as loaded, and
    /// resolved against the presentation; None draws the scene as it is.
    art_text: Option<String>,
    art: Option<gp_present::art::Art>,
    /// The level pictures' pixels the look's stages were built from
    /// (picture, width, height, RGBA), kept to build them again when the
    /// presentation is loaded afresh.
    stage_px: Vec<(u32, usize, usize, Vec<[u8; 4]>)>,
    /// What the large-art look keeps from frame to frame.
    painter: gp_present::art::Painter,
}

/// The model's sound, played live: the cues say what plays when, gp-sound's
/// player writes the APU registers and its synthesizer makes the samples,
/// the same code and numbers the Bevy build's WAV files were rendered with
/// (docs/sound.md, "Playback"). A model whose hooks keep a game's sound
/// driver as state gives the register writes itself ([`gp_sim::ext::hook_apu`]):
/// once a frame has any, those play and the cues' player stands down.
pub struct Sound {
    cues: gp_present::sound::Cues,
    director: gp_present::sound::Director,
    player: gp_sound::player::Player,
    synth: gp_sound::synth::Synth,
    /// Whether the hooks have given APU writes.
    driven: bool,
    /// A theme's sound per role (`gp_present::sound::RolePlayer`, the code
    /// the Bevy build plays it with): while it holds any, the model's own
    /// sound is silent and the hooks' APU writes aren't worked out.
    pub roles: gp_present::sound::RolePlayer,
    /// The last frame's samples, 16-bit mono at
    /// [`gp_sound::synth::SAMPLE_RATE`].
    pub samples: Vec<i16>,
}

impl Sound {
    /// Reads `assets/sound.json` (`gp_look::sound::web_bundle`).
    pub fn load(text: &str) -> Result<Self, String> {
        let v: serde_json::Value =
            serde_json::from_str(text).map_err(|e| format!("sound.json: {e}"))?;
        let cues =
            gp_present::sound::Cues::read(v["cues"].as_str().ok_or("sound.json has no cues")?)?;
        let mut sets = Vec::new();
        if let Some(o) = v["sets"].as_object() {
            for (slot, set) in o {
                let set = gp_sound::song::SongSet::parse(&set.to_string())?;
                sets.push((slot.clone(), set));
            }
        }
        let refs: Vec<(String, &gp_sound::song::SongSet)> =
            sets.iter().map(|(k, s)| (k.clone(), s)).collect();
        let set = gp_sound::song::SongSet::merged(&refs);
        let synth = gp_sound::synth::Synth::new(
            set.region,
            gp_sound::synth::SAMPLE_RATE,
            set.samples.clone(),
        );
        Ok(Self {
            roles: gp_present::sound::RolePlayer::new(&cues),
            cues,
            director: gp_present::sound::Director::new(),
            player: gp_sound::player::Player::new(set),
            synth,
            driven: false,
            samples: Vec::new(),
        })
    }

    /// Sets a sound role's file (a 16-bit PCM WAV); false when the cues
    /// have no such role.
    pub fn set_role(&mut self, role: &str, wav: &[u8]) -> Result<bool, String> {
        let s = gp_present::sound::read_wav(wav)?;
        Ok(self.roles.set(&self.cues, role, s))
    }

    /// Steps after a tick with its events and the hooks' APU writes for
    /// it: the cues, then one frame of register writes and its samples.
    pub fn step(
        &mut self,
        world: &gp_sim::World,
        events: &[gp_sim::systems::Event],
        hooks: &[(u8, u8)],
    ) {
        if self.roles.is_on() {
            self.samples = self
                .roles
                .frame(&self.cues, world, gp_present::sound::ROLE_FRAME);
            return;
        }
        if !hooks.is_empty() {
            self.driven = true;
        }
        if self.driven {
            let w: Vec<(u32, u8, u8)> = hooks.iter().map(|&(r, v)| (0, r, v)).collect();
            self.samples = self.synth.frame(&w);
            return;
        }
        let c = self.director.step(&self.cues, world, events);
        if let Some(m) = c.music {
            self.player.play_music(m.as_deref());
        }
        for e in c.sfx {
            self.player.play_effect(&e);
        }
        let w = self.player.frame();
        self.samples = self.synth.frame(&w);
    }
}

/// A model bundle in a profile (docs/model.md, "Profiles"): the base
/// bundle's model with the switches `name` turns on, from the text of the
/// model's `profiles.toml`, and the same assets. `retro` is the bundle as
/// it is. Any set of switches works, such as `more_lives+no_timer`, so the
/// page needs no bundle for each.
pub fn profile_bundle(bundle: &[u8], profiles: &str, name: &str) -> Result<Vec<u8>, String> {
    let b = Bundle::from_bytes(bundle)?;
    let p = gp_model::profile::Profiles::parse(profiles)?;
    let m = p.apply(&b.parse_model()?, name)?;
    Ok(Bundle::new(&m, b.assets).to_bytes())
}

/// The standard settings screen's catalog (docs/settings.md) from the text
/// of the model's `profiles.toml`: every group and switch with its
/// description, and the presets. The preview draws the screen with the
/// same `gp_present::settings` code as the generated game.
pub fn settings_catalog(profiles: &str) -> Result<gp_present::settings::Catalog, String> {
    use gp_present::settings as standard;
    let p = gp_model::profile::Profiles::parse(profiles)?;
    Ok(standard::Catalog {
        groups: p
            .groups
            .iter()
            .map(|g| standard::Group {
                title: g.heading().to_owned(),
                description: g.description.clone().unwrap_or_default(),
                switches: g
                    .switches
                    .iter()
                    .map(|s| standard::Switch {
                        name: s.name.clone(),
                        description: s.description.clone(),
                    })
                    .collect(),
            })
            .collect(),
        presets: p
            .presets
            .iter()
            .map(|(name, pr)| standard::Preset {
                name: name.clone(),
                description: pr.description.clone().unwrap_or_default(),
                switches: pr.switches.clone(),
            })
            .collect(),
    })
}

/// A key of the settings screen by the number `gp_settings_key` takes: 0
/// up, 1 down, 2 left, 3 right, 4 choose, 5 back, 6 the settings key.
pub fn settings_key(code: u32) -> Option<gp_present::settings::Key> {
    use gp_present::settings::Key;
    Some(match code {
        0 => Key::Up,
        1 => Key::Down,
        2 => Key::Left,
        3 => Key::Right,
        4 => Key::Choose,
        5 => Key::Back,
        6 => Key::Open,
        _ => return None,
    })
}

impl Preview {
    /// Compiles the model in a bundle and powers it on, with the `ext`
    /// hooks `gp` registers for its model (crates/gp-exts), so the preview
    /// runs the same hooks as the native interpreter that scores it. A
    /// model naming a hook this build has no crate for is refused, as
    /// `gp interp run` refuses it: its entities would do only what their
    /// components say, and nothing would say so.
    pub fn load(bundle: &[u8]) -> Result<Self, String> {
        let bundle = Bundle::from_bytes(bundle)?;
        let interp = Interp::from_bundle(&bundle)?;
        let (exts, hooks) = gp_exts::registry(&interp.spec().name);
        let interp = interp.with_exts(std::sync::Arc::new(exts));
        let missing = interp.ext_hooks();
        if !missing.is_empty() {
            return Err(format!(
                "the model `{}` names ext hooks this gp_web.wasm has no crate for: {}. Register the model's ext/ crate in crates/gp-exts (docs/interpreter.md, \"Hooks in gp\") and build the preview again",
                interp.spec().name,
                missing.join(", ")
            ));
        }
        Ok(Self {
            interp,
            hooks,
            present: None,
            anim: gp_present::Animator::new(),
            viewer: gp_present::view::Viewer::default(),
            sound: None,
            ring: gp_present::shell::Ring::new(gp_present::shell::REWIND_EVERY, 0),
            autosave_due: false,
            art_text: None,
            art: None,
            stage_px: Vec::new(),
            painter: gp_present::art::Painter::new(),
        })
    }

    /// Empties the rewind ring and sizes it for the standard's seconds at
    /// `rate` frames a second (`gp_present::shell::Ring::standard`).
    pub fn ring_reset(&mut self, rate: f64) {
        self.ring = gp_present::shell::Ring::standard(rate);
    }

    /// Empties the rewind ring: after a load, or a restart in other
    /// settings.
    pub fn ring_clear(&mut self) {
        self.ring.clear();
    }

    /// Keeps the game's state in the rewind ring when the standard's step
    /// says so. Called after each frame.
    pub fn ring_note(&mut self) {
        self.ring.note(self.interp.world(), "");
    }

    /// Puts back the newest state in the ring before this frame, as one
    /// step of held rewind. False when the ring has none.
    pub fn ring_back(&mut self) -> Result<bool, String> {
        match self.ring.back(self.interp.world().frame) {
            Some(s) => self.load_state(&s.state).map(|()| true),
            None => Ok(false),
        }
    }

    /// Whether the last tick started a level: the shell's autosave policy
    /// (`gp_present::shell::autosaves`), which writes slot 0 then.
    pub fn autosave_due(&self) -> bool {
        self.autosave_due
    }

    /// A save slot's line for the settings screen's Saves line, from what
    /// the page's storage keeps: the record, if any, and when it was saved.
    /// `profile` is the one the page runs the game in: a record of it with
    /// another digest is from another version.
    pub fn slot_line(
        &self,
        slot: usize,
        bytes: Option<&[u8]>,
        saved_at: Option<u64>,
        profile: &str,
    ) -> gp_present::shell::SlotLine {
        let digest = self.spec_digest();
        gp_present::shell::SlotLine::read(
            slot,
            bytes,
            saved_at,
            self.model(),
            Some((profile, &digest)),
            Some(self.interp.spec()),
        )
    }

    /// The hooks' label, as `gp` records it: the crate's path, version and
    /// code hash, or empty for a model without hooks.
    pub fn hooks(&self) -> &str {
        &self.hooks
    }

    /// Loads the model's sound (`assets/sound.json`), for [`Preview::audio`].
    pub fn load_sound(&mut self, text: &str) -> Result<(), String> {
        self.sound = Some(Sound::load(text)?);
        Ok(())
    }

    /// Sets a theme's sound for a role (a 16-bit PCM WAV file), played on
    /// the role's cue in place of the model's own sound.
    pub fn set_sound_role(&mut self, role: &str, wav: &[u8]) -> Result<bool, String> {
        match &mut self.sound {
            Some(s) => s.set_role(role, wav),
            None => Err("no sound is loaded".into()),
        }
    }

    /// Takes every role's sound away: the model's own sound plays again.
    pub fn clear_sound_roles(&mut self) {
        if let Some(s) = &mut self.sound {
            s.roles.clear();
        }
    }

    /// The last frame's samples, empty without sound.
    pub fn audio(&self) -> &[i16] {
        self.sound.as_ref().map_or(&[], |s| &s.samples)
    }

    /// Loads the presentation `gp gen bevy` and `gp-web-prep` write
    /// (`assets/present.txt`), for [`Preview::scene`].
    pub fn load_present(&mut self, text: &str) -> Result<(), String> {
        let p = gp_present::text::read(text)?;
        // A large-art look loaded before stays, resolved against this
        // presentation (a restart in new settings loads it afresh).
        let mut art = match &self.art_text {
            Some(t) => Some(gp_present::art::Art::new(
                gp_present::art::ArtRules::read(t)?,
                &p,
            )?),
            None => None,
        };
        if let Some(a) = &mut art {
            for (pic, w, h, px) in &self.stage_px {
                a.build_stage(&p, *pic, *w, *h, px)?;
            }
        }
        self.present = Some(p);
        self.art = art;
        self.painter = gp_present::art::Painter::new();
        self.anim = gp_present::Animator::new();
        Ok(())
    }

    /// Loads a large-art look's rules (`gp_present::art`, the file its
    /// `look.txt` names with `art`), after the presentation; an empty text
    /// takes them away. Returns the art files, in the numbers
    /// [`Preview::art_frame`] names them by, as JSON.
    pub fn load_art(&mut self, text: &str) -> Result<String, String> {
        self.stage_px.clear();
        if text.trim().is_empty() {
            self.art_text = None;
            self.art = None;
            return Ok(r#"{"files": [], "stages": []}"#.into());
        }
        let p = self.present.as_ref().ok_or("no presentation loaded")?;
        let art = gp_present::art::Art::new(gp_present::art::ArtRules::read(text)?, p)?;
        let out =
            serde_json::json!({"files": art.files, "stages": art.stage_levels(p)}).to_string();
        self.art_text = Some(text.to_owned());
        self.art = Some(art);
        self.painter = gp_present::art::Painter::new();
        Ok(out)
    }

    /// Builds the large-art look's stage over level picture `pic` from its
    /// pixels (`w` by `h`, RGBA row by row: the picture the runtime has,
    /// lifted from the player's ROM for a ROM build) and returns it as
    /// JSON (`gp_present::art::stage`): `size` (game pixels), `scale`
    /// (stage pixels to a game pixel), `backdrop` (two colors, top and
    /// bottom) and `draws`, each `[src, part, x, y, w, h, flip_x, shade,
    /// clip]` in stage pixels, `src` `["piece", file]` or `["backdrop"]`,
    /// `part` `[x, w]` of the piece in thousandths of its width, `shade` a
    /// brightness in thousandths.
    pub fn art_stage(
        &mut self,
        pic: u32,
        w: usize,
        h: usize,
        rgba: &[u8],
    ) -> Result<String, String> {
        use gp_present::art::stage::StageSrc;
        let p = self.present.as_ref().ok_or("no presentation loaded")?;
        let art = self.art.as_mut().ok_or("no large-art look loaded")?;
        let px: Vec<[u8; 4]> = rgba.as_chunks::<4>().0.to_vec();
        let st = art.build_stage(p, pic, w, h, &px)?;
        let draws: Vec<serde_json::Value> = st
            .draws
            .iter()
            .map(|d| {
                let src = match d.src {
                    StageSrc::Piece(i) => serde_json::json!(["piece", i]),
                    StageSrc::Backdrop => serde_json::json!(["backdrop"]),
                };
                serde_json::json!([src, d.part, d.x, d.y, d.w, d.h, d.flip_x, d.shade, d.clip])
            })
            .collect();
        let out = serde_json::json!({
            "size": st.size, "scale": st.scale, "backdrop": st.backdrop, "draws": draws,
        })
        .to_string();
        self.stage_px.retain(|x| x.0 != pic);
        self.stage_px.push((pic, w, h, px));
        Ok(out)
    }

    /// The picture options a frame is drawn with: the page's, less the
    /// sprite limit where a large-art look says so (its pictures aren't
    /// the console's 8-pixel sprites).
    fn draw_options(&self) -> gp_present::view::Options {
        let mut o = self.viewer.options;
        if self.art.as_ref().is_some_and(|a| !a.rules.limit) {
            o.sprite_limit = false;
        }
        o
    }

    /// A frame of a large-art look as JSON (`gp_present::art::Painter::frame`):
    /// `size` (the window in game pixels), `scale` (canvas pixels to a
    /// game pixel), `clear`, and `draws`, each `[src, x, y, w, h, flip_x,
    /// flip_y, alpha, cut, clip]` in the order painted, where `src` is
    /// `["own", picture]`, `["art", file]`, `["tint", picture, [r, g, b]]`
    /// or `["fill", [r, g, b]]`, places and sizes in sixteenths of a game
    /// pixel from the window's corner, `cut` the parts not painted (game
    /// pixels from the picture's corner, before any flip) and `clip` null
    /// or the only box painted (`[x, y, w, h]` like the place). Null when
    /// no large-art look is loaded.
    pub fn art_frame(&self) -> Result<String, String> {
        use gp_present::art::Src;
        let Some(art) = &self.art else {
            return Ok("null".into());
        };
        let p = self.present.as_ref().ok_or("no presentation loaded")?;
        let (spec, world) = (self.interp.spec(), self.interp.world());
        let scene = self.anim.scene(p, spec, world);
        let mut viewer = self.viewer.clone();
        viewer.options = self.draw_options();
        let f = viewer.frame(p, spec, world, &scene, gp_present::view::SUB);
        let height = p.screen.visible()[3];
        let a = self.painter.frame(art, p, &f, height);
        let draws: Vec<serde_json::Value> = a
            .draws
            .iter()
            .map(|d| {
                let src = match d.src {
                    Src::Own(i) => serde_json::json!(["own", i]),
                    Src::Art(i) => serde_json::json!(["art", i]),
                    Src::Tint(i, c) => serde_json::json!(["tint", i, c]),
                    Src::Fill(c) => serde_json::json!(["fill", c]),
                    Src::Stage(l) => serde_json::json!(["stage", l]),
                    Src::Backdrop => serde_json::json!(["backdrop"]),
                };
                serde_json::json!([
                    src, d.x, d.y, d.w, d.h, d.flip_x, d.flip_y, d.alpha, d.cut, d.clip
                ])
            })
            .collect();
        Ok(serde_json::json!({
            "size": a.size, "scale": a.scale, "clear": a.clear, "draws": draws,
            "backdrop": a.backdrop.map(|(t, b, y0, y1)| serde_json::json!([t, b, y0, y1])),
        })
        .to_string())
    }

    /// Runs one frame with controller 1's bits (A, B, Select, Start, Up,
    /// Down, Left, Right from bit 0). A reset starts the game over.
    pub fn step(&mut self, buttons: u8, reset: bool) -> Result<(), String> {
        self.step_pads(u16::from(buttons), reset)
    }

    /// [`Preview::step`] with controller 1's bits in the low byte and
    /// controller 2's in the high byte, as the interpreter takes them
    /// (`Interp::advance`): a model with core `input.turn` reads the turn's
    /// player's controller.
    pub fn step_pads(&mut self, buttons: u16, reset: bool) -> Result<(), String> {
        let hooks = if self.sound.as_ref().is_some_and(|s| !s.roles.is_on()) {
            gp_sim::ext::hook_apu(self.interp.spec(), self.interp.world(), self.interp.exts())
        } else {
            Vec::new()
        };
        let events = self.interp.advance(&InputFrame {
            buttons: u32::from(buttons),
            axes: Vec::new(),
            reset,
        })?;
        if let Some(s) = &mut self.sound {
            s.step(self.interp.world(), &events, &hooks);
        }
        self.step_view(&events);
        self.autosave_due = gp_present::shell::autosaves(&events);
        Ok(())
    }

    /// Steps the animation and the picture options' memory after a tick
    /// or a state put back.
    fn step_view(&mut self, events: &[gp_sim::Event]) {
        if let Some(p) = &self.present {
            self.anim.step(p, self.interp.spec(), self.interp.world());
            self.anim
                .fired(p, self.interp.spec(), self.interp.world(), events);
            let scene = self.anim.scene(p, self.interp.spec(), self.interp.world());
            self.viewer.step(p, self.interp.world(), &scene);
            if let Some(a) = &self.art {
                self.painter.step(a, &scene);
            }
        }
    }

    /// Sets the picture and sound options from a list, as the Bevy build's
    /// `play --picture` takes it: `wide` (16:9) or `wide=16:10`,
    /// `side=ahead|center|front`, `limit=on|off`, `dim`, `mute=a+b`
    /// (`pulse1`, `pulse2`, `triangle`, `noise`, `dmc`), `off`. The page
    /// draws no smooth motion, since its canvas is the game's pixels.
    pub fn set_view(&mut self, list: &str) -> Result<(), String> {
        use gp_present::view::{ASPECTS, Options, Side, Wide};
        let mut o = self.viewer.options;
        let mut mute = None;
        for item in list.split(',').map(str::trim).filter(|i| !i.is_empty()) {
            let (k, v) = item
                .split_once('=')
                .map_or((item, None), |(k, v)| (k, Some(v)));
            let on = || match v {
                None | Some("on") => Ok(true),
                Some("off") => Ok(false),
                Some(x) => Err(format!("{k}={x}: give on or off")),
            };
            match k {
                "off" => {
                    o = Options::default();
                    mute = Some([false; 5]);
                }
                "wide" => {
                    let side = o.wide.map_or(Side::Ahead, |w| w.side);
                    o.wide = match v {
                        None => Some(Wide {
                            aspect: ASPECTS[ASPECTS.len() - 1],
                            side,
                        }),
                        Some("off") => None,
                        Some(a) => {
                            let aspect = ASPECTS
                                .into_iter()
                                .find(|x| format!("{}:{}", x[0], x[1]) == a)
                                .ok_or_else(|| format!("wide={a}: give off, 16:10 or 16:9"))?;
                            Some(Wide { aspect, side })
                        }
                    };
                }
                "side" => {
                    let side =
                        Side::parse(v.unwrap_or("")).ok_or("side: give ahead, center or front")?;
                    if let Some(w) = &mut o.wide {
                        w.side = side;
                    }
                }
                "limit" => o.sprite_limit = on()?,
                "dim" => o.dim_flashes = on()?,
                "mute" => {
                    let mut m = [false; 5];
                    for c in v.unwrap_or("").split('+').filter(|c| !c.is_empty()) {
                        let ch = gp_sound::apu::Channel::parse(c)
                            .ok_or_else(|| format!("mute={c}: not a channel"))?;
                        m[ch.index()] = true;
                    }
                    mute = Some(m);
                }
                _ => return Err(format!("{item}: give wide, side, limit, dim, mute or off")),
            }
        }
        self.viewer.options = o;
        if let (Some(m), Some(snd)) = (mute, &mut self.sound) {
            snd.synth.gains = m.map(|x| if x { 0.0 } else { 1.0 });
        }
        Ok(())
    }

    /// Loads a canonical model state (`gp_sim::World::canonical`, the
    /// bytes the runtime protocol's `load-state` takes): the original's
    /// state translated into the model's, so the game view starts the
    /// model where the original is (docs/inspector.md, "The same commands
    /// to both"). A state of another model is refused.
    pub fn load_state(&mut self, state: &[u8]) -> Result<(), String> {
        use gp_runtime::Runtime as _;
        self.interp.restore(state)?;
        self.anim = gp_present::Animator::new();
        self.viewer.reset();
        self.step_view(&[]);
        Ok(())
    }

    /// What to draw this frame, as JSON, from the presentation
    /// (`gp_present::Animator::scene`): `clear`, `camera`, `level` (a
    /// picture number or null), `tints` (`[picture, [r, g, b]]`),
    /// `cells` (the cells the game changed, `[picture, x, y, flip_x,
    /// flip_y]`, drawn over the level and its tints), `bodies` (`[picture,
    /// x, y, flip_x, flip_y]` or null), `order` (the bodies' numbers in
    /// the order to paint them, the last on top: `Scene::paint_order`,
    /// the console's sprite table, as the native picture and the Bevy
    /// build paint them), `behind` (by
    /// body: drawn before the level picture), `front` (by body: drawn after
    /// every other body), `cut` (by body: the parts the console's sprite
    /// limit drops, `[x, y, w, h]` from the picture's top left on the
    /// screen, before any flip), `hud` and
    /// `hud_tints` in screen pixels (the last `hud_sprites` of `hud` are
    /// sprites, drawn over the bodies; the rest is background, under
    /// them), drawn with the picture options
    /// ([`Preview::set_view`]): `window` (`x`, `y`, `w`, `h` in world
    /// pixels, `view_x`, `wall`, and `hud`, the HUD's screen corner in
    /// world pixels), `alpha` (each body's opacity, 0 to 255) and
    /// `ghosts` (`[picture, x, y, flip_x, flip_y, alpha]`).
    pub fn scene(&self) -> Result<String, String> {
        let p = self.present.as_ref().ok_or("no presentation loaded")?;
        let (spec, world) = (self.interp.spec(), self.interp.world());
        let scene = self.anim.scene(p, spec, world);
        let f = self
            .viewer
            .frame(p, spec, world, &scene, gp_present::view::SUB);
        let s = &f.scene;
        let w = f.window;
        let draw = |d: &gp_present::Draw| serde_json::json!([d.pic, d.x, d.y, d.flip_x, d.flip_y]);
        Ok(serde_json::json!({
            "window": {"x": w.x, "y": w.y, "w": w.w, "h": w.h, "view_x": w.view_x, "wall": w.wall, "hud": w.hud},
            "alpha": f.alpha,
            "ghosts": f.ghosts.iter().map(|(d, a)| serde_json::json!([d.pic, d.x, d.y, d.flip_x, d.flip_y, a])).collect::<Vec<_>>(),
            "clear": s.clear,
            "camera": s.camera,
            "level": s.level,
            "tints": s.tints.iter().map(|(p, c)| serde_json::json!([p, c])).collect::<Vec<_>>(),
            "cells": s.cells.iter().map(draw).collect::<Vec<_>>(),
            "bodies": s.bodies.iter().map(|d| d.as_ref().map(draw)).collect::<Vec<_>>(),
            "order": s.paint_order(),
            "behind": s.behind,
            "front": s.front,
            "cut": s.cut,
            "split": s.split,
            "hud": s.hud.iter().map(draw).collect::<Vec<_>>(),
            "hud_sprites": s.hud_sprites,
            "hud_tints": s.hud_tints.iter().map(|(p, c)| serde_json::json!([p, c])).collect::<Vec<_>>(),
        })
        .to_string())
    }

    /// BLAKE3 of the compiled spec's debug form, in hex: the digest
    /// `gp gen bevy` writes into a generated game (`gp_look::spec_digest`).
    pub fn spec_digest(&self) -> String {
        gp_sim::save::spec_digest(self.interp.spec())
    }

    /// The model's name, as a save record names its game.
    pub fn model(&self) -> &str {
        &self.interp.spec().name
    }

    /// The game as a save record (`gp_sim::save`): the model's name, this
    /// build's digest, the profile the page runs it in (the page knows its
    /// name) and the canonical state. The generated game writes the same
    /// record (docs/web.md, "The shell").
    pub fn save_record(&self, profile: &str) -> Vec<u8> {
        gp_sim::save::Record::new(
            &self.interp.spec().name,
            &self.spec_digest(),
            profile,
            self.state(),
        )
        .to_bytes()
    }

    /// Loads a save record [`Preview::save_record`] made, refusing one of
    /// another game or another build (`gp_sim::save::OTHER_BUILD`). The
    /// page compares the record's profile with its own first, so the
    /// digest asked for is this preview's.
    pub fn load_record(&mut self, bytes: &[u8]) -> Result<(), String> {
        let digest = self.spec_digest();
        let name = self.interp.spec().name.clone();
        let r = gp_sim::save::Record::load(bytes, &name, |_| Ok(digest))?;
        self.load_state(&r.state)
    }

    /// The canonical state's BLAKE3 hash, as `headless --hashes` writes it.
    pub fn hash(&self) -> [u8; 32] {
        self.interp.world().hash()
    }

    /// The canonical model state (`gp_sim::World::canonical`), which
    /// [`Preview::load_state`] puts back: the page keeps it for save slots
    /// and rewinding (docs/web.md, "The shell").
    pub fn state(&self) -> Vec<u8> {
        self.interp.world().canonical()
    }

    /// What a renderer needs this frame, as JSON: the frame, the level's
    /// name, the first camera's top left corner in whole pixels, and each
    /// body's kind name, position in whole pixels, facing and whether it's
    /// alive.
    pub fn draw_state(&self) -> String {
        let spec = self.interp.spec();
        let w = self.interp.world();
        let [cx, cy] = w.cameras.first().copied().unwrap_or_default();
        let bodies: Vec<serde_json::Value> = w
            .bodies
            .iter()
            .map(|b| {
                serde_json::json!([
                    spec.kinds
                        .get(b.kind)
                        .map(|k| k.name.as_str())
                        .unwrap_or(""),
                    b.x.raw() >> 16,
                    b.y.raw() >> 16,
                    b.facing_left,
                    b.alive,
                ])
            })
            .collect();
        serde_json::json!({
            "frame": w.frame,
            "level": spec.levels.get(w.level).map(|l| l.name.as_str()).unwrap_or(""),
            "camera": [cx.raw() >> 16, cy.raw() >> 16],
            "bodies": bodies,
        })
        .to_string()
    }

    /// Runs a frames file (`gp-frames 1`, docs/generator.md) from this
    /// state and returns the hash lines `headless --hashes` would write:
    /// this state's first, then one per frame.
    pub fn run_frames(&mut self, frames: &str) -> Result<String, String> {
        let mut out = String::new();
        hash_line(&mut out, &self.hash());
        for (b, r) in parse_frames(frames)? {
            self.step_pads(b, r)?;
            hash_line(&mut out, &self.hash());
        }
        Ok(out)
    }
}

/// Reads a frames file's lines into controller bits (controller 2's in the
/// high byte, from the word after controller 1's when there is one) and
/// resets.
pub fn parse_frames(text: &str) -> Result<Vec<(u16, bool)>, String> {
    let mut lines = text.lines();
    if lines.next() != Some(gp_frames_header()) {
        return Err("not a frames file".into());
    }
    lines
        .enumerate()
        .map(|(i, l)| {
            let mut words = l.split_whitespace();
            let b = words
                .next()
                .and_then(|w| u8::from_str_radix(w, 16).ok())
                .ok_or(format!("frames line {}: no controller bits", i + 2))?;
            let mut b = u16::from(b);
            let mut r = false;
            for w in words {
                match w {
                    "r" => r = true,
                    w => {
                        let p2 = u8::from_str_radix(w, 16)
                            .map_err(|_| format!("frames line {}: `{w}`", i + 2))?;
                        b |= u16::from(p2) << 8;
                    }
                }
            }
            Ok((b, r))
        })
        .collect()
}

/// The frames file's first line, as `gp_look::FRAMES_HEADER` has it.
pub fn gp_frames_header() -> &'static str {
    "gp-frames 1"
}

/// One hash in hex and a newline.
pub fn hash_line(out: &mut String, hash: &[u8; 32]) {
    for b in hash {
        let _ = write!(out, "{b:02x}");
    }
    out.push('\n');
}

/// The preview as the runtime shell standard's check drives it
/// (`gp_present::shell::check`, docs/shell.md): its own save record and
/// load (the page's slots run them through `gp_save_record` and
/// `gp_load_record`) and its state put back, as a rewind does. `gp gen
/// conform` runs the check on it beside the generated game.
pub struct ShellTarget<'a> {
    pub preview: &'a mut Preview,
    /// The profile the page runs the game in, which its saves name.
    pub profile: String,
}

impl gp_present::shell::Target for ShellTarget<'_> {
    fn step(&mut self, (pads, reset): gp_present::shell::Input) -> Result<(), String> {
        self.preview.step_pads(pads, reset)
    }

    fn hash(&self) -> [u8; 32] {
        self.preview.hash()
    }

    fn frame(&self) -> u64 {
        self.preview.interp.world().frame
    }

    fn snapshot(&self) -> gp_present::shell::Snapshot {
        gp_present::shell::Snapshot {
            profile: self.profile.clone(),
            state: self.preview.state(),
        }
    }

    fn put(&mut self, snap: &gp_present::shell::Snapshot) -> Result<(), String> {
        self.preview.load_state(&snap.state)
    }

    fn save_record(&mut self) -> Result<Vec<u8>, String> {
        Ok(self.preview.save_record(&self.profile))
    }

    fn load_record(&mut self, bytes: &[u8]) -> Result<(), String> {
        self.preview.load_record(bytes)
    }
}

pub mod rt;

#[cfg(target_arch = "wasm32")]
pub mod abi;

#[cfg(test)]
mod settings_tests {
    use super::*;
    use gp_present::settings::{Key, Outcome, Screen};

    /// Donkey Kong's own file: the screen lists the timing group first,
    /// every switch, and normal turns them all on.
    #[test]
    #[ignore = "needs the games repo beside this one (phase 1.3)"]
    fn the_preview_screen_reads_a_templates_profiles() {
        let file = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../../games/donkey-kong/profiles.toml");
        let text = std::fs::read_to_string(file).unwrap();
        let c = settings_catalog(&text).unwrap();
        assert_eq!(c.groups[0].title, "Timing");
        assert_eq!(c.groups.iter().map(|g| g.switches.len()).sum::<usize>(), 12);
        assert_eq!(c.modes()[..2], ["retro", "normal"]);
        let mut s = Screen::open(c, "retro");
        assert_eq!(s.key(settings_key(3).unwrap()), Outcome::Redraw);
        assert_eq!(s.on.len(), 12);
        assert_eq!(
            s.key(Key::Open),
            Outcome::Closed {
                profile: "normal".into(),
                restart: true,
                theme: None,
            }
        );
        assert_eq!(settings_key(7), None);
        // Drawn at the game's visible size (256 by 224 on the NES with its
        // overscan), every line in reach.
        let px = s.draw(256, 224);
        assert_eq!(px.len(), 256 * 224 * 4);
    }

    /// Every template's file makes a screen with retro and normal.
    #[test]
    fn every_templates_screen_has_retro_and_normal() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../templates");
        for e in std::fs::read_dir(root).unwrap() {
            let f = e.unwrap().path().join("profiles.toml");
            if !f.is_file() {
                continue;
            }
            let c = settings_catalog(&std::fs::read_to_string(&f).unwrap()).unwrap();
            assert_eq!(c.modes()[..2], ["retro", "normal"], "{}", f.display());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frames_parse_with_resets() {
        let f = parse_frames("gp-frames 1\n01\n80 r\n").unwrap();
        assert_eq!(f, vec![(1, false), (0x80, true)]);
        // Controller 2's bits after controller 1's, as the generator writes them.
        let f = parse_frames("gp-frames 1\n00 08\n01 40 r\n").unwrap();
        assert_eq!(f, vec![(0x0800, false), (0x4001, true)]);
        assert!(parse_frames("01\n").is_err());
        assert!(parse_frames("gp-frames 1\n01 x\n").is_err());
    }

    /// The page's save slots and rewind put a state back with
    /// `load_state` and step on: the hashes are a straight run's. Moss Hop
    /// (templates/mosshop, our own game) on a made-up flat level.
    /// Moss Hop's model with a made-up flat level, as a bundle.
    fn mosshop_bundle() -> Vec<u8> {
        mosshop_bundle_with(|t| t)
    }

    /// The same, with the model's text changed first.
    fn mosshop_bundle_with(edit: impl Fn(String) -> String) -> Vec<u8> {
        let text = edit(include_str!("../../../templates/mosshop/model.toml").to_owned());
        let model = gp_model::Model::parse(&text).unwrap();
        // Moss Hop's nine metatiles: solid 1 to 3, 6 and 7; kind 2 the ?
        // block and 5 the coin.
        let tiles = |t: u8| {
            format!(
                "[[metatile]]\ntiles = [{t}, {t}, {t}, {t}]\nsolid = {}\nkind = {}\n",
                u8::from((1..=3).contains(&t) || t == 6 || t == 7),
                [0, 0, 0, 0, 0, 0, 2, 0, 5][usize::from(t)]
            )
        };
        let rows: Vec<String> = (0..15)
            .map(|r| {
                format!(
                    "\"{}\",",
                    [".", "M", "S"][usize::from(r >= 12) + usize::from(r >= 13)].repeat(32)
                )
            })
            .collect();
        let files = [
            ("assets.toml", include_str!("../../../assets/mosshop.toml").to_owned()),
            ("metatiles.toml", format!("schema = 1\nasset = \"metatiles\"\n{}", (0..9).map(tiles).collect::<String>())),
            ("slugs.toml", "schema = 1\nasset = \"slugs\"\n[[row]]\nstart = 64\nend = 96\n[[row]]\nstart = 160\nend = 192\n".to_owned()),
            ("level.toml", format!("schema = 1\nasset = \"level\"\nrows = [\n{}\n]\n", rows.join("\n"))),
        ]
        .into_iter()
        .map(|(k, v)| (k.to_owned(), v))
        .collect();
        Bundle::new(&model, gp_interp::Lifted { files }).to_bytes()
    }

    /// Any set of Moss Hop's switches is applied to the bundle from the
    /// text of its profiles.toml, and runs with the set's name in the hash.
    #[test]
    fn a_bundle_in_any_set_of_switches() {
        let base = mosshop_bundle();
        let text = include_str!("../../../templates/mosshop/profiles.toml");
        assert_eq!(profile_bundle(&base, text, "retro").unwrap(), base);
        let one = profile_bundle(&base, text, "slow_slugs").unwrap();
        let both = profile_bundle(&base, text, "easy_stomp+slow_slugs").unwrap();
        assert_eq!(profile_bundle(&base, text, "assist").unwrap(), both);
        let m = Bundle::from_bytes(&both).unwrap().parse_model().unwrap();
        assert_eq!(m.model.profile.as_deref(), Some("easy_stomp+slow_slugs"));
        let hash = |b: &[u8]| Preview::load(b).unwrap().hash();
        assert_ne!(hash(&base), hash(&one));
        assert_ne!(hash(&one), hash(&both));
        assert!(profile_bundle(&base, text, "fast").is_err());
    }

    #[test]
    fn a_state_taken_and_put_back_steps_on_to_the_same_hashes() {
        let bundle = mosshop_bundle();
        let input = |f: u32| 0x80 | if f % 40 < 8 { 1 } else { 0 };
        let mut p = Preview::load(&bundle).unwrap();
        let mut want = Vec::new();
        for f in 0..120 {
            p.step(input(f), false).unwrap();
            want.push(p.hash());
        }
        let mut p = Preview::load(&bundle).unwrap();
        for f in 0..60 {
            p.step(input(f), false).unwrap();
        }
        let kept = p.state();
        for _ in 0..30 {
            p.step(0x41, false).unwrap();
        }
        p.load_state(&kept).unwrap();
        assert_eq!(p.hash(), want[59]);
        for f in 60..120 {
            p.step(input(f), false).unwrap();
            assert_eq!(p.hash(), want[f as usize], "frame {}", f + 1);
        }
    }

    /// A save record loads in the build that made it onto a straight
    /// run's hashes, and a build whose model differs in one field (the
    /// jump's gravity) refuses it with the player's sentence.
    #[test]
    fn a_save_record_loads_in_its_own_build_only() {
        let bundle = mosshop_bundle();
        let input = |f: u32| 0x80 | if f % 40 < 8 { 1 } else { 0 };
        let mut p = Preview::load(&bundle).unwrap();
        let mut want = Vec::new();
        for f in 0..120 {
            p.step(input(f), false).unwrap();
            want.push(p.hash());
        }
        let mut p = Preview::load(&bundle).unwrap();
        for f in 0..60 {
            p.step(input(f), false).unwrap();
        }
        let record = p.save_record("retro");
        let r = gp_sim::save::Record::read(&record).unwrap();
        assert_eq!((r.model.as_str(), r.profile.as_str()), ("mosshop", "retro"));
        assert_eq!(r.digest, p.spec_digest());
        for _ in 0..30 {
            p.step(0x41, false).unwrap();
        }
        p.load_record(&record).unwrap();
        assert_eq!(p.hash(), want[59]);
        for f in 60..120 {
            p.step(input(f), false).unwrap();
            assert_eq!(p.hash(), want[f as usize], "frame {}", f + 1);
        }
        let changed = mosshop_bundle_with(|t| {
            let c = t.replacen("gravity = \"0.25\"", "gravity = \"0.1875\"", 1);
            assert_ne!(c, t);
            c
        });
        let mut q = Preview::load(&changed).unwrap();
        assert_ne!(q.spec_digest(), p.spec_digest());
        let before = q.hash();
        assert_eq!(
            q.load_record(&record).unwrap_err(),
            gp_sim::save::OTHER_BUILD
        );
        assert_eq!(q.hash(), before, "a refused save changes nothing");
    }

    #[test]
    fn picture_options_widen_the_window_and_leave_the_hashes() {
        let bundle = mosshop_bundle();
        let input = |f: u32| 0x82 | if f % 40 < 8 { 1 } else { 0 };
        let mut plain = Preview::load(&bundle).unwrap();
        let mut wide = Preview::load(&bundle).unwrap();
        let present = gp_present::text::write(&gp_present::Present::default());
        for p in [&mut plain, &mut wide] {
            p.load_present(&present).unwrap();
        }
        wide.set_view("wide=16:9,side=center,limit=off,dim,mute=noise+dmc")
            .unwrap();
        for f in 0..120 {
            plain.step(input(f), false).unwrap();
            wide.step(input(f), false).unwrap();
            assert_eq!(plain.hash(), wide.hash(), "frame {}", f + 1);
        }
        let w: serde_json::Value = serde_json::from_str(&wide.scene().unwrap()).unwrap();
        let n: serde_json::Value = serde_json::from_str(&plain.scene().unwrap()).unwrap();
        assert_eq!(
            (w["window"]["w"].as_u64(), n["window"]["w"].as_u64()),
            (Some(426), Some(256))
        );
        for bad in ["wide=4:3", "side=left", "mute=bell", "sharp"] {
            assert!(wide.set_view(bad).is_err(), "{bad}");
        }
    }

    /// Moss Hop's model renamed, its slug naming a hook.
    fn hooked(name: &str, hook: &str) -> Vec<u8> {
        mosshop_bundle_with(|t| {
            let t = t
                .replace("name = \"mosshop\"", &format!("name = \"{name}\""))
                .replace(
                    "[entities.slug]\nrole = \"enemy\"",
                    &format!("[entities.slug]\next = \"{hook}\"\nrole = \"enemy\""),
                );
            assert!(t.contains(&format!("ext = \"{hook}\"")));
            t
        })
    }

    /// The preview runs the hooks `gp` registers for the model, with gp's
    /// label, and refuses a model naming a hook it has no crate for, as
    /// `gp interp run` does.
    #[test]
    fn registered_hooks_run_and_missing_ones_are_refused() {
        let p = Preview::load(&hooked("hooks_demo", "demo_turn")).unwrap();
        assert_eq!(p.hooks(), gp_exts::for_model("hooks_demo").unwrap().label);
        assert!(Preview::load(&mosshop_bundle()).unwrap().hooks().is_empty());
        let err = Preview::load(&hooked("hooks_missing", "nope"))
            .err()
            .expect("refused");
        assert!(
            err.contains("hooks_missing") && err.contains("nope"),
            "{err}"
        );
        // A hook runs: the demo turns the slug round every 30 frames, so
        // the hashes part from the model without it.
        let frames = format!("{}\n{}", gp_frames_header(), "00\n".repeat(40));
        let mut with = Preview::load(&hooked("hooks_demo", "demo_turn")).unwrap();
        let mut plain = Preview::load(&mosshop_bundle_with(|t| {
            t.replace("name = \"mosshop\"", "name = \"hooks_demo\"")
        }))
        .unwrap();
        assert_ne!(
            with.run_frames(&frames).unwrap(),
            plain.run_frames(&frames).unwrap()
        );
    }

    #[test]
    fn the_frames_header_is_the_generators() {
        assert_eq!(gp_frames_header(), gp_look::FRAMES_HEADER);
    }

    #[test]
    fn sound_plays_the_level_song_live() {
        // A made-up song: one pulse note held for 60 frames.
        let mut set = gp_sound::song::SongSet::new(gp_sound::apu::Region::Ntsc);
        set.instruments.push(gp_sound::song::Instrument {
            name: "lead".into(),
            voice: gp_sound::song::Voice::Pulse,
            role: Some("lead".into()),
            volume: vec![10],
            duty: vec![2],
            pitch: vec![],
            sample: None,
            notes: 1,
        });
        let tempo = gp_sound::song::Tempo {
            frames_per_row: 6,
            rows_per_pattern: 10,
        };
        let note = gp_sound::song::Placed {
            start: 0,
            note: gp_sound::song::Note {
                at: 0,
                len: 60,
                pitch: 69,
                fine: 0,
                instrument: 0,
            },
        };
        set.songs.push(gp_sound::song::Song {
            name: "tune".into(),
            kind: gp_sound::song::Kind::Music,
            origin: gp_sound::song::Origin::Edited,
            confidence: 1.0,
            driver: None,
            frames: 60,
            loop_frame: Some(0),
            tempo,
            tracks: vec![gp_sound::song::Track::from_placed(
                gp_sound::apu::Channel::Pulse1,
                &[note],
                tempo,
                60,
            )],
            request: None,
            event: None,
            level: None,
            state: None,
            captured: None,
            notes: vec![],
        });
        let cues = gp_present::sound::Cues {
            level: vec![(0, "music/tune".into())],
            ..Default::default()
        };
        let art = gp_look::sound::SoundArt {
            cues,
            files: vec![],
            sets: [("music".to_string(), set.to_text())].into(),
            ..Default::default()
        };
        let mut s = Sound::load(&gp_look::sound::web_bundle(&art)).unwrap();
        let w = gp_sim::World::default();
        let mut loud = 0;
        for _ in 0..10 {
            s.step(&w, &[], &[]);
            assert!((733..=735).contains(&s.samples.len()));
            loud += s.samples.iter().filter(|x| x.unsigned_abs() > 1000).count();
        }
        assert!(loud > 1000, "the song sounds");
    }

    /// The autosave policy after a tick: due on the tick that starts a
    /// level, and on no other. Walking right, Moss Hop's hero meets the
    /// first slug and the level starts again: frame 147 first.
    #[test]
    fn the_preview_autosaves_as_a_level_starts() {
        let mut p = Preview::load(&mosshop_bundle()).unwrap();
        let mut due = Vec::new();
        for _ in 0..600 {
            p.step_pads(0x80, false).unwrap();
            if p.autosave_due() {
                due.push(p.interp.world().frame);
            }
        }
        assert_eq!(due, [147, 295, 563]);
    }

    /// The preview passes the runtime shell standard's check: saves, loads
    /// onto the straight run's hashes, refuses another build's save, and
    /// its ring rewinds onto them too.
    #[test]
    fn the_preview_passes_the_shell_check() {
        let mut p = Preview::load(&mosshop_bundle()).unwrap();
        let mut t = ShellTarget {
            preview: &mut p,
            profile: "retro".into(),
        };
        let input = gp_present::shell::made_up_input(240);
        let mut store = gp_present::shell::Memory::default();
        let c = gp_present::shell::check(&mut t, &mut store, &input).unwrap();
        assert_eq!((c.frames, c.refused.len()), (240, 5));
        assert_eq!(c.resumed_at, c.rewound_to);
        // The Saves line reads the preview's own record: its frame, mode
        // and stamp, and another version's refused by the same sentence.
        let rec = p.save_record("retro");
        let line = p.slot_line(1, Some(&rec), Some(7), "retro");
        let s = line.save.unwrap();
        assert_eq!(
            (s.frame, s.profile.as_str(), s.saved_at, s.refused),
            (p.interp.world().frame, "retro", Some(7), None)
        );
        let mut r = gp_sim::save::Record::read(&rec).unwrap();
        r.digest = "0".repeat(64);
        let other = p.slot_line(2, Some(&r.to_bytes()), None, "retro");
        assert_eq!(other.save.unwrap().refused, Some(gp_sim::save::OTHER_BUILD));
        assert!(p.slot_line(3, None, None, "retro").save.is_none());
        // The page's ring: two frames a step, back to the newest before now.
        p.ring_reset(60.0);
        for _ in 0..10 {
            p.step_pads(0x80, false).unwrap();
            p.ring_note();
        }
        let at = p.state();
        assert!(p.ring_back().unwrap());
        assert_eq!(p.interp.world().frame, 248);
        p.ring_clear();
        assert!(!p.ring_back().unwrap());
        assert_ne!(p.state(), at);
    }
}
