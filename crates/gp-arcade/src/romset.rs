//! Reading a romset: a MAME zip, every chip checked by size and SHA-1
//! against the set's list.
//!
//! The chip lists (names, places, sizes and hashes) come from MAME 0.276's
//! src/mame/midway/mcr.cpp, `ROM_START( rbtapper )`, `tapper`, `tappera`,
//! `tapperg` and `sutapper`: license:BSD-3-Clause, copyright-holders:Aaron
//! Giles. The SSIO board's PROM comes from src/mame/midway/midway.cpp,
//! `ROM_START( midway_ssio )`, under the same licence and holder.
//!
//! # A set's identity
//!
//! A zip is known by its chips' contents, never by its file names: the
//! set is the one in [`SETS`] all of whose chips are in the zip by SHA-1.
//! Its identity, the `sha256` a games checkout's roms.toml and package.toml
//! give it, is [`identity`] of the set's chips: the SHA-256 of their SHA-1s,
//! in lower-case hex, sorted, each followed by a newline. So it is the
//! same for any zip holding those chips under any names, a person can
//! work it out from MAME's list without the ROMs, and two sets that share
//! every chip would share it (none here do). The board's own PROM
//! ([`SSIO_PROM`]) isn't in the set's list, so it isn't in the identity:
//! a zip with or without it is the same set.
//!
//! ROM bytes stay in memory. Nothing here prints or writes them.

use std::io::Read;
use std::path::Path;

/// One chip of a set: its file name, the region and offset it loads at,
/// its size and its SHA-1.
#[derive(Debug, Clone, Copy)]
pub struct Chip {
    pub name: &'static str,
    pub region: Region,
    pub offset: usize,
    pub size: usize,
    pub sha1: &'static str,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Region {
    /// The main board's Z80 program, 0x10000 bytes.
    MainCpu,
    /// The SSIO board's Z80 program, 0x10000 bytes.
    SsioCpu,
    /// Background tiles, 0x8000 bytes.
    Gfx1,
    /// Sprites, 0x20000 bytes.
    Gfx2,
}

impl Region {
    /// MAME's name for the region.
    pub fn name(self) -> &'static str {
        match self {
            Self::MainCpu => "maincpu",
            Self::SsioCpu => "ssio:cpu",
            Self::Gfx1 => "gfx1",
            Self::Gfx2 => "gfx2",
        }
    }

    pub fn size(self) -> usize {
        match self {
            Self::MainCpu | Self::SsioCpu => 0x10000,
            Self::Gfx1 => 0x8000,
            Self::Gfx2 => 0x20000,
        }
    }
}

const fn chip(
    name: &'static str,
    region: Region,
    offset: usize,
    size: usize,
    sha1: &'static str,
) -> Chip {
    Chip {
        name,
        region,
        offset,
        size,
        sha1,
    }
}

/// MAME's `tapper`.
pub const TAPPER: [Chip; 18] = [
    chip(
        "tapper_c.p.u._pg_0_1c_1-27-84.1c",
        Region::MainCpu,
        0x00000,
        0x4000,
        "ff5a729e36faea3758c8c7b345a42dd8bb465f44",
    ),
    chip(
        "tapper_c.p.u._pg_1_2c_1-27-84.2c",
        Region::MainCpu,
        0x04000,
        0x4000,
        "b9f0396e2eba5772deec4725c41fa9de49658e72",
    ),
    chip(
        "tapper_c.p.u._pg_2_3c_1-27-84.3c",
        Region::MainCpu,
        0x08000,
        0x4000,
        "434d3c27b9f1e43def081d79b9f56dbce93a9207",
    ),
    chip(
        "tapper_c.p.u._pg_3_4c_1-27-84.4c",
        Region::MainCpu,
        0x0c000,
        0x2000,
        "5e4e2dc1703b39f588ba374f6a610f273d710532",
    ),
    chip(
        "tapper_sound_snd_0_a7_12-7-83.a7",
        Region::SsioCpu,
        0x0000,
        0x1000,
        "9e281c340b7702523c86d56317efad9e3688e585",
    ),
    chip(
        "tapper_sound_snd_1_a8_12-7-83.a8",
        Region::SsioCpu,
        0x1000,
        0x1000,
        "14334b9d2bfece3fe5bda0cbd53158ead8d27e53",
    ),
    chip(
        "tapper_sound_snd_2_a9_12-7-83.a9",
        Region::SsioCpu,
        0x2000,
        0x1000,
        "b38bba5f12516d899e023f99147868e3402fbd7b",
    ),
    chip(
        "tapper_sound_snd_3_a10_12-7-83.a10",
        Region::SsioCpu,
        0x3000,
        0x1000,
        "0011407c1e886071282808c0a561789b1245a789",
    ),
    chip(
        "tapper_c.p.u._bg_1_6f_12-7-83.6f",
        Region::Gfx1,
        0x00000,
        0x4000,
        "eb30b9bb654324340f0fc5b44776ac2440c1e869",
    ),
    chip(
        "tapper_c.p.u._bg_0_5f_12-7-83.5f",
        Region::Gfx1,
        0x04000,
        0x4000,
        "23e29ec942e1e7516ae8068837af2d1c79592378",
    ),
    chip(
        "tapper_video_fg_1_a7_12-7-83.a7",
        Region::Gfx2,
        0x00000,
        0x4000,
        "a38667573d235efe2dc515e52af05825fe4e0f30",
    ),
    chip(
        "tapper_video_fg_0_a8_12-7-83.a8",
        Region::Gfx2,
        0x04000,
        0x4000,
        "2077f79177fda26f9c674b2ab525ec3833802059",
    ),
    chip(
        "tapper_video_fg_3_a5_12-7-83.a5",
        Region::Gfx2,
        0x08000,
        0x4000,
        "930142dd73fb30c4d3ec09a1e37517c6c6774024",
    ),
    chip(
        "tapper_video_fg_2_a6_12-7-83.a6",
        Region::Gfx2,
        0x0c000,
        0x4000,
        "d553b8517c1d03a2be0b065f4da2fa99d9e6fb30",
    ),
    chip(
        "tapper_video_fg_5_a3_12-7-83.a3",
        Region::Gfx2,
        0x10000,
        0x4000,
        "8aead89e1adaee5f0b679661c4bfba36e0d639e8",
    ),
    chip(
        "tapper_video_fg_4_a4_12-7-83.a4",
        Region::Gfx2,
        0x14000,
        0x4000,
        "402c166d50b4a693959b3f0706a7931a5daef6ce",
    ),
    chip(
        "tapper_video_fg_7_a1_12-7-83.a1",
        Region::Gfx2,
        0x18000,
        0x4000,
        "95879a455ecfe2e3de7fe2a75078f9e6934960f5",
    ),
    chip(
        "tapper_video_fg_6_a2_12-7-83.a2",
        Region::Gfx2,
        0x1c000,
        0x4000,
        "a24696f16d467d9eea4f25aef5f4c5ff55bf51ff",
    ),
];

