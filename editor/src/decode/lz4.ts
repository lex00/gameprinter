// LZ4 block decompression: the block format with no frame and no size
// prefix, as gp-evidence writes trace and event bodies with lz4_flex
// (docs/formats/recording.md). The caller knows the decompressed length
// from the chunk header.

export class DecodeError extends Error {}

/** Decompresses one LZ4 block of `outLen` bytes. */
export function lz4Decompress(src: Uint8Array, outLen: number): Uint8Array {
  const out = new Uint8Array(outLen);
  let s = 0;
  let d = 0;
  const byte = (): number => {
    if (s >= src.length) throw new DecodeError("LZ4 block ends early");
    return src[s++];
  };
  const length = (start: number): number => {
    let n = start;
    if (n === 15) {
      let b: number;
      do {
        b = byte();
        n += b;
      } while (b === 255);
    }
    return n;
  };
  while (s < src.length) {
    const token = byte();
    const literals = length(token >>> 4);
    if (s + literals > src.length || d + literals > outLen) {
      throw new DecodeError("LZ4 literals run past the block");
    }
    out.set(src.subarray(s, s + literals), d);
    s += literals;
    d += literals;
    // The last sequence is literals only.
    if (s === src.length) break;
    const offset = byte() | (byte() << 8);
    if (offset === 0 || offset > d) throw new DecodeError(`LZ4 match offset ${offset} at ${d}`);
    const match = length(token & 15) + 4;
    if (d + match > outLen) throw new DecodeError("LZ4 match runs past the output");
    // Byte by byte: a match may overlap the bytes it's copying.
    for (let i = 0; i < match; i++, d++) out[d] = out[d - offset];
  }
  if (d !== outLen) throw new DecodeError(`LZ4 block holds ${d} bytes, not ${outLen}`);
  return out;
}
