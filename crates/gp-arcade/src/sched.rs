// license:BSD-3-Clause
// copyright-holders:Aaron Giles
//
// The scheduling rules of MAME 0.276's device_scheduler, emu_timer and
// device_execute_interface (src/emu/schedule.cpp, diexec.cpp), rewritten
// for a machine with a fixed set of CPUs and timers. The order things
// happen in follows MAME's code exactly, since the frame-by-frame match
// depends on it.

//! Timeslices and timers, as MAME has them.
//!
//! - Timers sit in one list ordered by expiry time; a timer that ties with
//!   others goes after them.
//! - A timeslice runs every CPU in turn up to the next timer, or to the end
//!   of the quantum. A CPU may run past the target by the rest of its
//!   current step; that overshoot is carried into the next timeslice.
//! - A timer set while a CPU runs starts at that CPU's local time. If it
//!   becomes the first timer, the CPU stops at the end of its current step
//!   (abort_timeslice), and the CPUs after it only run up to where it
//!   stopped.
//! - When the slice ends, every timer due by then fires, in list order.

use crate::attotime::{ATTOSECONDS_PER_SECOND, Attotime};

/// The per-CPU half of device_execute_interface.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Exec {
    pub localtime: Attotime,
    pub cycles_running: i32,
    pub cycles_stolen: i32,
    pub totalcycles: u64,
    pub cycles_per_second: u32,
    pub attoseconds_per_cycle: i64,
    pub divshift: u32,
    pub divisor: u32,
    pub suspended: bool,
}

impl Exec {
    /// interface_clock_changed for a CPU at `clock` Hz.
    pub fn new(clock: u32) -> Self {
        let attoseconds_per_cycle = ATTOSECONDS_PER_SECOND / i64::from(clock);
        let mut attos = attoseconds_per_cycle;
        let mut divshift = 0;
        while attos >= (1i64 << 31) {
            divshift += 1;
            attos >>= 1;
        }
        Self {
            localtime: Attotime::ZERO,
            cycles_running: 0,
            cycles_stolen: 0,
            totalcycles: 0,
            cycles_per_second: clock,
            attoseconds_per_cycle,
            divshift,
            divisor: attos as u32,
            suspended: false,
        }
    }

    /// cycles_to_attotime.
    pub fn cycles_to_attotime(&self, cycles: i64) -> Attotime {
        if cycles < i64::from(self.cycles_per_second) {
            Attotime::new(0, cycles * self.attoseconds_per_cycle)
        } else {
            let secs = cycles / i64::from(self.cycles_per_second);
            let rem = cycles % i64::from(self.cycles_per_second);
            Attotime::new(secs as i32, rem * self.attoseconds_per_cycle)
        }
    }

    /// local_time while this CPU runs, `icount` cycles short of its target.
    pub fn local_time(&self, icount: i32) -> Attotime {
        let cycles = self.cycles_running - icount;
        self.localtime + self.cycles_to_attotime(i64::from(cycles))
    }
}

/// One emu_timer. `T` names what it does when it fires.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Timer<T> {
    pub what: T,
    pub param: i32,
    pub enabled: bool,
    pub temporary: bool,
    pub period: Attotime,
    pub start: Attotime,
    pub expire: Attotime,
}

/// The device_scheduler, minus the CPUs themselves (the machine runs them
/// and tells the scheduler what they did).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Scheduler<T> {
    /// Every timer by handle; handles of fired temporary timers are reused.
    pub timers: Vec<Option<Timer<T>>>,
    /// Handles of the enabled timers, earliest first.
    pub list: Vec<usize>,
    pub basetime: Attotime,
    /// The quantum: the longest a timeslice runs with no timer due.
    pub quantum: i64,
    /// The CPU running now, and its cycle counter as last handed to a
    /// device (devices see time through it).
    pub executing: Option<usize>,
    pub icount: i32,
    /// Set while a timer callback runs: MAME's m_callback_timer and the
    /// time it was due.
    pub callback: Option<(usize, Attotime)>,
    pub callback_modified: bool,
    pub execs: Vec<Exec>,
}

impl<T: Copy + Eq> Scheduler<T> {
    pub fn new(execs: Vec<Exec>, quantum: Attotime) -> Self {
        Self {
            timers: Vec::new(),
            list: Vec::new(),
            basetime: Attotime::ZERO,
            quantum: quantum.attoseconds,
            executing: None,
            icount: 0,
            callback: None,
            callback_modified: false,
            execs,
        }
    }