/// MAME's `tappera`.
pub const TAPPERA: [Chip; 18] = [
    chip(
        "tapper_c.p.u._pg_0_1c_1-12-84.1c",
        Region::MainCpu,
        0x00000,
        0x4000,
        "373e9a9d73b71e100c02862662d025f5ead2f94d",
    ),
    chip(
        "tapper_c.p.u._pg_1_2c_1-12-84.1c",
        Region::MainCpu,
        0x04000,
        0x4000,
        "e493e46fc70a765e54bfdd7ba7ca570e6a5c79d6",
    ),
    chip(
        "tapper_c.p.u._pg_2_3c_1-12-84.3c",
        Region::MainCpu,
        0x08000,
        0x4000,
        "cb46a2248289ced7282b1463f433dcb970c42c1a",
    ),
    chip(
        "tapper_c.p.u._pg_3_4c_1-12-84.4c",
        Region::MainCpu,
        0x0c000,
        0x2000,
        "45bf1571a2418c7dc00ccc7061a3e04e65cb6bff",
    ),
    chip(
        "tapper_sound_snd_0_a7_12-7-83.a7",
        Region::SsioCpu,
        0x0000,
        0x1000,
        "9e281c340b7702523c86d56317efad9e3688e585",
    ),
    chip(
        "tapper_sound_snd_1_a8_12-7-83.a8",
        Region::SsioCpu,
        0x1000,
        0x1000,
        "14334b9d2bfece3fe5bda0cbd53158ead8d27e53",
    ),
    chip(
        "tapper_sound_snd_2_a9_12-7-83.a9",
        Region::SsioCpu,
        0x2000,
        0x1000,
        "b38bba5f12516d899e023f99147868e3402fbd7b",
    ),
    chip(
        "tapper_sound_snd_3_a10_12-7-83.a10",
        Region::SsioCpu,
        0x3000,
        0x1000,
        "0011407c1e886071282808c0a561789b1245a789",
    ),
    chip(
        "tapper_c.p.u._bg_1_6f_12-7-83.6f",
        Region::Gfx1,
        0x00000,
        0x4000,
        "eb30b9bb654324340f0fc5b44776ac2440c1e869",
    ),
    chip(
        "tapper_c.p.u._bg_0_5f_12-7-83.5f",
        Region::Gfx1,
        0x04000,
        0x4000,
        "23e29ec942e1e7516ae8068837af2d1c79592378",
    ),
    chip(
        "tapper_video_fg_1_a7_12-7-83.a7",
        Region::Gfx2,
        0x00000,
        0x4000,
        "a38667573d235efe2dc515e52af05825fe4e0f30",
    ),
    chip(
        "tapper_video_fg_0_a8_12-7-83.a8",
        Region::Gfx2,
        0x04000,
        0x4000,
        "2077f79177fda26f9c674b2ab525ec3833802059",
    ),
    chip(
        "tapper_video_fg_3_a5_12-7-83.a5",
        Region::Gfx2,
        0x08000,
        0x4000,
        "930142dd73fb30c4d3ec09a1e37517c6c6774024",
    ),
    chip(
        "tapper_video_fg_2_a6_12-7-83.a6",
        Region::Gfx2,
        0x0c000,
        0x4000,
        "d553b8517c1d03a2be0b065f4da2fa99d9e6fb30",
    ),
    chip(
        "tapper_video_fg_5_a3_12-7-83.a3",
        Region::Gfx2,
        0x10000,
        0x4000,
        "8aead89e1adaee5f0b679661c4bfba36e0d639e8",
    ),
    chip(
        "tapper_video_fg_4_a4_12-7-83.a4",
        Region::Gfx2,
        0x14000,
        0x4000,
        "402c166d50b4a693959b3f0706a7931a5daef6ce",
    ),
    chip(
        "tapper_video_fg_7_a1_12-7-83.a1",
        Region::Gfx2,
        0x18000,
        0x4000,
        "95879a455ecfe2e3de7fe2a75078f9e6934960f5",
    ),
    chip(
        "tapper_video_fg_6_a2_12-7-83.a2",
        Region::Gfx2,
        0x1c000,
        0x4000,
        "a24696f16d467d9eea4f25aef5f4c5ff55bf51ff",
    ),
];

