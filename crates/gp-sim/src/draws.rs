//! A random draws script: the original's random draws, in order, that a
//! rebuild takes in place of its own generator's, so a lockstep clip makes
//! the original's random choices (docs/benchmarks.md, "Random choices").
//!
//! A hook draws a byte with [`crate::ExtCx::draw`], naming its draw site.
//! Without a script the byte is the low byte of the world's generator's
//! next number, so free play is as before. With one, the byte is the
//! script's next draw when the rebuild draws at the same site on the same
//! frame; anything else is a desync, kept with where it happened, after
//! which the generator draws again. A desync is a defect in the rebuild:
//! it draws at another point, or another number of times, than the
//! original.
//!
//! The script is not world state: it isn't in the canonical state or its
//! hash, and a run without one is unchanged.

/// One of the original's draws.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Draw {
    /// The original's frame the draw is in, counted as the script's
    /// `base` is.
    pub frame: u64,
    /// The draw site's name, as the state mapping's `[[draws]]` and the
    /// hook name it.
    pub site: String,
    pub value: u8,
}

/// The original's draws for a run, and how far the rebuild has taken them.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct DrawScript {
    pub draws: Vec<Draw>,
    /// The original's frame at which the rebuild's world is `origin`: the
    /// frame a lockstep starts from.
    pub base: u64,
    /// The rebuild's frame count when the script started, set by the
    /// runtime when it loads the start. Until then the script isn't used:
    /// draws come from the generator and none is checked.
    pub origin: Option<u64>,
    /// The first draw not yet taken: every one before it is.
    pub next: usize,
    /// The draws taken past `next`, by index: within a frame the rebuild
    /// may take one site's draws before another's.
    pub taken: Vec<bool>,
    /// The first desync, in words, with the original's frame it was on.
    pub desync: Option<(u64, String)>,
    /// Soft sites (a state mapping's `soft` draw sites): read for what a
    /// listener hears or sees, never for what happens, so a rebuild that
    /// draws one on another frame, or not at all, isn't out of step. Its
    /// draws are taken when the rebuild asks on the same frame and passed
    /// over otherwise; they never desync the script.
    pub soft: Vec<String>,
    /// Every desync of the run, with the draw site it was at: a resync
    /// forgets [`Self::desync`] but not these.
    pub desyncs: Vec<(u64, String, String)>,
    /// When kept, each draw the rebuild makes, as its frame and site, in
    /// order, whether or not the script holds one for it: what the
    /// behaviour check (`gp interp odds`) reads of a free run.
    pub calls: Option<Vec<(u64, String)>>,
    /// A forcing script: a draw it doesn't hold falls back to the hook's
    /// own, with no desync, so one draw of a frame can be set alone.
    pub lenient: bool,
}

/// The text form's first line.
pub const DRAWS_HEADER: &str = "gp-draws 0";

impl DrawScript {
    /// The original's frame for the rebuild's frame `frame`.
    pub fn original_frame(&self, frame: u64) -> u64 {
        (frame + self.base).saturating_sub(self.origin.unwrap_or(0))
    }

    /// Starts taking draws: the rebuild's frame `origin` is the original's
    /// `base`. The draws on or before `base` count as taken, so a script
    /// of a whole run serves a start at any of its frames.
    pub fn start(&mut self, origin: u64) {
        self.origin = Some(origin);
        self.next = self
            .draws
            .iter()
            .position(|d| d.frame > self.base)
            .unwrap_or(self.draws.len());
        self.taken = (0..self.draws.len()).map(|i| i < self.next).collect();
        self.desync = None;
    }

    /// A state loaded again in the same run, as a resync loads the
    /// original's: the first load starts the script ([`Self::start`]); a
    /// later one keeps the origin, so the frames still line up, forgets a
    /// desync, and goes on from the original's first draw after the
    /// frame loaded, as the original goes on from there.
    pub fn resume(&mut self, frame: u64) {
        if self.origin.is_none() {
            self.start(frame);
            return;
        }
        let at = self.original_frame(frame);
        self.next = self
            .draws
            .iter()
            .position(|d| d.frame > at)
            .unwrap_or(self.draws.len());
        self.taken = (0..self.draws.len()).map(|i| i < self.next).collect();
        self.desync = None;
    }