    /// device_scheduler::time().
    pub fn time(&self) -> Attotime {
        if let Some((_, expire)) = self.callback {
            return expire;
        }
        match self.executing {
            Some(cpu) => self.execs[cpu].local_time(self.icount),
            None => self.basetime,
        }
    }

    /// timer_alloc: a permanent timer that isn't running.
    pub fn alloc(&mut self, what: T) -> usize {
        let start = self.time();
        self.timers.push(Some(Timer {
            what,
            param: 0,
            enabled: false,
            temporary: false,
            period: Attotime::NEVER,
            start,
            expire: Attotime::NEVER,
        }));
        self.timers.len() - 1
    }

    fn insert(&mut self, handle: usize) {
        let t = self.timers[handle].as_ref().expect("live timer");
        if t.expire.is_never() || !t.enabled {
            return;
        }
        let expire = t.expire;
        let pos = self
            .list
            .iter()
            .position(|&h| self.timers[h].as_ref().expect("live").expire > expire)
            .unwrap_or(self.list.len());
        self.list.insert(pos, handle);
    }

    fn remove(&mut self, handle: usize) {
        if let Some(pos) = self.list.iter().position(|&h| h == handle) {
            self.list.remove(pos);
        }
    }

    /// emu_timer::adjust.
    pub fn adjust(&mut self, handle: usize, delay: Attotime, param: i32, period: Attotime) {
        if let Some((cb, _)) = self.callback
            && cb == handle
        {
            self.callback_modified = true;
        }
        let delay = if delay.seconds < 0 {
            Attotime::ZERO
        } else {
            delay
        };
        let now = self.time();
        {
            let t = self.timers[handle].as_mut().expect("live timer");
            t.param = param;
            t.enabled = true;
            t.start = now;
            t.expire = now + delay;
            t.period = period;
        }
        self.remove(handle);
        self.insert(handle);
        if self.list.first() == Some(&handle) {
            self.abort_timeslice();
        }
    }

    /// The timer's expiry, for remaining() and enabled().
    pub fn timer(&self, handle: usize) -> &Timer<T> {
        self.timers[handle].as_ref().expect("live timer")
    }

    /// emu_timer::remaining.
    pub fn remaining(&self, handle: usize) -> Attotime {
        let now = self.time();
        let t = self.timer(handle);
        if now >= t.expire {
            Attotime::ZERO
        } else {
            t.expire - now
        }
    }

    /// synchronize / timer_set: a one-shot timer `delay` from now.
    pub fn timer_set(&mut self, delay: Attotime, what: T, param: i32) {
        let now = self.time();
        let timer = Timer {
            what,
            param,
            enabled: true,
            temporary: true,
            period: Attotime::NEVER,
            start: now,
            expire: now + delay,
        };
        let handle = match self.timers.iter().position(Option::is_none) {
            Some(h) => {
                self.timers[h] = Some(timer);
                h
            }
            None => {
                self.timers.push(Some(timer));
                self.timers.len() - 1
            }
        };
        self.insert(handle);
        if self.list.first() == Some(&handle) {
            self.abort_timeslice();
        }
    }

    pub fn synchronize(&mut self, what: T, param: i32) {
        self.timer_set(Attotime::ZERO, what, param);
    }

    /// device_execute_interface::abort_timeslice for the running CPU.
    pub fn abort_timeslice(&mut self) {
        if let Some(cpu) = self.executing
            && self.icount > 0
        {
            let e = &mut self.execs[cpu];
            e.cycles_stolen += self.icount;
            e.cycles_running -= self.icount;
            self.icount = 0;
        }
    }

    /// The time of the first timer.
    pub fn next_expire(&self) -> Attotime {
        self.list
            .first()
            .map_or(Attotime::NEVER, |&h| self.timer(h).expire)
    }

    /// The first part of execute_timers: the next timer due by the base
    /// time, marked as the running callback. Returns what it does and its
    /// parameter, or None when no timer is due.
    pub fn pop_due(&mut self) -> Option<(usize, T, i32, bool)> {
        let &handle = self.list.first()?;
        let base = self.basetime;
        let t = self.timers[handle].as_mut().expect("live timer");
        if t.expire > base {
            return None;
        }
        let was_enabled = t.enabled;
        if t.period == Attotime::ZERO || t.period.is_never() {
            t.enabled = false;
        }
        self.callback_modified = false;
        self.callback = Some((handle, t.expire));
        Some((handle, t.what, t.param, was_enabled))
    }