/// MAME's `tapperg`.
pub const TAPPERG: [Chip; 18] = [
    chip(
        "tapper_c.p.u._pg_0_1c_1-27-84.1c",
        Region::MainCpu,
        0x00000,
        0x4000,
        "ff5a729e36faea3758c8c7b345a42dd8bb465f44",
    ),
    chip(
        "tapper_c.p.u._pg_1_2c_1-27-84.2c",
        Region::MainCpu,
        0x04000,
        0x4000,
        "b9f0396e2eba5772deec4725c41fa9de49658e72",
    ),
    chip(
        "tapper_c.p.u._pg_2_3c_1-27-84.3c",
        Region::MainCpu,
        0x08000,
        0x4000,
        "434d3c27b9f1e43def081d79b9f56dbce93a9207",
    ),
    chip(
        "tapper_c.p.u._pg_3_4c_1-27-84.4c",
        Region::MainCpu,
        0x0c000,
        0x2000,
        "5e4e2dc1703b39f588ba374f6a610f273d710532",
    ),
    chip(
        "tapper_sound_snd_0_a7_12-7-83.a7",
        Region::SsioCpu,
        0x0000,
        0x1000,
        "9e281c340b7702523c86d56317efad9e3688e585",
    ),
    chip(
        "tapper_sound_snd_1_a8_12-7-83.a8",
        Region::SsioCpu,
        0x1000,
        0x1000,
        "14334b9d2bfece3fe5bda0cbd53158ead8d27e53",
    ),
    chip(
        "tapper_sound_snd_2_a9_12-7-83.a9",
        Region::SsioCpu,
        0x2000,
        0x1000,
        "b38bba5f12516d899e023f99147868e3402fbd7b",
    ),
    chip(
        "tapper_sound_snd_3_a10_12-7-83.a10",
        Region::SsioCpu,
        0x3000,
        0x1000,
        "0011407c1e886071282808c0a561789b1245a789",
    ),
    chip(
        "tapper_c.p.u._bg_1_6f_12-7-83.6f",
        Region::Gfx1,
        0x00000,
        0x4000,
        "eb30b9bb654324340f0fc5b44776ac2440c1e869",
    ),
    chip(
        "tapper_c.p.u._bg_0_5f_12-7-83.5f",
        Region::Gfx1,
        0x04000,
        0x4000,
        "23e29ec942e1e7516ae8068837af2d1c79592378",
    ),
    chip(
        "fg1_a7.128",
        Region::Gfx2,
        0x00000,
        0x4000,
        "7fd26cc8ff2faab86d04fcee2b5ec49ecf6b8143",
    ),
    chip(
        "fg0_a8.128",
        Region::Gfx2,
        0x04000,
        0x4000,
        "45df1ac033512be942460d678a7c1ba9dcef1937",
    ),
    chip(
        "fg3_a5.128",
        Region::Gfx2,
        0x08000,
        0x4000,
        "0b28e7e59eba983bc1929758f8dcaf315b7134a1",
    ),
    chip(
        "fg2_a6.128",
        Region::Gfx2,
        0x0c000,
        0x4000,
        "faf631d4ee96edf6b2c4349780e2d89eaedf70ab",
    ),
    chip(
        "fg5_a3.128",
        Region::Gfx2,
        0x10000,
        0x4000,
        "9829c9574fff0803973246f9d22311ca76be4e58",
    ),
    chip(
        "fg4_a4.128",
        Region::Gfx2,
        0x14000,
        0x4000,
        "52672b853d67432fd80e4612fa54208c225d2444",
    ),
    chip(
        "fg7_a1.128",
        Region::Gfx2,
        0x18000,
        0x4000,
        "6fe170695386fc77310384a5590a7cc3671ae853",
    ),
    chip(
        "fg6_a2.128",
        Region::Gfx2,
        0x1c000,
        0x4000,
        "5a6d30974e73f952694b8c09fb3a5393a76db4f2",
    ),
];

