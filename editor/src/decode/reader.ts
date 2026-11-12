// Little-endian integers and unsigned LEB128, as the store's formats use.

import { DecodeError } from "./lz4";

export class Reader {
  pos = 0;
  constructor(readonly bytes: Uint8Array) {}

  get rest(): number {
    return this.bytes.length - this.pos;
  }

  take(n: number, what: string): Uint8Array {
    if (n > this.rest) throw new DecodeError(`${what}: needs ${n} bytes, ${this.rest} left`);
    const out = this.bytes.subarray(this.pos, this.pos + n);
    this.pos += n;
    return out;
  }

  u8(what: string): number {
    return this.take(1, what)[0];
  }

  u32(what: string): number {
    const b = this.take(4, what);
    return (b[0] | (b[1] << 8) | (b[2] << 16) | (b[3] << 24)) >>> 0;
  }

  /** A u64 as a number; frame numbers stay far below 2^53. */
  u64(what: string): number {
    const lo = this.u32(what);
    const hi = this.u32(what);
    if (hi >= 0x200000) throw new DecodeError(`${what}: ${hi} * 2^32 is too large`);
    return hi * 0x1_0000_0000 + lo;
  }

  /** An unsigned LEB128 integer of at most 53 bits. */
  varint(what: string): number {
    let n = 0;
    let scale = 1;
    for (let i = 0; i < 8; i++) {
      const b = this.u8(what);
      n += (b & 0x7f) * scale;
      if ((b & 0x80) === 0) return n;
      scale *= 128;
    }
    throw new DecodeError(`${what}: LEB128 integer too long`);
  }
}