    /// The second part: reschedule or free the timer that just fired.
    pub fn finish_due(&mut self, handle: usize) {
        if !self.callback_modified {
            let t = self.timers[handle].as_mut().expect("live timer");
            if !t.temporary {
                // schedule_next_period
                t.start = t.expire;
                t.expire += t.period;
                self.remove(handle);
                self.insert(handle);
            } else {
                self.remove(handle);
                self.timers[handle] = None;
            }
        }
        self.callback = None;
    }

    /// The cycles a CPU should run to reach `target`, or None when it's
    /// already there (MAME runs no CPU for less than a cycle).
    pub fn cycles_to_run(&self, cpu: usize, target: Attotime) -> Option<i32> {
        let e = &self.execs[cpu];
        if e.suspended || target.seconds < e.localtime.seconds {
            return None;
        }
        let mut delta = target.attoseconds - e.localtime.attoseconds;
        if delta < 0 && target.seconds > e.localtime.seconds {
            delta += ATTOSECONDS_PER_SECOND;
        }
        if delta < e.attoseconds_per_cycle {
            return None;
        }
        Some(((delta as u64 >> e.divshift) / u64::from(e.divisor)) as i32)
    }

    /// Accounts for a CPU's run: the `given` cycles less what is left and
    /// what was taken back. Returns its new local time.
    pub fn account(&mut self, cpu: usize, given: i32, icount_left: i32) -> Attotime {
        let e = &mut self.execs[cpu];
        let ran = given - icount_left - e.cycles_stolen;
        e.totalcycles += ran as u64;
        let delta = if ran < e.cycles_per_second as i32 {
            Attotime::new(0, e.attoseconds_per_cycle * i64::from(ran))
        } else {
            let secs = ran as u32 / e.cycles_per_second;
            let rem = ran as u32 % e.cycles_per_second;
            Attotime::new(secs as i32, i64::from(rem) * e.attoseconds_per_cycle)
        };
        e.localtime += delta;
        e.localtime
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    enum What {
        A,
        B,
        C,
    }

    fn sched() -> Scheduler<What> {
        Scheduler::new(vec![Exec::new(5_000_000)], Attotime::from_hz(60))
    }

    #[test]
    fn ties_fire_in_the_order_they_were_set() {
        let mut s = sched();
        let a = s.alloc(What::A);
        let b = s.alloc(What::B);
        let c = s.alloc(What::C);
        let t = Attotime::new(0, 1000);
        s.adjust(b, t, 0, Attotime::NEVER);
        s.adjust(a, t, 0, Attotime::NEVER);
        s.adjust(c, Attotime::new(0, 500), 0, Attotime::NEVER);
        s.basetime = t;
        let mut order = Vec::new();
        while let Some((h, what, _, _)) = s.pop_due() {
            order.push(what);
            s.finish_due(h);
        }
        assert_eq!(order, [What::C, What::B, What::A]);
    }

    #[test]
    fn periodic_timers_come_back() {
        let mut s = sched();
        let a = s.alloc(What::A);
        let p = Attotime::new(0, 100);
        s.adjust(a, p, 0, p);
        s.basetime = Attotime::new(0, 250);
        let mut n = 0;
        while let Some((h, _, _, _)) = s.pop_due() {
            n += 1;
            s.finish_due(h);
        }
        assert_eq!(n, 2);
        assert_eq!(s.next_expire(), Attotime::new(0, 300));
    }

    #[test]
    fn a_timer_set_while_a_cpu_runs_stops_it() {
        let mut s = sched();
        s.execs[0].cycles_running = 100;
        s.executing = Some(0);
        s.icount = 60;
        // 40 cycles in, at 200 ns a cycle
        assert_eq!(s.time(), Attotime::new(0, 40 * 200_000_000_000));
        s.synchronize(What::A, 7);
        assert_eq!(s.icount, 0);
        assert_eq!(s.execs[0].cycles_running, 40);
        assert_eq!(s.execs[0].cycles_stolen, 60);
        // The CPU finishes a 3-cycle step: icount -3.
        s.executing = None;
        let t = s.account(0, 100, -3);
        assert_eq!(t, Attotime::new(0, 43 * 200_000_000_000));
    }

    #[test]
    fn cycles_round_down() {
        let s = sched();
        assert_eq!(s.cycles_to_run(0, Attotime::new(0, 199_999_999_999)), None);
        assert_eq!(
            s.cycles_to_run(0, Attotime::new(0, 200_000_000_000)),
            Some(1)
        );
        assert_eq!(
            s.cycles_to_run(0, Attotime::new(0, 1_000_000_000_000_001)),
            Some(5000)
        );
    }
}