/// MAME's `sutapper`.
pub const SUTAPPER: [Chip; 18] = [
    chip(
        "epr-5791",
        Region::MainCpu,
        0x0000,
        0x4000,
        "155dc1df977a474f3f1bd238d851c2ff8fe1faba",
    ),
    chip(
        "epr-5792",
        Region::MainCpu,
        0x4000,
        0x4000,
        "0eebe98be6c21d701c7b7fc6547b5c94f934f5ab",
    ),
    chip(
        "epr-5793",
        Region::MainCpu,
        0x8000,
        0x4000,
        "de365f4e567d93a9ed9672fabbc739a3a0d47d59",
    ),
    chip(
        "epr-5794",
        Region::MainCpu,
        0xc000,
        0x2000,
        "ee038443ae55598568bd1a53c0a671a2828d3949",
    ),
    chip(
        "epr-5788.h11.ic8",
        Region::SsioCpu,
        0x00000,
        0x1000,
        "c2c94ab26ebce30ce4efc239e555c6368794d265",
    ),
    chip(
        "epr-5787.h10.ic6",
        Region::SsioCpu,
        0x01000,
        0x1000,
        "f5c8585d443bca67d4065314a06431d1f104c553",
    ),
    chip(
        "epr-5786.h9.ic5",
        Region::SsioCpu,
        0x02000,
        0x1000,
        "2a4dc0deca48f4d2ac9fe673ecb9548415c996a9",
    ),
    chip(
        "epr-5785.h7.ic4",
        Region::SsioCpu,
        0x03000,
        0x1000,
        "a41323149c50adcae7675efcef69fd7d8365e527",
    ),
    chip(
        "epr-5790",
        Region::Gfx1,
        0x00000,
        0x4000,
        "f976889b529937948043460679f84b962b4c12bc",
    ),
    chip(
        "epr-5789",
        Region::Gfx1,
        0x04000,
        0x4000,
        "96b89dc08f2feeb9950fbbba43d0ee57a9e31804",
    ),
    chip(
        "epr-5795",
        Region::Gfx2,
        0x00000,
        0x4000,
        "3c26b0b0d903fb6782c6a1d72e32cd8c57ecad1f",
    ),
    chip(
        "epr-5796",
        Region::Gfx2,
        0x04000,
        0x4000,
        "c613c2225eeff5cc65dc6ec301e616e54755b1c2",
    ),
    chip(
        "epr-5797",
        Region::Gfx2,
        0x08000,
        0x4000,
        "ca54d1fa6704d38e65a4d2a94449ed8dd56cc94b",
    ),
    chip(
        "epr-5798",
        Region::Gfx2,
        0x0c000,
        0x4000,
        "1a6eac2a8fa99d86889d5042c6b64f828b3c5d65",
    ),
    chip(
        "epr-5799",
        Region::Gfx2,
        0x10000,
        0x4000,
        "7f4260f4a4e8b33842355e9d8e859ffb9278c3c2",
    ),
    chip(
        "epr-5800",
        Region::Gfx2,
        0x14000,
        0x4000,
        "251cf018da8db11c3844123255082146b22507e5",
    ),
    chip(
        "epr-5801",
        Region::Gfx2,
        0x18000,
        0x4000,
        "e8ceabe94080eb28aa393b97ec54729cf8aba001",
    ),
    chip(
        "epr-5802",
        Region::Gfx2,
        0x1c000,
        0x4000,
        "58ae647b4fd0f48af4158b85e29c813605d930d3",
    ),
];