    /// The draw for site `site` on the rebuild's frame `frame`, or `None`
    /// before the start or once the run has desynced (the caller draws
    /// from its generator).
    pub fn take(&mut self, frame: u64, site: &str) -> Option<u8> {
        if let Some(c) = &mut self.calls {
            c.push((frame, site.to_owned()));
        }
        if self.desync.is_some() {
            return None;
        }
        self.origin?;
        let at = self.original_frame(frame);
        if self.is_soft(site) {
            let i = self.find(at, site)?;
            self.taken[i] = true;
            self.pass_taken(at.saturating_sub(1));
            return Some(self.draws[i].value);
        }
        // A draw the original made on an earlier frame and the rebuild
        // didn't.
        self.check_through(frame.saturating_sub(1));
        if self.desync.is_some() {
            return None;
        }
        if self.taken.len() != self.draws.len() {
            self.taken = vec![false; self.draws.len()];
        }
        // The frame's next untaken draw at this site: a frame's sites may
        // be drawn in any order (a game reads one random byte each frame,
        // whichever object reads it first), but one site's in the
        // original's order.
        let found = (self.next..self.draws.len())
            .take_while(|&i| self.draws[i].frame <= at)
            .find(|&i| !self.taken[i] && self.draws[i].frame == at && self.draws[i].site == site);
        match found {
            Some(i) => {
                self.taken[i] = true;
                while self.next < self.draws.len() && self.taken[self.next] {
                    self.next += 1;
                }
                Some(self.draws[i].value)
            }
            None if self.lenient => None,
            None => {
                let why = match (self.next..self.draws.len()).find(|&i| !self.taken[i]) {
                    Some(i) => format!(
                        "draw {}: the rebuild drew `{site}` on frame {at}; the original's next draw is `{}` on frame {}",
                        i + 1,
                        self.draws[i].site,
                        self.draws[i].frame
                    ),
                    None => format!(
                        "the rebuild drew `{site}` on frame {at}, after the original's last draw ({} in all)",
                        self.draws.len()
                    ),
                };
                self.desyncs.push((at, site.to_owned(), why.clone()));
                self.desync = Some((at, why));
                None
            }
        }
    }

    /// Notes a desync for a draw of the original's the rebuild hasn't taken
    /// by the end of its frame `frame`.
    pub fn check_through(&mut self, frame: u64) {
        if self.desync.is_some() || self.origin.is_none() {
            return;
        }
        let at = self.original_frame(frame);
        self.pass_taken(at);
        if let Some(d) = self.draws.get(self.next)
            && d.frame <= at
            && !self.taken.get(self.next).copied().unwrap_or(false)
        {
            let why = format!(
                "draw {}: the original drew `{}` on frame {}; the rebuild didn't",
                self.next + 1,
                d.site,
                d.frame
            );
            self.desyncs.push((d.frame, d.site.clone(), why.clone()));
            self.desync = Some((d.frame, why));
        }
    }

    fn is_soft(&self, site: &str) -> bool {
        self.soft.iter().any(|s| s == site)
    }

    /// The untaken draw at `site` on the original's frame `at`.
    fn find(&mut self, at: u64, site: &str) -> Option<usize> {
        if self.taken.len() != self.draws.len() {
            self.taken = vec![false; self.draws.len()];
        }
        (self.next..self.draws.len())
            .take_while(|&i| self.draws[i].frame <= at)
            .find(|&i| !self.taken[i] && self.draws[i].frame == at && self.draws[i].site == site)
    }

    /// Moves `next` past the draws taken and the soft ones on or before
    /// the original's frame `at`, which no rebuild must take.
    fn pass_taken(&mut self, at: u64) {
        if self.taken.len() != self.draws.len() {
            self.taken = vec![false; self.draws.len()];
        }
        while let Some(d) = self.draws.get(self.next) {
            let soft_past = d.frame <= at && self.soft.contains(&d.site);
            if !(self.taken[self.next] || soft_past) {
                break;
            }
            self.taken[self.next] = true;
            self.next += 1;
        }
    }

