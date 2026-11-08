//! [`Present`] as text, so the generator can hand it to any runtime as a
//! file (`assets/present.txt`) and a runtime reads it with no other crate.
//!
//! The text is the line `gp-present 1` and then whitespace-separated
//! tokens in a fixed order: numbers in decimal, booleans as `0` or `1`, an
//! absent value as `-` before a present one's `+`, a list as its length
//! then its items, and a string as `x` and its UTF-8 bytes in hex.

use crate::{
    Bump, CellLook, Clip, Cycle, Effect, Field, Frame, Glyph, HiddenLook, Hud, HudSprite,
    LevelLook, Look, Motion, Phase, Picture, Present, Screen, ScreenLook, ScreenSprite, Source,
    SplitLook, SpriteLimit, SpriteOrder, When,
};

/// The first line.
pub const HEADER: &str = "gp-present 23";

/// A comparison as the text writes it.
fn cmp_code(c: gp_sim::spec::Cmp) -> u8 {
    use gp_sim::spec::Cmp as C;
    match c {
        C::Lt => 0,
        C::Le => 1,
        C::Eq => 2,
        C::Ne => 3,
        C::Ge => 4,
        C::Gt => 5,
    }
}

fn cmp_of(n: u8) -> Result<gp_sim::spec::Cmp, String> {
    use gp_sim::spec::Cmp as C;
    Ok(match n {
        0 => C::Lt,
        1 => C::Le,
        2 => C::Eq,
        3 => C::Ne,
        4 => C::Ge,
        5 => C::Gt,
        n => return Err(format!("present text: `{n}` isn't a comparison")),
    })
}

struct W(String);

impl W {
    fn tok(&mut self, t: impl std::fmt::Display) {
        if !self.0.ends_with('\n') {
            self.0.push(' ');
        }
        self.0.push_str(&t.to_string());
    }
    fn nl(&mut self) {
        self.0.push('\n');
    }
    fn b(&mut self, v: bool) {
        self.tok(u8::from(v));
    }
    fn s(&mut self, v: &str) {
        let mut t = String::from("x");
        for b in v.bytes() {
            t.push_str(&format!("{b:02x}"));
        }
        self.tok(t);
    }
    fn opt<T>(&mut self, v: &Option<T>, f: impl FnOnce(&mut Self, &T)) {
        match v {
            None => self.tok("-"),
            Some(x) => {
                self.tok("+");
                f(self, x);
            }
        }
    }
    fn list<T>(&mut self, v: &[T], mut f: impl FnMut(&mut Self, &T)) {
        self.tok(v.len());
        for x in v {
            f(self, x);
        }
    }
    fn frame(&mut self, f: &Frame) {
        self.tok(f.id);
        self.b(f.flip_x);
        self.b(f.flip_y);
    }
    fn motion(&mut self, m: &Motion) {
        self.tok(m.speed);
        self.tok(m.gravity);
        self.opt(&m.max_fall, |w, v| w.tok(v));
    }
    fn obool(&mut self, v: &Option<bool>) {
        self.opt(v, |w, x| w.b(*x));
    }
    fn when(&mut self, wh: &When) {
        self.obool(&wh.ground);
        self.obool(&wh.moving);
        self.obool(&wh.skidding);
        self.obool(&wh.rising);
        self.opt(&wh.speed_at_least, |w, v| w.tok(v));
        self.obool(&wh.jumped);
        self.obool(&wh.invulnerable);
        self.obool(&wh.climbing);
        self.opt(&wh.word, |w, (i, a, b)| {
            w.tok(i);
            w.opt(a, |w, v| w.tok(v));
            w.opt(b, |w, v| w.tok(v));
        });
        self.list(&wh.words, |w, (i, a, b)| {
            w.tok(i);
            w.opt(a, |w, v| w.tok(v));
            w.opt(b, |w, v| w.tok(v));
        });
        self.list(&wh.counters, |w, (k, a, b)| {
            w.tok(k);
            w.opt(a, |w, v| w.tok(v));
            w.opt(b, |w, v| w.tok(v));
        });
        self.list(&wh.slot_counters, |w, (ks, a, b)| {
            w.list(ks, |w, k| w.tok(k));
            w.opt(a, |w, v| w.tok(v));
            w.opt(b, |w, v| w.tok(v));
        });
        self.opt(&wh.x.0, |w, v| w.tok(v));
        self.opt(&wh.x.1, |w, v| w.tok(v));
    }
}

