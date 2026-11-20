//! The original's random draws at a game's draw sites (a state mapping's
//! `[[draws]]`, docs/formats/draws.md), recorded a frame at a time on the
//! main Z80 as the machine runs.
//!
//! A Z80 game's random routine returns its number in A (a `call` of the
//! game's random routine), so a site can name either the call (what `gp
//! interp draws --find` proposes) or the instruction after it (where A
//! holds the draw): either way the draw is A when control comes back from
//! the routine, which may be a frame later. Any other site draws the first
//! byte its instruction reads (the site's `ram` when it reads none), and a
//! site with `reg` the register as its instruction reads it ([`z80_reg`]).
//! A draw is in the frame it completes in.
//!
//! [`Recorder`] keeps the trace and the access log to the sites' own
//! addresses and their calls' returns ([`Mcr::set_watch`]), so it costs
//! little enough to run on every frame of a person's play: `gp`'s
//! recordings (gp-cli's `record_draws_arcade`) and the page's player
//! (gp-arcade-play) both record through it, so the two give the same
//! draws.

use std::collections::BTreeMap;

use crate::Mcr;
use crate::dasm::{self, Flow};
use crate::mcr::{AccessKind, Trace};

/// One draw site.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Site {
    /// The name the model's hooks draw by.
    pub site: String,
    /// The instruction that draws.
    pub pc: u16,
    /// The byte it reads, when it reads none of its own.
    pub ram: Option<u16>,
    /// A register it draws instead of a byte (`a` to `l`, `i` or `r`).
    pub reg: Option<String>,
    /// Read only for presentation (docs/formats/draws.md, `soft`).
    pub soft: bool,
}

impl Site {
    /// One site in its line form: `SITE PC ram=ADDR`, `SITE PC reg=R` or
    /// `SITE PC`, then `soft` for a soft site; numbers in hex with `0x`
    /// or in decimal. What the page's player takes (gp-arcade-play's
    /// `gp_play_draw_sites`).
    pub fn parse_line(line: &str) -> Result<Self, String> {
        let num = |t: &str| {
            let v = match t.strip_prefix("0x").or_else(|| t.strip_prefix("0X")) {
                Some(h) => u16::from_str_radix(h, 16),
                None => t.parse(),
            };
            v.map_err(|_| format!("draw site `{line}`: {t} isn't an address"))
        };
        let mut words = line.split_whitespace();
        let (Some(site), Some(pc)) = (words.next(), words.next()) else {
            return Err(format!("draw site `{line}`: give its name and address"));
        };
        let mut s = Site {
            site: site.to_owned(),
            pc: num(pc)?,
            ram: None,
            reg: None,
            soft: false,
        };
        for w in words {
            if let Some(a) = w.strip_prefix("ram=") {
                s.ram = Some(num(a)?);
            } else if let Some(r) = w.strip_prefix("reg=") {
                s.reg = Some(r.to_owned());
            } else if w == "soft" {
                s.soft = true;
            } else {
                return Err(format!("draw site `{line}`: {w}?"));
            }
        }
        Ok(s)
    }

    /// The line [`Site::parse_line`] reads.
    pub fn line(&self) -> String {
        let mut l = format!("{} {:#06x}", self.site, self.pc);
        if let Some(a) = self.ram {
            l += &format!(" ram={a:#06x}");
        }
        if let Some(r) = &self.reg {
            l += &format!(" reg={r}");
        }
        if self.soft {
            l += " soft";
        }
        l
    }
}

/// Records the draws a machine makes at a set of sites, frame by frame:
/// [`Recorder::arm`] before a frame, [`Recorder::take`] after it.
#[derive(Debug, Clone)]
pub struct Recorder {
    sites: Vec<Site>,
    /// Sites just after a call: the call's address.
    after_call: BTreeMap<u16, u16>,
    /// Calls waiting for their routine to return: site, return address, SP.
    pending: Vec<(String, u16, u16)>,
    /// The addresses the trace keeps.
    watch: Vec<u16>,
}

impl Recorder {
    /// A recorder for `sites` on the code `t` runs (read to tell a call
    /// site from one after a call). A register site's register is checked
    /// here, so a frame's recording never fails.
    pub fn new(sites: Vec<Site>, t: &Mcr) -> Result<Self, String> {
        let decode = |at: u16| {
            let bytes: Vec<u8> = (0..4).map(|k| t.peek(at.wrapping_add(k))).collect();
            dasm::decode(&bytes, at)
        };
        let calls = |i: &dasm::Insn| matches!(i.flow, Flow::Call(_) | Flow::CallIf(_));
        let mut after_call = BTreeMap::new();
        let mut watch = Vec::new();
        for s in &sites {
            if let Some(r) = &s.reg
                && !REGS.contains(&r.as_str())
            {
                return Err(format!(
                    "draw site {}: register {r:?} isn't one of the Z80's",
                    s.site
                ));
            }
            watch.push(s.pc);
            if s.reg.is_some() {
                continue;
            }
            let here = decode(s.pc);
            // A site that is itself a call draws by the call; otherwise a
            // `call nn` just before it makes it the place the number lands.
            let at = s.pc.wrapping_sub(3);
            let before = decode(at);
            if !calls(&here) && before.len == 3 && calls(&before) {
                after_call.insert(s.pc, at);
            }
            if matches!(here.flow, Flow::Call(_) | Flow::CallIf(_) | Flow::Rst(_)) {
                watch.push(s.pc.wrapping_add(u16::from(here.len)));
            }
        }
        Ok(Self {
            sites,
            after_call,
            pending: Vec::new(),
            watch,
        })
    }