    /// The draw at soft site `site` that the rebuild's frame `frame` would
    /// take, without taking it: what a runtime that plays the next frame's
    /// sound ahead of it hears (`gp_sim::Ext::apu_peek`). `None` for a
    /// site that isn't soft, before the start, after a desync or with no
    /// such draw.
    pub fn peek(&self, frame: u64, site: &str) -> Option<u8> {
        if self.desync.is_some() || self.origin.is_none() || !self.is_soft(site) {
            return None;
        }
        let at = self.original_frame(frame);
        (self.next..self.draws.len())
            .take_while(|&i| self.draws[i].frame <= at)
            .find(|&i| {
                !self.taken.get(i).copied().unwrap_or(false)
                    && self.draws[i].frame == at
                    && self.draws[i].site == site
            })
            .map(|i| self.draws[i].value)
    }

    /// Reads the text form: [`DRAWS_HEADER`], then one draw a line, `frame
    /// site value`, the value in decimal, and a `soft SITE` line for each
    /// soft site. `#` starts a comment.
    pub fn parse(text: &str) -> Result<Self, String> {
        let mut lines = text.lines();
        if lines.next().map(str::trim) != Some(DRAWS_HEADER) {
            return Err(format!("a draws script starts with `{DRAWS_HEADER}`"));
        }
        let mut draws = Vec::new();
        let mut soft = Vec::new();
        for (n, line) in lines.enumerate() {
            let line = line.split('#').next().unwrap_or("").trim();
            if line.is_empty() {
                continue;
            }
            if let Some(site) = line.strip_prefix("soft ") {
                soft.push(site.trim().to_owned());
                continue;
            }
            let bad = || format!("line {}: `{line}` isn't `frame site value`", n + 2);
            let mut it = line.split_whitespace();
            let (Some(f), Some(s), Some(v), None) = (it.next(), it.next(), it.next(), it.next())
            else {
                return Err(bad());
            };
            draws.push(Draw {
                frame: f.parse().map_err(|_| bad())?,
                site: s.to_owned(),
                value: v.parse().map_err(|_| bad())?,
            });
        }
        Ok(Self {
            draws,
            soft,
            ..Self::default()
        })
    }

