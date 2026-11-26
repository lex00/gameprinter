//! Threads with a main thread's room on their stack, for work a server
//! hands off: a request, the making of a game's page, a replay.
//!
//! A thread from `std::thread::spawn` has 2 MB of stack, a quarter of the
//! main thread's 8 MB. Making a game's page (`mechanic::server::prepare`)
//! and some of its requests go deep enough in a debug build to need more:
//! check-web on GitHub's Linux runners failed at mechanic-page with gp
//! serve's "thread '<unknown>' has overflowed its stack" when it made the
//! game's page on a thread of its own, while `gp mechanic start
//! --own-server`, which makes it on the main thread, passed. The room is
//! address space taken when touched, so a thread that stays shallow costs
//! what it did.

/// The stack a deep thread has: four times the main thread's.
pub(crate) const STACK: usize = 32 << 20;

/// Runs `f` on a new thread with [`STACK`] bytes of stack.
pub(crate) fn spawn<F, T>(f: F) -> std::thread::JoinHandle<T>
where
    F: FnOnce() -> T + Send + 'static,
    T: Send + 'static,
{
    std::thread::Builder::new()
        .stack_size(STACK)
        .spawn(f)
        .expect("starting a thread")
}

#[cfg(test)]
mod tests {
    /// Uses about `kb` KB of stack, one KB a call.
    #[inline(never)]
    fn deep(kb: usize) -> u8 {
        let buf = std::hint::black_box([kb as u8; 1024]);
        if kb == 0 {
            buf[0]
        } else {
            deep(kb - 1).wrapping_add(buf[1023])
        }
    }

    #[test]
    fn a_deep_thread_has_room_for_a_main_threads_frames() {
        // 8 MB of frames: four times what std::thread::spawn gives.
        assert!(super::spawn(|| deep(8 << 10)).join().is_ok());
    }
}