    /// The sites.
    pub fn sites(&self) -> &[Site] {
        &self.sites
    }

    /// Forgets the calls waiting to return: after the machine is put back
    /// to another state.
    pub fn forget(&mut self) {
        self.pending.clear();
    }

    /// Turns on the trace and the access log, kept to the sites, for the
    /// next frame.
    pub fn arm(&self, t: &mut Mcr) {
        t.set_watch(Some(&self.watch));
        t.set_trace(true);
        t.set_accesses(true);
    }

    /// The draws of the frame run since [`Recorder::arm`], site and byte
    /// in order, and the trace and the log off again.
    pub fn take(&mut self, t: &mut Mcr) -> Vec<(String, u8)> {
        let mut trace: Vec<Trace> = t.take_trace().into_iter().filter(|e| e.cpu == 0).collect();
        trace.sort_by_key(|e| e.time);
        let acc: Vec<_> = t
            .take_accesses()
            .into_iter()
            .filter(|a| a.cpu == 0 && a.kind == AccessKind::Read)
            .collect();
        t.set_trace(false);
        t.set_accesses(false);
        t.set_watch(None);
        let mut out = Vec::new();
        let mut j = 0;
        for (n, e) in trace.iter().enumerate() {
            let end = trace.get(n + 1).map(|x| x.time);
            let from_j = j;
            while j < acc.len() && end.is_none_or(|end| acc[j].time < end) {
                j += 1;
            }
            if e.interrupt.is_some() {
                continue;
            }
            if let Some(k) = self.pending.iter().position(|p| p.1 == e.pc && p.2 == e.sp) {
                let (site, _, _) = self.pending.remove(k);
                out.push((site, (e.af >> 8) as u8));
            }
            let Some(d) = self.sites.iter().find(|d| d.pc == e.pc) else {
                continue;
            };
            if let Some(r) = &d.reg {
                out.push((d.site.clone(), z80_reg(e, r).unwrap_or(0)));
                continue;
            }
            if let Some(&call) = self.after_call.get(&e.pc) {
                // Back from the routine, not a conditional call that fell
                // through without calling.
                if e.prev_pc != call {
                    out.push((d.site.clone(), (e.af >> 8) as u8));
                }
                continue;
            }
            let insn = dasm::decode(&e.bytes, e.pc);
            match insn.flow {
                Flow::Call(_) | Flow::CallIf(_) | Flow::Rst(_) => {
                    let back = e.pc.wrapping_add(u16::from(insn.len));
                    // A call not taken comes straight back with SP as is.
                    self.pending.push((d.site.clone(), back, e.sp));
                }
                _ => {
                    let value = acc[from_j..j]
                        .first()
                        .map_or_else(|| d.ram.map_or(0, |at| t.peek(at)), |a| a.value);
                    out.push((d.site.clone(), value));
                }
            }
        }
        out
    }
}

/// The registers a register site may name.
pub const REGS: [&str; 10] = ["a", "f", "b", "c", "d", "e", "h", "l", "i", "r"];

/// The value a register site (`reg`) draws: the register as the
/// instruction traced at `e` reads it. Every register but R is as it was
/// at the instruction's start. R counts opcode fetches, the instruction's
/// own as well (two with a prefix byte), so `ld a,r` loads R two past its
/// count at the start, with bit 7 the one `ld r,a` set last.
pub fn z80_reg(e: &Trace, reg: &str) -> Result<u8, String> {
    let hi = |w: u16| (w >> 8) as u8;
    let lo = |w: u16| w as u8;
    Ok(match reg {
        "a" => hi(e.af),
        "f" => lo(e.af),
        "b" => hi(e.bc),
        "c" => lo(e.bc),
        "d" => hi(e.de),
        "e" => lo(e.de),
        "h" => hi(e.hl),
        "l" => lo(e.hl),
        "i" => e.i,
        "r" => {
            let fetches = if matches!(e.bytes[0], 0xcb | 0xdd | 0xed | 0xfd) {
                2
            } else {
                1
            };
            (e.r.wrapping_add(fetches) & 0x7f) | (e.r2 & 0x80)
        }
        _ => return Err(format!("draw site register {reg:?} isn't one of the Z80's")),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_site_line_reads_back() {
        for l in [
            "pick 0x0003 ram=0xe002",
            "twice 0x467e reg=r",
            "buzz 0xfc62 ram=0x0018 soft",
        ] {
            let s = Site::parse_line(l).unwrap();
            assert_eq!(s.line(), l);
        }
        let s = Site::parse_line("x 16").unwrap();
        assert_eq!((s.pc, s.ram, s.reg, s.soft), (16, None, None, false));
        assert!(Site::parse_line("x").is_err());
        assert!(Site::parse_line("x 0x10 loud").is_err());
    }
}
