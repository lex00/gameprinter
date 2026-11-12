//! Standard image and sprite formats (docs/formats/image-config.md): a
//! layered, versioned config for how frames are laid out in image files
//! ([`config`]), palette files for editors ([`palette`]), and the writer
//! and reader of sheets with their JSON sidecar ([`sheet`]). `gp sprites
//! export` and `import` use it, and so does `assets_place` for RGBA
//! images.

pub mod config;
pub mod palette;
pub mod sheet;

#[cfg(test)]
mod tests;