/// MAME's `rbtapper`.
pub const RBTAPPER: [Chip; 18] = [
    chip(
        "rbtpg0.bin",
        Region::MainCpu,
        0x00000,
        0x4000,
        "31b583c98e7c2cecab42223df8b5400004c9f3d3",
    ),
    chip(
        "rbtpg1.bin",
        Region::MainCpu,
        0x04000,
        0x4000,
        "7de10632b2538524fee418175e2cc41be9c8c0e8",
    ),
    chip(
        "rbtpg2.bin",
        Region::MainCpu,
        0x08000,
        0x4000,
        "b9878c8a61a98e787e547bb6ab81c809875891f3",
    ),
    chip(
        "rbtpg3.bin",
        Region::MainCpu,
        0x0c000,
        0x2000,
        "ddb21e39ede2222cb2286ec9dba06341fe1c9db7",
    ),
    chip(
        "5788",
        Region::SsioCpu,
        0x00000,
        0x1000,
        "c2c94ab26ebce30ce4efc239e555c6368794d265",
    ),
    chip(
        "5787",
        Region::SsioCpu,
        0x01000,
        0x1000,
        "f5c8585d443bca67d4065314a06431d1f104c553",
    ),
    chip(
        "5786",
        Region::SsioCpu,
        0x02000,
        0x1000,
        "2a4dc0deca48f4d2ac9fe673ecb9548415c996a9",
    ),
    chip(
        "5785",
        Region::SsioCpu,
        0x03000,
        0x1000,
        "a41323149c50adcae7675efcef69fd7d8365e527",
    ),
    chip(
        "rbtbg1.bin",
        Region::Gfx1,
        0x00000,
        0x4000,
        "9e96a3ff0b66a4b1d07fe86059f7dfab0d53231c",
    ),
    chip(
        "rbtbg0.bin",
        Region::Gfx1,
        0x04000,
        0x4000,
        "a4b24fffbbe299e0c2058abe372f00954e5edf98",
    ),
    chip(
        "rbtfg1.bin",
        Region::Gfx2,
        0x00000,
        0x4000,
        "532239605b74b137cd0f6035a6bc7ac43f559d82",
    ),
    chip(
        "rbtfg0.bin",
        Region::Gfx2,
        0x04000,
        0x4000,
        "3a8e3e927b0fd2d49222e417e4c1f28b5a45c6ca",
    ),
    chip(
        "rbtfg3.bin",
        Region::Gfx2,
        0x08000,
        0x4000,
        "3811f499aa076fb81af42f7cb522879b0af50e37",
    ),
    chip(
        "rbtfg2.bin",
        Region::Gfx2,
        0x0c000,
        0x4000,
        "d02a306f8b32b3a097fc51e75e106a40fe942d35",
    ),
    chip(
        "rbtfg5.bin",
        Region::Gfx2,
        0x10000,
        0x4000,
        "523efaba9a3cbe871868ee09caea062382f31fad",
    ),
    chip(
        "rbtfg4.bin",
        Region::Gfx2,
        0x14000,
        0x4000,
        "e2a427324fc1d4422e50a1ab79822aff0290e109",
    ),
    chip(
        "rbtfg7.bin",
        Region::Gfx2,
        0x18000,
        0x4000,
        "dab52db0bca13b9f3470060bc3babe4a6c2cdcc8",
    ),
    chip(
        "rbtfg6.bin",
        Region::Gfx2,
        0x1c000,
        0x4000,
        "400d390729601610b152e21da55399399830221f",
    ),
];

/// A set this board runs: MAME's name for it, its title, and its chips.
#[derive(Debug, Clone, Copy)]
pub struct SetInfo {
    pub name: &'static str,
    pub title: &'static str,
    pub year: u16,
    pub chips: &'static [Chip],
}

impl SetInfo {
    /// The set's identity ([`identity`] of its chips).
    pub fn identity(&self) -> String {
        identity(self.chips.iter().map(|c| c.sha1))
    }
}

/// The Tapper sets, all MAME's `mcr_91490` board, rbtapper first. tapperb
/// (12/9/83) is left out: nobody here has it to check against.
pub const SETS: [SetInfo; 5] = [
    SetInfo {
        name: "rbtapper",
        title: "Tapper (Root Beer)",
        year: 1984,
        chips: &RBTAPPER,
    },
    SetInfo {
        name: "tapper",
        title: "Tapper (Budweiser, 1/27/84)",
        year: 1983,
        chips: &TAPPER,
    },
    SetInfo {
        name: "tappera",
        title: "Tapper (Budweiser, 1/12/84)",
        year: 1983,
        chips: &TAPPERA,
    },
    SetInfo {
        name: "tapperg",
        title: "Tapper (Budweiser, 1/27/84 - Alternate graphics)",
        year: 1983,
        chips: &TAPPERG,
    },
    SetInfo {
        name: "sutapper",
        title: "Tapper (Suntory)",
        year: 1983,
        chips: &SUTAPPER,
    },
];

/// The set MAME calls `name`.
pub fn set(name: &str) -> Option<&'static SetInfo> {
    SETS.iter().find(|s| s.name == name)
}

/// A set's identity from its chips' SHA-1s (lower-case hex): the SHA-256 of
/// them sorted, each followed by a newline, in lower-case hex.
pub fn identity<'a>(sha1s: impl IntoIterator<Item = &'a str>) -> String {
    use sha2::Digest as _;
    let mut v: Vec<&str> = sha1s.into_iter().collect();
    v.sort_unstable();
    let mut h = sha2::Sha256::new();
    for s in v {
        h.update(s.as_bytes());
        h.update(b"\n");
    }
    hex(&h.finalize())
}

/// The SSIO board's 82S123 PROM: the board's own ROM, which MAME's set lists
/// don't name. Only the sound volumes read it. Zips carry it as
/// `82s123.12d` or (rbtapper's) `5784`, so it is found by its hash.
pub const SSIO_PROM: Chip = Chip {
    name: "82s123.12d",
    region: Region::SsioCpu,
    offset: 0,
    size: 32,
    sha1: "9ac9b01d24affc0ee9227a4364c4fd8f8290343a",
};