    /// The text form [`Self::parse`] reads.
    pub fn text(&self) -> String {
        let mut out = format!("{DRAWS_HEADER}\n");
        for s in &self.soft {
            out.push_str(&format!("soft {s}\n"));
        }
        for d in &self.draws {
            out.push_str(&format!("{} {} {}\n", d.frame, d.site, d.value));
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn script() -> DrawScript {
        let mut s =
            DrawScript::parse("gp-draws 0\n# two draws\n105 ladder 136\n110 ladder 0\n").unwrap();
        s.base = 100;
        s.start(1000);
        s
    }

    /// A kept call list records every draw, before the start too, and a
    /// lenient script sets one draw and lets the others fall back.
    #[test]
    fn calls_are_kept_and_a_lenient_script_sets_one_draw() {
        let mut s = DrawScript {
            calls: Some(Vec::new()),
            ..DrawScript::default()
        };
        assert_eq!(s.take(3, "ladder"), None);
        assert_eq!(s.calls.as_deref(), Some(&[(3, "ladder".to_owned())][..]));
        let mut f = DrawScript {
            draws: vec![Draw {
                frame: 7,
                site: "ladder".into(),
                value: 9,
            }],
            origin: Some(0),
            lenient: true,
            ..DrawScript::default()
        };
        assert_eq!(f.take(7, "fire_way"), None);
        assert_eq!(f.take(7, "ladder"), Some(9));
        assert_eq!(f.take(7, "ladder"), None);
        assert!(f.desync.is_none() && f.desyncs.is_empty());
        // A strict script keeps every desync, by site.
        let mut s = script();
        assert_eq!(s.take(1002, "ladder"), None);
        s.resume(1007);
        assert_eq!(s.take(1008, "roll_off"), None);
        let sites: Vec<&str> = s.desyncs.iter().map(|d| d.1.as_str()).collect();
        assert_eq!(sites, ["ladder", "roll_off"]);
    }

    #[test]
    fn a_soft_site_is_taken_on_its_frame_and_never_desyncs() {
        let mut s = DrawScript::parse(
            "gp-draws 0\nsoft buzz\n103 buzz 8\n104 buzz 0\n105 ladder 136\n106 buzz 8\n",
        )
        .unwrap();
        assert_eq!(s.text().lines().nth(1), Some("soft buzz"));
        s.base = 100;
        s.start(1000);
        // Frame 1003's buzz isn't drawn; 1004's is peeked, then taken.
        assert_eq!(s.peek(1004, "buzz"), Some(0));
        assert_eq!(s.take(1004, "buzz"), Some(0));
        assert_eq!(s.take(1005, "ladder"), Some(136));
        // No buzz on 1005, none drawn on 1006: no desync.
        assert_eq!(s.take(1005, "buzz"), None);
        s.check_through(1010);
        assert_eq!(s.desync, None);
        assert_eq!(s.next, s.draws.len());
        // A hard site is never peeked.
        assert_eq!(s.peek(1005, "ladder"), None);
    }

    #[test]
    fn before_the_start_nothing_is_taken_or_checked() {
        let mut s = script();
        s.origin = None;
        assert_eq!(s.take(5, "ladder"), None);
        s.check_through(5000);
        assert_eq!((s.next, s.desync.clone()), (0, None));
    }

    #[test]
    fn a_resync_goes_on_from_the_draws_after_the_frame_it_loads() {
        let mut s = script();
        // The rebuild drew where the original didn't: a desync.
        assert_eq!(s.take(1002, "ladder"), None);
        assert!(s.desync.is_some());
        // A resync at the rebuild's frame 1007 (the original's 107) skips
        // the draw on 105 and takes the one on 110 at 1010.
        s.resume(1007);
        assert_eq!((s.next, s.desync.clone()), (1, None));
        assert_eq!(s.take(1010, "ladder"), Some(0));
        // The first load of a script starts it.
        let mut f = script();
        f.origin = None;
        f.resume(500);
        assert_eq!((f.origin, f.next), (Some(500), 0));
    }

    #[test]
    fn the_text_form_reads_back() {
        let s = script();
        assert_eq!(DrawScript::parse(&s.text()).unwrap().draws, s.draws);
        assert!(DrawScript::parse("105 ladder 1\n").is_err());
        assert!(DrawScript::parse("gp-draws 0\n105 ladder\n").is_err());
    }

    #[test]
    fn draws_at_the_originals_points_take_its_values() {
        let mut s = script();
        assert_eq!(s.take(1005, "ladder"), Some(136));
        s.check_through(1009);
        assert_eq!(s.take(1010, "ladder"), Some(0));
        s.check_through(2000);
        assert_eq!(s.desync, None);
    }

    #[test]
    fn a_draw_on_another_frame_or_site_is_a_desync() {
        let mut s = script();
        assert_eq!(s.take(1004, "ladder"), None);
        let (f, why) = s.desync.clone().unwrap();
        assert_eq!(f, 104);
        assert!(
            why.contains("frame 104") && why.contains("frame 105"),
            "{why}"
        );
        // After a desync the generator draws.
        assert_eq!(s.take(1005, "ladder"), None);

        let mut s = script();
        assert_eq!(s.take(1005, "roll_off"), None);
        assert!(s.desync.is_some());
    }

    #[test]
    fn a_frames_sites_may_be_drawn_in_any_order() {
        let mut s =
            DrawScript::parse("gp-draws 0\n105 fire 7\n105 ladder 8\n105 ladder 9\n").unwrap();
        s.base = 100;
        s.start(1000);
        assert_eq!(s.take(1005, "ladder"), Some(8));
        assert_eq!(s.take(1005, "fire"), Some(7));
        assert_eq!(s.take(1005, "ladder"), Some(9));
        s.check_through(1005);
        assert_eq!((s.next, s.desync.clone()), (3, None));
    }

    #[test]
    fn a_draw_the_rebuild_skips_or_adds_is_a_desync() {
        let mut s = script();
        s.check_through(1005);
        assert!(s.desync.unwrap().1.contains("the rebuild didn't"));

        let mut s = script();
        s.take(1005, "ladder");
        s.take(1010, "ladder");
        assert_eq!(s.take(1020, "ladder"), None);
        assert!(
            s.desync
                .unwrap()
                .1
                .contains("after the original's last draw")
        );
    }
}
