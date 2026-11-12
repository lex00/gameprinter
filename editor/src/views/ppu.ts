// OAM and nametable arithmetic for their views.

export interface Sprite {
  index: number;
  y: number;
  tile: number;
  attr: number;
  x: number;
  /** Below the visible picture: a sprite parked off screen. */
  hidden: boolean;
}

/** The 64 sprites in OAM, four bytes each: Y, tile, attributes, X. */
export function sprites(oam: Uint8Array | undefined): Sprite[] {
  if (!oam) return [];
  const out: Sprite[] = [];
  for (let i = 0; i + 3 < oam.length && i < 256; i += 4) {
    const y = oam[i];
    out.push({ index: i / 4, y, tile: oam[i + 1], attr: oam[i + 2], x: oam[i + 3], hidden: y >= 0xef });
  }
  return out;
}

/** One 1KB nametable: 30 rows of 32 tile indices, then 64 attribute bytes. */
export function nametableTiles(ram: Uint8Array | undefined, table: number): Uint8Array | undefined {
  if (!ram || ram.length < (table + 1) * 1024) return undefined;
  return ram.subarray(table * 1024, table * 1024 + 960);
}

/**
 * The scroll position from the PPU's `t` and fine `x` registers, in pixels
 * over the 512x480 space of four nametables: coarse X in bits 0-4, coarse
 * Y in bits 5-9, the nametable in bits 10-11, fine Y in bits 12-14.
 */
export function scroll(t: number | undefined, fineX: number | undefined): { x: number; y: number } | undefined {
  if (t === undefined) return undefined;
  const coarseX = t & 0x1f;
  const coarseY = (t >> 5) & 0x1f;
  const nt = (t >> 10) & 3;
  const fineY = (t >> 12) & 7;
  return {
    x: (nt & 1) * 256 + coarseX * 8 + (fineX ?? 0),
    y: (nt >> 1) * 240 + coarseY * 8 + fineY,
  };
}
