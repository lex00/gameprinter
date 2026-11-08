//! Platform-specific time and date methods.

use cfg_if::cfg_if;

cfg_if! {
    if #[cfg(all(target_arch = "wasm32", target_os = "unknown"))] {
        pub use web_time::{Duration, Instant};
    } else {
        pub use std::time::{Duration, Instant};
    }
}
