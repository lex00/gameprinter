// Frame trace chunks (docs/formats/recording.md, "Frame trace"): a 56-byte
// header, then one LZ4 block holding the first frame's state in full and,
// for each later frame, the byte ranges that changed. `gp serve` hands the
// browser chunks byte for byte; the server's frames endpoint is the checked
// path, since this decoder doesn't hash states against the frame hashes.

import { DecodeError, lz4Decompress } from "./lz4";
import { Reader } from "./reader";

export const TRACE_MAGIC = "GPTR";
export const TRACE_SCHEMA = 1;
const HEADER = 56;

export interface ChunkHeader {
  firstFrame: number;
  frames: number;
  /** The field table's BLAKE3 hash, as hex. */
  fields: string;
  bodyLength: number;
}

/** A field as `gp trace fields --json` lists it. */
export interface FieldDef {
  name: string;
  kind: "fixed" | "region";
  width: number | null;
}

const ascii = (b: Uint8Array) => String.fromCharCode(...b);
const hex = (b: Uint8Array) => Array.from(b, (x) => x.toString(16).padStart(2, "0")).join("");

export function chunkHeader(bytes: Uint8Array): ChunkHeader {
  const r = new Reader(bytes);
  if (ascii(r.take(4, "magic")) !== TRACE_MAGIC) throw new DecodeError("not a trace chunk");
  const schema = r.u32("schema");
  if (schema !== TRACE_SCHEMA) throw new DecodeError(`trace chunk schema ${schema}`);
  const firstFrame = r.u64("first frame");
  const frames = r.u32("frame count");
  if (frames === 0) throw new DecodeError("a chunk with no frames");
  const fields = hex(r.take(32, "field table hash"));
  const bodyLength = r.u32("body length");
  return { firstFrame, frames, fields, bodyLength };
}

/** Every state in a chunk, in frame order, each a full copy. */
export function decodeChunk(bytes: Uint8Array): { header: ChunkHeader; states: Uint8Array[] } {
  const header = chunkHeader(bytes);
  const body = lz4Decompress(bytes.subarray(HEADER), header.bodyLength);
  const r = new Reader(body);
  let state = r.take(r.varint("keyframe length"), "keyframe").slice();
  const states = [state];
  for (let i = 1; i < header.frames; i++) {
    state = state.slice();
    const ranges = r.varint("range count");
    let at = 0;
    for (let k = 0; k < ranges; k++) {
      at += r.varint("skip");
      const n = r.varint("range length");
      if (at + n > state.length) throw new DecodeError("a range runs past the state");
      state.set(r.take(n, "range"), at);
      at += n;
    }
    states.push(state);
  }
  if (r.rest !== 0) throw new DecodeError(`${r.rest} bytes after the last frame`);
  return { header, states };
}

/** Splits a state into named fields: an 8-byte header, then the fields. */
export function splitState(state: Uint8Array, table: FieldDef[]): Map<string, Uint8Array> {
  const r = new Reader(state);
  r.take(8, "state header");
  const out = new Map<string, Uint8Array>();
  for (const f of table) {
    const n = f.kind === "region" ? r.u32(`${f.name} length`) : (f.width ?? 0);
    out.set(f.name, r.take(n, f.name));
  }
  if (r.rest !== 0) throw new DecodeError(`${r.rest} bytes after the last field`);
  return out;
}
