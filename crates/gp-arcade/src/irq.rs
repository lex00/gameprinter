// license:BSD-3-Clause
// copyright-holders:Aaron Giles
//
// A port of device_execute_interface::device_input from MAME 0.276
// (src/emu/diexec.cpp): how a CPU's input line changes.

//! A CPU input line. MAME never changes a line on the spot: set_input_line
//! queues the change and sets a timer for "now", so the CPU that caused it
//! stops at the end of its current step and the line changes when the timer
//! fires. Getting this the same as MAME decides which instruction an
//! interrupt lands after.

use crate::sched::Scheduler;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IrqLine<T> {
    /// The timer kind that empties the queue.
    pub what: T,
    /// Queued changes: (asserted, vector).
    pub queue: Vec<(bool, i32)>,
    pub stored_vector: i32,
    pub curvector: i32,
    pub curstate: bool,
}

impl<T: Copy + Eq> IrqLine<T> {
    pub fn new(what: T, default_vector: i32) -> Self {
        Self {
            what,
            queue: Vec::new(),
            stored_vector: default_vector,
            curvector: default_vector,
            curstate: false,
        }
    }

    /// device_input::reset.
    pub fn reset(&mut self, default_vector: i32) {
        self.curvector = default_vector;
        self.stored_vector = default_vector;
    }

    /// set_state_synced with the stored vector: queue the change, and set
    /// the timer when the queue was empty.
    pub fn set(&mut self, sched: &mut Scheduler<T>, asserted: bool) {
        // MAME's queue holds 32 changes; more than that is flushed on the
        // spot, which never happens on these boards.
        self.queue.push((asserted, self.stored_vector));
        if self.queue.len() == 1 {
            sched.synchronize(self.what, 0);
        }
    }

    /// empty_event_queue: the changes in order, for the CPU to apply.
    pub fn take(&mut self) -> Vec<bool> {
        let events: Vec<(bool, i32)> = std::mem::take(&mut self.queue);
        events
            .into_iter()
            .map(|(state, vector)| {
                self.curstate = state;
                self.curvector = vector;
                state
            })
            .collect()
    }

    /// default_irq_callback: the vector, for an acknowledge without a
    /// daisy chain. (HOLD_LINE isn't used on these boards.)
    pub fn ack_vector(&self) -> i32 {
        self.curvector
    }

    #[cfg(test)]
    pub fn queued(&self) -> Vec<bool> {
        self.queue.iter().map(|&(s, _)| s).collect()
    }

    #[cfg(test)]
    pub fn clear_queue(&mut self) {
        self.queue.clear();
    }
}