/// Writes a presentation as text.
pub fn write(p: &Present) -> String {
    let mut w = W(String::new());
    w.0.push_str(HEADER);
    w.nl();
    let s = &p.screen;
    for v in s.size.iter().chain(&s.aspect).chain(&s.overscan) {
        w.tok(v);
    }
    w.opt(&s.sprite_limit, |w, l| {
        w.tok(l.per_line);
        w.tok(l.width);
        w.b(l.flicker);
    });
    w.b(s.x_wraps);
    w.nl();
    w.list(&p.pictures, |w, pic| {
        w.s(&pic.file);
        w.tok(pic.size[0]);
        w.tok(pic.size[1]);
        w.nl();
    });
    w.list(&p.frames, |w, f| {
        w.list(f, |w, v| w.tok(v));
        w.nl();
    });
    w.list(&p.origins, |w, o| {
        w.opt(o, |w, [x, y]| {
            w.tok(x);
            w.tok(y);
        });
    });
    w.nl();
    w.list(&p.kinds, |w, states| {
        w.list(states, |w, look| {
            w.opt(look, |w, l| {
                w.frame(&l.base);
                w.b(l.facing_flip);
                w.opt(&l.copy, |w, c| {
                    for n in c {
                        w.tok(n);
                    }
                });
                w.tok(l.mirror);
                w.list(&l.clips, |w, c| {
                    w.when(&c.when);
                    w.list(&c.frames, |w, f| w.frame(f));
                    w.list(&c.timing, |w, (at, every)| {
                        w.opt(at, |w, v| w.tok(v));
                        w.tok(every);
                    });
                    w.b(c.game_clock);
                    w.b(c.body_clock);
                    w.b(c.while_frozen);
                    w.b(c.hold);
                    w.b(c.speed_before);
                    w.list(&c.lengths, |w, v| w.tok(v));
                });
                w.opt(&l.blink, |w, (when, every)| {
                    w.when(when);
                    w.tok(every);
                });
                w.b(l.behind);
                w.opt(&l.behind_when, |w, c| w.when(c));
                w.opt(&l.split, |w, sp| {
                    for v in sp.keep {
                        w.tok(v);
                    }
                    w.opt(&sp.x, |w, v| w.tok(v));
                    w.opt(&sp.y, |w, v| w.tok(v));
                });
                w.opt(&l.replaced, |w, f| w.b(*f));
                w.opt(&l.leaves, |w, f| w.tok(f));
                w.b(l.front);
                w.b(l.name_table);
                w.b(l.in_place);
            });
        });
        w.nl();
    });
    w.list(&p.levels, |w, l| {
        w.opt(&l.picture, |w, v| w.tok(v));
        for c in l.clear {
            w.tok(c);
        }
        w.tok(l.set);
        w.list(&l.cycles, |w, c| {
            w.opt(&c.mask, |w, v| w.tok(v));
            w.opt(&c.hud_mask, |w, v| w.tok(v));
            w.list(&c.colors, |w, rgb| {
                for v in rgb {
                    w.tok(v);
                }
            });
            w.tok(c.every);
            w.opt(&c.counter, |w, (k, shift)| {
                w.tok(k);
                w.tok(shift);
            });
        });
        w.s(&l.label);
        w.opt(&l.hud_layer, |w, v| w.tok(v));
        w.opt(&l.cells, |w, c| {
            w.list(&c.kinds, |w, (t, bg, sp)| {
                w.tok(t);
                w.tok(bg);
                w.tok(sp);
            });
            w.opt(&c.held, |w, v| w.tok(v));
            w.list(&c.own, |w, (cell, pic)| {
                w.tok(cell);
                w.tok(pic);
            });
        });
        w.list(&l.order, |w, o| {
            w.opt(o, |w, o| {
                w.tok(o.at);
                w.tok(o.step);
                w.opt(&o.word, |w, v| w.tok(v));
                w.opt(&o.turn, |w, (c, n)| {
                    w.tok(c);
                    w.tok(n);
                });
            });
        });
        w.list(&l.effect_order, |w, o| {
            w.opt(o, |w, (at, step)| {
                w.tok(at);
                w.tok(step);
            });
        });
        w.nl();
    });
    w.opt(&p.hud, |w, h| {
        w.list(&h.fields, |w, f| {
            w.tok(f.at[0]);
            w.tok(f.at[1]);
            match &f.source {
                Source::Counter(i) => {
                    w.tok("c");
                    w.tok(i);
                }
                Source::Level => w.tok("l"),
                Source::Text(t) => {
                    w.tok("t");
                    w.s(t);
                }
            }
            w.tok(f.digits);
            w.tok(f.divide);
            w.s(&f.pad.to_string());
            w.tok(f.palette);
            w.list(&f.when, |w, g| {
                w.tok(g.counter);
                w.tok(cmp_code(g.cmp));
                w.tok(g.value);
            });
        });
        w.nl();
        w.list(&h.glyphs, |w, g| {
            w.tok(g.set);
            w.tok(g.palette);
            w.s(&g.ch.to_string());
            w.tok(g.pic);
        });
        w.nl();
        w.list(&h.sprites, |w, sp| {
            w.tok(sp.at[0]);
            w.tok(sp.at[1]);
            w.list(&sp.pics, |w, v| w.tok(v));
        });
        w.nl();
        w.list(&h.sets, |w, v| w.tok(v));
    });
    w.nl();
    w.opt(&p.bump, |w, b| {
        w.motion(&b.motion);
        w.tok(b.offset[0]);
        w.tok(b.offset[1]);
    });
    w.list(&p.effects, |w, e| {
        w.tok(e.event);
        w.b(e.at_hit);
        w.tok(e.offset[0]);
        w.tok(e.offset[1]);
        w.list(&e.phases, |w, ph| {
            w.list(&ph.frames, |w, f| w.frame(f));
            w.tok(ph.every);
            w.opt(&ph.motion, |w, m| w.motion(m));
            w.tok(ph.rise_every);
            w.b(ph.with_camera);
            w.opt(&ph.until_speed, |w, v| w.tok(v));
            w.opt(&ph.length, |w, v| w.tok(v));
            w.b(ph.while_frozen);
            w.opt(&ph.until_counter, |w, v| w.tok(v));
            w.list(&ph.until_counters, |w, v| w.tok(v));
            w.b(ph.until_gone);
        });
        w.nl();
    });
    w.nl();
    w.list(&p.screens, |w, sc| {
        w.list(&sc.states, |w, (m, s)| {
            w.tok(m);
            w.tok(s);
        });
        w.list(&sc.counters, |w, (k, a, b)| {
            w.tok(k);
            w.opt(a, |w, v| w.tok(v));
            w.opt(b, |w, v| w.tok(v));
        });
        w.tok(sc.picture);
        w.list(&sc.clear, |w, v| w.tok(v));
        w.list(&sc.sprites, |w, sp| {
            w.tok(sp.pic);
            w.tok(sp.at[0]);
            w.tok(sp.at[1]);
            w.opt(&sp.counter, |w, v| w.tok(v));
            w.tok(sp.step[0]);
            w.tok(sp.step[1]);
        });
        w.opt(&sc.from_line, |w, v| w.tok(v));
        w.nl();
    });
    w.list(&p.sprites_off, |w, (m, s)| {
        w.tok(m);
        w.tok(s);
    });
    w.nl();
    w.list(&p.hidden, |w, h| {
        w.list(&h.states, |w, (m, s)| {
            w.tok(m);
            w.tok(s);
        });
        w.list(&h.counters, |w, (k, a, b)| {
            w.tok(k);
            w.opt(a, |w, v| w.tok(v));
            w.opt(b, |w, v| w.tok(v));
        });
        w.list(&h.kinds, |w, k| w.tok(k));
        w.b(h.hud);
    });
    w.nl();
    w.0
}