/// Where a set's zip is on this machine, for the tests and checks that
/// play it and skip without it: `$GP_ARCADE_<NAME>` (upper case), else
/// `$GP_ROM_DIR/arcade/<name>.zip`, else `~/Downloads/<name>.zip`.
pub fn local_romset(name: &str) -> Option<std::path::PathBuf> {
    use std::path::PathBuf;
    let env = |k: &str| {
        std::env::var_os(k)
            .filter(|v| !v.is_empty())
            .map(PathBuf::from)
    };
    let file = format!("{name}.zip");
    [
        env(&format!("GP_ARCADE_{}", name.to_uppercase())),
        env("GP_ROM_DIR").map(|d| d.join("arcade").join(&file)),
        env("HOME").map(|h| h.join("Downloads").join(&file)),
    ]
    .into_iter()
    .flatten()
    .find(|p| p.is_file())
}

/// Whether `data` starts as a zip file does.
pub fn looks_like_zip(data: &[u8]) -> bool {
    data.starts_with(b"PK\x03\x04") || data.starts_with(b"PK\x05\x06")
}

/// A loaded set: the four regions, and what else the zip held.
#[derive(Clone)]
pub struct Romset {
    /// MAME's name for the set.
    pub name: &'static str,
    /// Its identity ([`identity`]).
    pub identity: String,
    pub maincpu: Vec<u8>,
    pub ssiocpu: Vec<u8>,
    pub gfx1: Vec<u8>,
    pub gfx2: Vec<u8>,
    /// Files in the zip that aren't chips of the set, with what they are
    /// when MAME knows them. Reported, then ignored.
    pub extras: Vec<String>,
    /// The SSIO board's PROM ([`SSIO_PROM`]), found by its hash under
    /// whatever name: the sound's volumes come from it. Without it the
    /// board is silent.
    pub ssio_prom: Option<Vec<u8>>,
}

impl std::fmt::Debug for Romset {
    // Never the bytes.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Romset")
            .field("name", &self.name)
            .field("identity", &self.identity)
            .field("extras", &self.extras)
            .finish_non_exhaustive()
    }
}

#[derive(Debug, thiserror::Error)]
pub enum RomError {
    #[error("{0}: {1}")]
    Io(String, std::io::Error),
    #[error("{0}: not a zip file I can read ({1})")]
    Zip(String, String),
    #[error("{set}: chip {chip} is missing")]
    Missing {
        set: &'static str,
        chip: &'static str,
    },
    #[error("{set}: chip {chip} is {found} bytes, the set's list says {want}")]
    Size {
        set: &'static str,
        chip: &'static str,
        found: usize,
        want: usize,
    },
    #[error("{set}: chip {chip} has SHA-1 {found}, the set's list says {want}")]
    Hash {
        set: &'static str,
        chip: &'static str,
        found: String,
        want: &'static str,
    },
    #[error(
        "the zip matches no set gp-arcade knows ({known}); the nearest is {nearest}, with {have} of its {of} chips"
    )]
    Unknown {
        known: String,
        nearest: &'static str,
        have: usize,
        of: usize,
    },
}

impl Romset {
    /// Reads a MAME zip and finds which set of [`SETS`] it is.
    pub fn load_zip(path: &Path) -> Result<Self, RomError> {
        let shown = path.display().to_string();
        let data = std::fs::read(path).map_err(|e| RomError::Io(shown.clone(), e))?;
        Self::from_zip(&data).map_err(|e| match e {
            RomError::Zip(_, why) => RomError::Zip(shown, why),
            e => e,
        })
    }

    /// [`Romset::load_zip`] on a zip's bytes.
    pub fn from_zip(data: &[u8]) -> Result<Self, RomError> {
        let files = read_zip(data).map_err(|e| RomError::Zip("the romset".into(), e))?;
        Self::identify(files)
    }

    /// The set of [`SETS`] whose every chip is among `files` by SHA-1,
    /// whatever the files are named, loaded from them.
    pub fn identify(files: Vec<(String, Vec<u8>)>) -> Result<Self, RomError> {
        let have: std::collections::HashSet<String> = files
            .iter()
            .map(|(_, d)| hex(&crate::sha1::sha1(d)))
            .collect();
        let count = |s: &SetInfo| s.chips.iter().filter(|c| have.contains(c.sha1)).count();
        if let Some(s) = SETS.iter().find(|s| count(s) == s.chips.len()) {
            return Self::from_files(s.name, s.chips, files);
        }
        let nearest = SETS
            .iter()
            .max_by_key(|s| count(s))
            .expect("SETS isn't empty");
        Err(RomError::Unknown {
            known: SETS.iter().map(|s| s.name).collect::<Vec<_>>().join(", "),
            nearest: nearest.name,
            have: count(nearest),
            of: nearest.chips.len(),
        })
    }

