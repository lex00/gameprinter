// Drawing lifted tiles in the game's colors. A lifted tiles PNG keeps color
// indices 0 to 3 (docs/assets.md, "Tiles"); the colors come from a palette
// chosen when the tiles are drawn: one of the frame's palette RAM, or one
// from a lifted palettes file. Nothing here changes the lifted files.

import type { AssetsJson } from "../api";

/** A palette to draw with: four NES color numbers and where they came from. */
export interface PaletteChoice {
  key: string;
  label: string;
  colors: number[];
}

/** The 64 NES colors, from `/api/nes/colors`'s hex, as RGB triples. */
export function parseColors(hex: string[]): [number, number, number][] {
  return hex.map((h) => [parseInt(h.slice(0, 2), 16), parseInt(h.slice(2, 4), 16), parseInt(h.slice(4, 6), 16)]);
}

/**
 * The palettes to offer: the eight in the frame's palette RAM, then each
 * lifted palette. In palette RAM every palette's color 0 is drawn as the
 * shared backdrop, $3F00, as the PPU draws it.
 */
export function paletteChoices(paletteRam: Uint8Array | undefined, lifted: AssetsJson["palettes"]): PaletteChoice[] {
  const out: PaletteChoice[] = [];
  if (paletteRam && paletteRam.length >= 32) {
    for (let p = 0; p < 8; p++) {
      const c = Array.from(paletteRam.subarray(p * 4, p * 4 + 4), (v) => v & 0x3f);
      c[0] = paletteRam[0] & 0x3f;
      out.push({ key: `ram:${p}`, label: `this frame: ${p < 4 ? "background" : "sprites"} ${p % 4}`, colors: c });
    }
  }
  for (const set of lifted) {
    set.palettes.forEach((colors, i) => {
      out.push({ key: `lifted:${set.name}:${i}`, label: `${set.name} ${i}`, colors: colors.map((v) => v & 0x3f) });
    });
  }
  return out;
}

/** RGBA pixels for a tile set's indices (one digit a pixel) through four NES colors. */
export function tileRgba(pixels: string, width: number, height: number, palette: number[], colors: [number, number, number][]): Uint8ClampedArray {
  const out = new Uint8ClampedArray(width * height * 4);
  const rgb = [0, 1, 2, 3].map((i) => colors[palette[i] ?? 0x0f] ?? [0, 0, 0]);
  for (let i = 0; i < width * height; i++) {
    const c = rgb[(pixels.charCodeAt(i) - 48) & 3];
    out[i * 4] = c[0];
    out[i * 4 + 1] = c[1];
    out[i * 4 + 2] = c[2];
    out[i * 4 + 3] = 255;
  }
  return out;
}

/** The tile under a point of a tile set drawn at one pixel a pixel. */
export function tileAt(x: number, y: number, tilesPerRow: number, tiles: number): number | undefined {
  const t = Math.floor(y / 8) * tilesPerRow + Math.floor(x / 8);
  return x >= 0 && y >= 0 && Math.floor(x / 8) < tilesPerRow && t < tiles ? t : undefined;
}
