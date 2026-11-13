//! The Tapper auto-player, as an example: the same code as
//! `gp mechanic autoplay` (gp_arcade::autoplay), for running it without gp.
//!
//!   cargo run --release -p gp-arcade --example autoplay -- \
//!       --rom ~/Documents/roms/arcade/tapper.zip --out DIR/NAME

fn main() {
    gp_arcade::autoplay::main_with(std::env::args().skip(1));
}