struct R<'a> {
    toks: std::str::SplitWhitespace<'a>,
}

impl R<'_> {
    fn tok(&mut self) -> Result<&str, String> {
        self.toks
            .next()
            .ok_or_else(|| "present text ends early".to_string())
    }
    fn n<T: std::str::FromStr>(&mut self) -> Result<T, String> {
        let t = self.tok()?;
        t.parse()
            .map_err(|_| format!("present text: `{t}` isn't a number"))
    }
    fn b(&mut self) -> Result<bool, String> {
        match self.tok()? {
            "0" => Ok(false),
            "1" => Ok(true),
            t => Err(format!("present text: `{t}` isn't 0 or 1")),
        }
    }
    fn s(&mut self) -> Result<String, String> {
        let t = self.tok()?;
        let hex = t
            .strip_prefix('x')
            .ok_or_else(|| format!("present text: `{t}` isn't a string"))?;
        if !hex.len().is_multiple_of(2) {
            return Err(format!("present text: `{t}` has an odd length"));
        }
        let bytes = (0..hex.len())
            .step_by(2)
            .map(|i| u8::from_str_radix(&hex[i..i + 2], 16))
            .collect::<Result<Vec<u8>, _>>()
            .map_err(|_| format!("present text: `{t}` isn't hex"))?;
        String::from_utf8(bytes).map_err(|_| format!("present text: `{t}` isn't UTF-8"))
    }
    fn ch(&mut self) -> Result<char, String> {
        let s = self.s()?;
        let mut c = s.chars();
        match (c.next(), c.next()) {
            (Some(ch), None) => Ok(ch),
            _ => Err(format!("present text: `{s}` isn't one character")),
        }
    }
    fn opt<T>(
        &mut self,
        f: impl FnOnce(&mut Self) -> Result<T, String>,
    ) -> Result<Option<T>, String> {
        match self.tok()? {
            "-" => Ok(None),
            "+" => f(self).map(Some),
            t => Err(format!("present text: `{t}` isn't - or +")),
        }
    }
    fn list<T>(
        &mut self,
        mut f: impl FnMut(&mut Self) -> Result<T, String>,
    ) -> Result<Vec<T>, String> {
        let n: usize = self.n()?;
        if n > 1 << 24 {
            return Err(format!("present text: a list of {n}"));
        }
        (0..n).map(|_| f(self)).collect()
    }
    fn frame(&mut self) -> Result<Frame, String> {
        Ok(Frame {
            id: self.n()?,
            flip_x: self.b()?,
            flip_y: self.b()?,
        })
    }
    fn motion(&mut self) -> Result<Motion, String> {
        Ok(Motion {
            speed: self.n()?,
            gravity: self.n()?,
            max_fall: self.opt(|r| r.n())?,
        })
    }
    fn obool(&mut self) -> Result<Option<bool>, String> {
        self.opt(|r| r.b())
    }
    fn when(&mut self) -> Result<When, String> {
        Ok(When {
            ground: self.obool()?,
            moving: self.obool()?,
            skidding: self.obool()?,
            rising: self.obool()?,
            speed_at_least: self.opt(|r| r.n())?,
            jumped: self.obool()?,
            invulnerable: self.obool()?,
            climbing: self.obool()?,
            word: self.opt(|r| Ok((r.n()?, r.opt(|r| r.n())?, r.opt(|r| r.n())?)))?,
            words: self.list(|r| Ok((r.n()?, r.opt(|r| r.n())?, r.opt(|r| r.n())?)))?,
            counters: self.list(|r| Ok((r.n()?, r.opt(|r| r.n())?, r.opt(|r| r.n())?)))?,
            slot_counters: self
                .list(|r| Ok((r.list(|r| r.n())?, r.opt(|r| r.n())?, r.opt(|r| r.n())?)))?,
            x: (self.opt(|r| r.n())?, self.opt(|r| r.n())?),
        })
    }
}