    /// Builds a set from (name, bytes) pairs. A chip is found by its name,
    /// or else by its SHA-1, as MAME does.
    pub fn from_files(
        name: &'static str,
        chips: &[Chip],
        files: Vec<(String, Vec<u8>)>,
    ) -> Result<Self, RomError> {
        let hashed: Vec<(String, String, Vec<u8>)> = files
            .into_iter()
            .map(|(n, d)| (n, hex(&crate::sha1::sha1(&d)), d))
            .collect();
        let mut used = vec![false; hashed.len()];
        let mut regions = [
            vec![0u8; Region::MainCpu.size()],
            vec![0u8; Region::SsioCpu.size()],
            vec![0u8; Region::Gfx1.size()],
            vec![0u8; Region::Gfx2.size()],
        ];
        for c in chips {
            let found = hashed
                .iter()
                .position(|(n, _, _)| n == c.name)
                .or_else(|| hashed.iter().position(|(_, h, _)| h == c.sha1));
            let Some(i) = found else {
                return Err(RomError::Missing {
                    set: name,
                    chip: c.name,
                });
            };
            let (_, hash, bytes) = &hashed[i];
            if bytes.len() != c.size {
                return Err(RomError::Size {
                    set: name,
                    chip: c.name,
                    found: bytes.len(),
                    want: c.size,
                });
            }
            if hash != c.sha1 {
                return Err(RomError::Hash {
                    set: name,
                    chip: c.name,
                    found: hash.clone(),
                    want: c.sha1,
                });
            }
            used[i] = true;
            let r = match c.region {
                Region::MainCpu => 0,
                Region::SsioCpu => 1,
                Region::Gfx1 => 2,
                Region::Gfx2 => 3,
            };
            regions[r][c.offset..c.offset + c.size].copy_from_slice(bytes);
        }
        let mut ssio_prom = None;
        let mut extras = Vec::new();
        for ((n, h, d), _) in hashed.iter().zip(&used).filter(|(_, u)| !**u) {
            if h == SSIO_PROM.sha1 {
                ssio_prom = Some(d.clone());
            } else {
                extras.push(format!(
                    "{n} ({} bytes, SHA-1 {h}): not in the set's list; ignored",
                    d.len()
                ));
            }
        }
        let [maincpu, ssiocpu, gfx1, gfx2] = regions;
        Ok(Self {
            name,
            identity: identity(chips.iter().map(|c| c.sha1)),
            maincpu,
            ssiocpu,
            gfx1,
            gfx2,
            extras,
            ssio_prom,
        })
    }
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn le16(d: &[u8], at: usize) -> Result<usize, String> {
    d.get(at..at + 2)
        .map(|b| usize::from(u16::from_le_bytes([b[0], b[1]])))
        .ok_or_else(|| "truncated".to_string())
}

fn le32(d: &[u8], at: usize) -> Result<usize, String> {
    d.get(at..at + 4)
        .map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]) as usize)
        .ok_or_else(|| "truncated".to_string())
}

