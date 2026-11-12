//! Which lifted tile a sprite shows: the pattern table the PPU reads,
//! through the cartridge's banks (CHR ROM) or as the game wrote it (CHR
//! RAM), matched to the lifted `chr-2bpp` assets. A CHR ROM tile is found
//! by its place in the CHR data, a CHR RAM tile by its pixels, so a game
//! that copies tiles from PRG (Mega Man 2) is found as well as one that
//! switches CHR banks (Super Mario Bros. 2).

use std::collections::HashMap;
use std::path::Path;

use gp_assets::map::{Asset, AssetMap, Region};
use gp_model::Model;
use gp_model::model::AssetKind;

/// One tile's pixels, 0 to 3, row by row.
pub type Pixels = [u8; 64];

/// A lifted tiles asset.
pub struct TileAsset {
    pub name: String,
    /// The model's tiles slot filled from it, if it has one.
    pub slot: Option<String>,
    pub region: Region,
    pub offset: u32,
    pub tiles: Vec<Pixels>,
}

/// Every lifted tiles asset of a source, indexed by place and by pixels.
#[derive(Default)]
pub struct TileBank {
    pub assets: Vec<TileAsset>,
    by_pixels: HashMap<Pixels, (usize, u32)>,
}

/// A tile: an index into [`TileBank::assets`] and a tile number in it.
pub type TileRef = (usize, u32);

/// Pixels from a tile's 16 bytes of bit planes.
pub fn pixels(bytes: &[u8]) -> Pixels {
    let mut px = [0u8; 64];
    for y in 0..8 {
        let (p0, p1) = (bytes[y], bytes[y + 8]);
        for x in 0..8 {
            let bit = 7 - x;
            px[y * 8 + x] = ((p0 >> bit) & 1) | (((p1 >> bit) & 1) << 1);
        }
    }
    px
}

impl TileBank {
    /// Reads the tiles assets of a lifted directory. Tiles of the model's
    /// own slots are preferred when two assets hold the same pixels.
    pub fn load(dir: &Path, map: &AssetMap, model: &Model) -> Result<Self, String> {
        let mut bank = TileBank::default();
        for a in &map.assets {
            let Asset::Chr {
                name,
                region,
                offset,
                tiles,
                tiles_per_row,
                ..
            } = a
            else {
                continue;
            };
            let file = dir.join(a.file());
            let Ok(png) = std::fs::read(&file) else {
                continue;
            };
            let size = gp_assets::chr::image_size(*tiles, *tiles_per_row);
            let px = gp_assets::chr::read_png(&png, size)
                .map_err(|e| format!("{}: {e}", file.display()))?;
            let mut out = Vec::with_capacity(*tiles as usize);
            for t in 0..*tiles {
                let (tx, ty) = ((t % tiles_per_row) * 8, (t / tiles_per_row) * 8);
                let mut p = [0u8; 64];
                for y in 0..8 {
                    for x in 0..8 {
                        p[(y * 8 + x) as usize] = px[((ty + y) * size.0 + tx + x) as usize];
                    }
                }
                out.push(p);
            }
            let slot = model
                .assets
                .iter()
                .find(|(_, s)| {
                    s.kind == AssetKind::Tiles && s.from.as_ref().is_some_and(|f| &f.asset == name)
                })
                .map(|(n, _)| n.clone());
            bank.assets.push(TileAsset {
                name: name.clone(),
                slot,
                region: *region,
                offset: *offset,
                tiles: out,
            });
        }
        bank.index();
        Ok(bank)
    }

    /// A bank from tiles in memory, for tests.
    pub fn from_assets(assets: Vec<TileAsset>) -> Self {
        let mut bank = TileBank {
            assets,
            by_pixels: HashMap::new(),
        };
        bank.index();
        bank
    }

    fn index(&mut self) {
        let mut order: Vec<usize> = (0..self.assets.len()).collect();
        order.sort_by_key(|&i| self.assets[i].slot.is_none());
        for i in order {
            for (t, px) in self.assets[i].tiles.iter().enumerate() {
                self.by_pixels.entry(*px).or_insert((i, t as u32));
            }
        }
    }

    pub fn pixels(&self, t: TileRef) -> &Pixels {
        &self.assets[t.0].tiles[t.1 as usize]
    }

    pub fn is_blank(&self, t: TileRef) -> bool {
        self.pixels(t).iter().all(|&p| p == 0)
    }

    /// The tile at CHR byte `offset`, from an asset of the CHR region.
    pub fn at_chr(&self, offset: u32) -> Option<TileRef> {
        self.assets.iter().enumerate().find_map(|(i, a)| {
            (a.region == Region::Chr
                && offset >= a.offset
                && offset < a.offset + a.tiles.len() as u32 * 16
                && (offset - a.offset).is_multiple_of(16))
            .then(|| (i, (offset - a.offset) / 16))
        })
    }