/// Reads what [`write`] wrote.
pub fn read(text: &str) -> Result<Present, String> {
    let rest = text
        .strip_prefix(HEADER)
        .ok_or_else(|| format!("present text doesn't start with `{HEADER}`"))?;
    let mut r = R {
        toks: rest.split_whitespace(),
    };
    let mut nums = [0u32; 8];
    for v in &mut nums {
        *v = r.n()?;
    }
    let screen = Screen {
        size: [nums[0], nums[1]],
        aspect: [nums[2], nums[3]],
        overscan: [nums[4], nums[5], nums[6], nums[7]],
        sprite_limit: r.opt(|r| {
            Ok(SpriteLimit {
                per_line: r.n()?,
                width: r.n()?,
                flicker: r.b()?,
            })
        })?,
        x_wraps: r.b()?,
    };
    let pictures = r.list(|r| {
        Ok(Picture {
            file: r.s()?,
            size: [r.n()?, r.n()?],
        })
    })?;
    let frames = r.list(|r| r.list(|r| r.n()))?;
    let origins = r.list(|r| r.opt(|r| Ok([r.n()?, r.n()?])))?;
    let kinds = r.list(|r| {
        r.list(|r| {
            r.opt(|r| {
                Ok(Look {
                    base: r.frame()?,
                    facing_flip: r.b()?,
                    copy: r.opt(|r| Ok([r.n()?, r.n()?, r.n()?]))?,
                    mirror: r.n()?,
                    clips: r.list(|r| {
                        Ok(Clip {
                            when: r.when()?,
                            frames: r.list(|r| r.frame())?,
                            timing: r.list(|r| Ok((r.opt(|r| r.n())?, r.n()?)))?,
                            game_clock: r.b()?,
                            body_clock: r.b()?,
                            while_frozen: r.b()?,
                            hold: r.b()?,
                            speed_before: r.b()?,
                            lengths: r.list(|r| r.n())?,
                        })
                    })?,
                    blink: r.opt(|r| Ok((r.when()?, r.n()?)))?,
                    behind: r.b()?,
                    behind_when: r.opt(|r| r.when())?,
                    split: r.opt(|r| {
                        Ok(SplitLook {
                            keep: [r.n()?, r.n()?, r.n()?, r.n()?],
                            x: r.opt(|r| r.n())?,
                            y: r.opt(|r| r.n())?,
                        })
                    })?,
                    replaced: r.opt(|r| r.b())?,
                    leaves: r.opt(|r| r.n())?,
                    front: r.b()?,
                    name_table: r.b()?,
                    in_place: r.b()?,
                })
            })
        })
    })?;
    let levels = r.list(|r| {
        Ok(LevelLook {
            picture: r.opt(|r| r.n())?,
            clear: [r.n()?, r.n()?, r.n()?],
            set: r.n()?,
            cycles: r.list(|r| {
                Ok(Cycle {
                    mask: r.opt(|r| r.n())?,
                    hud_mask: r.opt(|r| r.n())?,
                    colors: r.list(|r| Ok([r.n()?, r.n()?, r.n()?]))?,
                    every: r.n()?,
                    counter: r.opt(|r| Ok((r.n()?, r.n()?)))?,
                })
            })?,
            label: r.s()?,
            hud_layer: r.opt(|r| r.n())?,
            cells: r.opt(|r| {
                Ok(CellLook {
                    kinds: r.list(|r| Ok((r.n()?, r.n()?, r.n()?)))?,
                    held: r.opt(|r| r.n())?,
                    own: r.list(|r| Ok((r.n()?, r.n()?)))?,
                })
            })?,
            order: r.list(|r| {
                r.opt(|r| {
                    Ok(SpriteOrder {
                        at: r.n()?,
                        step: r.n()?,
                        word: r.opt(|r| r.n())?,
                        turn: r.opt(|r| Ok((r.n()?, r.n()?)))?,
                    })
                })
            })?,
            effect_order: r.list(|r| r.opt(|r| Ok((r.n()?, r.n()?))))?,
        })
    })?;
    let hud = r.opt(|r| {
        Ok(Hud {
            fields: r.list(|r| {
                let at = [r.n()?, r.n()?];
                let source = match r.tok()? {
                    "c" => Source::Counter(r.n()?),
                    "l" => Source::Level,
                    "t" => Source::Text(r.s()?),
                    t => return Err(format!("present text: `{t}` isn't a field source")),
                };
                Ok(Field {
                    at,
                    source,
                    digits: r.n()?,
                    divide: r.n()?,
                    pad: r.ch()?,
                    palette: r.n()?,
                    when: r.list(|r| {
                        Ok(gp_sim::spec::Guard {
                            counter: r.n()?,
                            cmp: cmp_of(r.n()?)?,
                            value: r.n()?,
                        })
                    })?,
                })
            })?,
            glyphs: r.list(|r| {
                Ok(Glyph {
                    set: r.n()?,
                    palette: r.n()?,
                    ch: r.ch()?,
                    pic: r.n()?,
                })
            })?,
            sprites: r.list(|r| {
                Ok(HudSprite {
                    at: [r.n()?, r.n()?],
                    pics: r.list(|r| r.n())?,
                })
            })?,
            sets: r.list(|r| r.n())?,
        })
    })?;
    let bump = r.opt(|r| {
        Ok(Bump {
            motion: r.motion()?,
            offset: [r.n()?, r.n()?],
        })
    })?;
    let effects = r.list(|r| {
        Ok(Effect {
            event: r.n()?,
            at_hit: r.b()?,
            offset: [r.n()?, r.n()?],
            phases: r.list(|r| {
                Ok(Phase {
                    frames: r.list(|r| r.frame())?,
                    every: r.n()?,
                    motion: r.opt(|r| r.motion())?,
                    rise_every: r.n()?,
                    with_camera: r.b()?,
                    until_speed: r.opt(|r| r.n())?,
                    length: r.opt(|r| r.n())?,
                    while_frozen: r.b()?,
                    until_counter: r.opt(|r| r.n())?,
                    until_counters: r.list(|r| r.n())?,
                    until_gone: r.b()?,
                })
            })?,
        })
    })?;
    let screens = r.list(|r| {
        Ok(ScreenLook {
            states: r.list(|r| Ok((r.n()?, r.n()?)))?,
            counters: r.list(|r| Ok((r.n()?, r.opt(|r| r.n())?, r.opt(|r| r.n())?)))?,
            picture: r.n()?,
            clear: {
                let c: Vec<u8> = r.list(|r| r.n())?;
                [
                    c.first().copied().unwrap_or(0),
                    c.get(1).copied().unwrap_or(0),
                    c.get(2).copied().unwrap_or(0),
                ]
            },
            sprites: r.list(|r| {
                Ok(ScreenSprite {
                    pic: r.n()?,
                    at: [r.n()?, r.n()?],
                    counter: r.opt(|r| r.n())?,
                    step: [r.n()?, r.n()?],
                })
            })?,
            from_line: r.opt(|r| r.n())?,
        })
    })?;
    let sprites_off = r.list(|r| Ok((r.n()?, r.n()?)))?;
    let hidden = r.list(|r| {
        Ok(HiddenLook {
            states: r.list(|r| Ok((r.n()?, r.n()?)))?,
            counters: r.list(|r| Ok((r.n()?, r.opt(|r| r.n())?, r.opt(|r| r.n())?)))?,
            kinds: r.list(|r| r.n())?,
            hud: r.b()?,
        })
    })?;
    if let Some(t) = r.toks.next() {
        return Err(format!("present text: `{t}` after the end"));
    }
    Ok(Present {
        screen,
        pictures,
        frames,
        origins,
        kinds,
        levels,
        hud,
        bump,
        effects,
        screens,
        sprites_off,
        hidden,
    })
}