/// The files of a zip: (name, contents). Stored and deflated entries only,
/// which is what MAME romsets use.
pub fn read_zip(d: &[u8]) -> Result<Vec<(String, Vec<u8>)>, String> {
    // The end of central directory record, searched for from the end.
    let eocd = (0..=d.len().saturating_sub(22))
        .rev()
        .find(|&i| d[i..].starts_with(b"PK\x05\x06"))
        .ok_or("no end of central directory")?;
    let count = le16(d, eocd + 10)?;
    let mut at = le32(d, eocd + 16)?;
    let mut out = Vec::with_capacity(count);
    for _ in 0..count {
        if !d.get(at..).is_some_and(|s| s.starts_with(b"PK\x01\x02")) {
            return Err("bad central directory entry".into());
        }
        let method = le16(d, at + 10)?;
        let csize = le32(d, at + 20)?;
        let usize_ = le32(d, at + 24)?;
        let nlen = le16(d, at + 28)?;
        let elen = le16(d, at + 30)?;
        let clen = le16(d, at + 32)?;
        let local = le32(d, at + 42)?;
        let name = String::from_utf8_lossy(d.get(at + 46..at + 46 + nlen).ok_or("truncated")?)
            .into_owned();
        at += 46 + nlen + elen + clen;
        if name.ends_with('/') {
            continue;
        }
        if !d.get(local..).is_some_and(|s| s.starts_with(b"PK\x03\x04")) {
            return Err(format!("{name}: bad local header"));
        }
        let start = local + 30 + le16(d, local + 26)? + le16(d, local + 28)?;
        let raw = d.get(start..start + csize).ok_or("truncated entry")?;
        let bytes = match method {
            0 => raw.to_vec(),
            8 => {
                let mut v = Vec::with_capacity(usize_);
                flate2::read::DeflateDecoder::new(raw)
                    .read_to_end(&mut v)
                    .map_err(|e| format!("{name}: {e}"))?;
                v
            }
            m => return Err(format!("{name}: compression method {m}")),
        };
        if bytes.len() != usize_ {
            return Err(format!(
                "{name}: {} bytes, the directory says {usize_}",
                bytes.len()
            ));
        }
        out.push((name, bytes));
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fake(chips: &[Chip]) -> Vec<(String, Vec<u8>)> {
        chips
            .iter()
            .map(|c| (c.name.to_string(), vec![0u8; c.size]))
            .collect()
    }

    #[test]
    fn a_wrong_chip_is_named() {
        let err = Romset::from_files("rbtapper", &RBTAPPER, fake(&RBTAPPER)).unwrap_err();
        assert!(
            matches!(
                err,
                RomError::Hash {
                    chip: "rbtpg0.bin",
                    ..
                }
            ),
            "{err}"
        );
        let mut files = fake(&RBTAPPER);
        files.remove(0);
        let err = Romset::from_files("rbtapper", &RBTAPPER, files).unwrap_err();
        assert!(
            matches!(
                err,
                RomError::Missing {
                    chip: "rbtpg0.bin",
                    ..
                }
            ),
            "{err}"
        );
    }

    #[test]
    fn checks_against_a_made_up_list_and_reports_extras() {
        // A two-chip "set" whose hashes are those of the made-up contents.
        let a = vec![1u8; 0x1000];
        let b = vec![2u8; 0x4000];
        let ha: &'static str = Box::leak(hex(&crate::sha1::sha1(&a)).into_boxed_str());
        let hb: &'static str = Box::leak(hex(&crate::sha1::sha1(&b)).into_boxed_str());
        let chips = [
            chip("a", Region::SsioCpu, 0x1000, 0x1000, ha),
            chip("b", Region::Gfx1, 0x4000, 0x4000, hb),
        ];
        let files = vec![
            ("renamed".to_string(), b.clone()),
            ("a".to_string(), a),
            ("junk".to_string(), vec![9u8; 7]),
        ];
        let set = Romset::from_files("test", &chips, files).unwrap();
        assert_eq!(set.ssiocpu[0x1000], 1);
        assert_eq!(set.ssiocpu[0x0fff], 0);
        assert_eq!(set.gfx1[0x4000], 2);
        assert_eq!(set.extras.len(), 1);
        assert!(set.extras[0].starts_with("junk (7 bytes"));
    }

    #[test]
    fn identity_is_the_sorted_sha1s_and_names_do_not_matter() {
        // Each set's identity is distinct, and order-free.
        let ids: std::collections::BTreeSet<String> = SETS.iter().map(SetInfo::identity).collect();
        assert_eq!(ids.len(), SETS.len());
        assert_eq!(identity(["b", "a"]), identity(["a", "b"]));
        // The SHA-256 of "a\nb\n".
        assert_eq!(
            identity(["b", "a"]),
            "911169ddaaf146aff539f58c26c489af3b892dff0fe283c1c264c65ae5aa59a2"
        );
        // No set lists the board's PROM.
        for s in &SETS {
            assert!(
                s.chips.iter().all(|c| c.sha1 != SSIO_PROM.sha1),
                "{}",
                s.name
            );
            assert_eq!(s.chips.len(), 18);
        }
    }

    #[test]
    fn a_zip_of_made_up_chips_matches_no_set_and_says_the_nearest() {
        let err = Romset::identify(fake(&RBTAPPER)).unwrap_err();
        assert!(matches!(err, RomError::Unknown { have: 0, .. }), "{err}");
        assert!(err.to_string().contains("rbtapper, tapper"), "{err}");
    }

    #[test]
    fn reads_a_stored_zip() {
        // A zip with one stored file, built by hand.
        let body = b"hello";
        let mut z = Vec::new();
        z.extend_from_slice(b"PK\x03\x04");
        z.extend_from_slice(&[20, 0, 0, 0, 0, 0, 0, 0, 0, 0]);
        z.extend_from_slice(&0u32.to_le_bytes()); // crc, unchecked here
        z.extend_from_slice(&5u32.to_le_bytes());
        z.extend_from_slice(&5u32.to_le_bytes());
        z.extend_from_slice(&1u16.to_le_bytes());
        z.extend_from_slice(&0u16.to_le_bytes());
        z.push(b'x');
        z.extend_from_slice(body);
        let cd = z.len();
        z.extend_from_slice(b"PK\x01\x02");
        z.extend_from_slice(&[20, 0, 20, 0, 0, 0, 0, 0, 0, 0, 0, 0]);
        z.extend_from_slice(&0u32.to_le_bytes());
        z.extend_from_slice(&5u32.to_le_bytes());
        z.extend_from_slice(&5u32.to_le_bytes());
        z.extend_from_slice(&1u16.to_le_bytes());
        z.extend_from_slice(&[0; 12]);
        z.extend_from_slice(&0u32.to_le_bytes());
        z.push(b'x');
        let cdlen = z.len() - cd;
        z.extend_from_slice(b"PK\x05\x06");
        z.extend_from_slice(&[0, 0, 0, 0, 1, 0, 1, 0]);
        z.extend_from_slice(&(cdlen as u32).to_le_bytes());
        z.extend_from_slice(&(cd as u32).to_le_bytes());
        z.extend_from_slice(&[0, 0]);
        let files = read_zip(&z).unwrap();
        assert_eq!(files, vec![("x".to_string(), body.to_vec())]);
    }
}
