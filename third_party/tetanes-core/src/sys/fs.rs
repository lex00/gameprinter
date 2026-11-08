//! Platform-specific filesystem methods.

use cfg_if::cfg_if;

cfg_if! {
    if #[cfg(all(target_arch = "wasm32", target_os = "unknown"))] {
        mod wasm;
        pub use wasm::*;
    } else {
        mod os;
        pub use os::*;
    }
}