    pub fn by_pixels(&self, px: &Pixels) -> Option<TileRef> {
        self.by_pixels.get(px).copied()
    }
}

/// What a frame's PPU reads its pattern tables through.
#[derive(Debug, Clone, Default)]
pub struct Banks<'a> {
    /// CHR RAM, 8KB, when the cartridge has it.
    pub chr_ram: Option<&'a [u8]>,
    /// The iNES mapper number.
    pub mapper: u16,
    /// MMC1: control, chr0, chr1.
    pub mmc1: [u8; 3],
    /// MMC3: bank select, then R0 to R7.
    pub mmc3: [u8; 9],
    /// CNROM: the 8KB CHR bank.
    pub cnrom: u8,
}

impl Banks<'_> {
    /// The CHR ROM byte a PPU address `0x0000..0x2000` reads.
    pub fn chr_offset(&self, addr: u16) -> u32 {
        let a = u32::from(addr & 0x1FFF);
        match self.mapper {
            1 => {
                let [control, chr0, chr1] = self.mmc1.map(u32::from);
                if control & 0x10 == 0 {
                    (chr0 & 0x1E) * 0x1000 + a
                } else if a < 0x1000 {
                    chr0 * 0x1000 + a
                } else {
                    chr1 * 0x1000 + (a - 0x1000)
                }
            }
            4 => {
                let r = |i: usize| u32::from(self.mmc3[1 + i]);
                // With bank select bit 7 the two halves swap.
                let a2 = if self.mmc3[0] & 0x80 != 0 {
                    a ^ 0x1000
                } else {
                    a
                };
                match a2 {
                    0x0000..=0x07FF => (r(0) & 0xFE) * 0x400 + a2,
                    0x0800..=0x0FFF => (r(1) & 0xFE) * 0x400 + (a2 - 0x0800),
                    _ => {
                        let k = ((a2 - 0x1000) / 0x400) as usize;
                        r(2 + k) * 0x400 + (a2 % 0x400)
                    }
                }
            }
            3 => u32::from(self.cnrom) * 0x2000 + a,
            _ => a,
        }
    }

    /// The lifted tile at PPU address `addr` (a tile's first byte).
    pub fn tile(&self, bank: &TileBank, addr: u16) -> Option<TileRef> {
        match self.chr_ram {
            Some(ram) => {
                let at = usize::from(addr & 0x1FF0);
                ram.get(at..at + 16)
                    .and_then(|b| bank.by_pixels(&pixels(b)))
            }
            None => bank.at_chr(self.chr_offset(addr)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn asset(name: &str, region: Region, offset: u32, tiles: Vec<Pixels>) -> TileAsset {
        TileAsset {
            name: name.into(),
            slot: None,
            region,
            offset,
            tiles,
        }
    }

    #[test]
    fn chr_rom_tiles_by_bank_and_chr_ram_tiles_by_pixels() {
        let mut a = [0u8; 64];
        a[0] = 1;
        let mut b = [0u8; 64];
        b[63] = 3;
        let bank = TileBank::from_assets(vec![
            asset("low", Region::Chr, 0, vec![[0; 64]; 64]),
            asset("high", Region::Chr, 0x400, vec![a, b]),
            asset("prg_copy", Region::Prg, 0x9000, vec![b]),
        ]);
        // No mapper: the address is the CHR offset.
        let flat = Banks::default();
        assert_eq!(flat.tile(&bank, 0x0410), Some((1, 1)));
        // MMC3: R2 (the first 1KB at $1000) is bank 1, so $1010 is CHR
        // $0410; with bank select bit 7 the halves swap.
        let mut mmc3 = Banks {
            mapper: 4,
            ..Banks::default()
        };
        mmc3.mmc3[1 + 2] = 1;
        assert_eq!(mmc3.chr_offset(0x1010), 0x410);
        mmc3.mmc3[0] = 0x80;
        assert_eq!(mmc3.chr_offset(0x0010), 0x410);
        // MMC1 in 4KB mode: chr1 picks the upper table.
        let mmc1 = Banks {
            mapper: 1,
            mmc1: [0x10, 0, 2],
            ..Banks::default()
        };
        assert_eq!(mmc1.chr_offset(0x1000), 0x2000);
        // CNROM: one 8KB bank for both tables.
        let cnrom = Banks {
            mapper: 3,
            cnrom: 1,
            ..Banks::default()
        };
        assert_eq!(cnrom.chr_offset(0x1010), 0x3010);
        // CHR RAM: the tile is found by its pixels.
        let mut ram = vec![0u8; 0x2000];
        ram[0x20 + 7] = 0x01;
        ram[0x20 + 15] = 0x01;
        let banks = Banks {
            chr_ram: Some(&ram),
            ..Banks::default()
        };
        assert_eq!(banks.tile(&bank, 0x20), Some((1, 1)));
        assert!(bank.is_blank((0, 5)));
    }
}
