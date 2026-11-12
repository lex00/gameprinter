import { describe, expect, it } from "vitest";
import v1 from "./fixtures/mosshop-trace-v1.json";
import v2 from "./fixtures/mosshop-trace-v2.json";
import { DecodeError, lz4Decompress } from "../src/decode/lz4";
import { Reader } from "../src/decode/reader";
import { chunkHeader, decodeChunk, splitState, type FieldDef } from "../src/decode/trace";
import { heldNames, parseInputLog } from "../src/decode/input";
import { fromHex } from "../src/state";

interface Fixture {
  chunk_base64: string;
  chunk_frames: number;
  state_version: number;
  fields: FieldDef[];
  frames: Record<string, Record<string, string>>;
}
// One chunk per field table version: states split by each recording's own
// table, which the trace stores as an object and `gp trace fields` serves.
const fixtures = [v1, v2] as unknown as Fixture[];
const bytesOf = (f: Fixture) => Uint8Array.from(atob(f.chunk_base64), (c) => c.charCodeAt(0));
const chunk = bytesOf(fixtures[0]);

describe("lz4", () => {
  it("decodes literals only", () => {
    // Token 0x50: five literals, no match.
    expect(Array.from(lz4Decompress(Uint8Array.from([0x50, 1, 2, 3, 4, 5]), 5))).toEqual([1, 2, 3, 4, 5]);
  });
  it("decodes an overlapping match", () => {
    // One literal, then a match of 4+3 at offset 1, then one literal.
    const src = Uint8Array.from([0x13, 9, 1, 0, 0x10, 7]);
    expect(Array.from(lz4Decompress(src, 9))).toEqual([9, 9, 9, 9, 9, 9, 9, 9, 7]);
  });
  it("decodes long literal runs", () => {
    const lits = Array.from({ length: 300 }, (_, i) => i & 255);
    const src = Uint8Array.from([0xf0, 255, 300 - 15 - 255, ...lits]);
    expect(Array.from(lz4Decompress(src, 300))).toEqual(lits);
  });
  it("refuses damaged blocks", () => {
    expect(() => lz4Decompress(Uint8Array.from([0x50, 1, 2]), 5)).toThrow(DecodeError);
    expect(() => lz4Decompress(Uint8Array.from([0x10, 1, 5, 0, 0x10, 2]), 7)).toThrow(DecodeError);
    expect(() => lz4Decompress(Uint8Array.from([0x20, 1, 2]), 3)).toThrow(DecodeError);
  });
});

describe("reader", () => {
  it("reads LEB128 and little-endian integers", () => {
    const r = new Reader(Uint8Array.from([0xe5, 0x8e, 0x26, 0x78, 0x56, 0x34, 0x12, 1, 0, 0, 0, 0, 0, 0, 0]));
    expect(r.varint("v")).toBe(624485);
    expect(r.u32("u")).toBe(0x12345678);
    expect(r.u64("w")).toBe(1);
    expect(() => r.u8("past")).toThrow(DecodeError);
  });
});

describe.each(fixtures.map((f) => [f.state_version, f] as const))("trace chunks, field table version %i", (version, fixture) => {
  const chunk = bytesOf(fixture);
  it("reads the header", () => {
    const h = chunkHeader(chunk);
    expect(h.firstFrame).toBe(64);
    expect(h.frames).toBe(fixture.chunk_frames);
    expect(h.fields).toMatch(/^[0-9a-f]{64}$/);
  });

  it("decodes every frame as gp trace frame shows it", () => {
    const { header, states } = decodeChunk(chunk);
    expect(states).toHaveLength(header.frames);
    // The state header names the table version: "GPNS", then a u32.
    expect(states[0][4]).toBe(version);
    for (const [frame, want] of Object.entries(fixture.frames)) {
      const fields = splitState(states[Number(frame) - header.firstFrame], fixture.fields);
      expect(Array.from(fields.keys())).toEqual(fixture.fields.map((f) => f.name));
      for (const [name, hex] of Object.entries(want)) {
        expect(Array.from(fields.get(name)!), `${name} at frame ${frame}`).toEqual(Array.from(fromHex(hex)));
      }
    }
  });
});

describe("a damaged chunk", () => {
  it("is refused", () => {
    const bad = chunk.slice();
    bad[0] = 0x58;
    expect(() => chunkHeader(bad)).toThrow(DecodeError);
    expect(() => decodeChunk(chunk.subarray(0, chunk.length - 3))).toThrow(DecodeError);
    const long = chunk.slice();
    long[52] += 1; // the body length
    expect(() => decodeChunk(long)).toThrow(DecodeError);
  });
  it("doesn't split under another version's table", () => {
    const { states } = decodeChunk(bytesOf(fixtures[0]));
    expect(() => splitState(states[0], fixtures[1].fields)).toThrow(DecodeError);
  });
});

describe("input logs", () => {
  it("parses frames and names held buttons", () => {
    const log = parseInputLog("gp-input-log 0\ncontrollers 2\n- ........ ........\nP R.....BA ........\n");
    expect(log).toHaveLength(2);
    expect(log[0].polled).toBe(false);
    expect(heldNames(log[1].pads[0])).toBe("Right+B+A");
    expect(heldNames(log[0].pads[0])).toBe("none");
    expect(() => parseInputLog("nope")).toThrow();
    const reset = parseInputLog("gp-input-log 1\ncontrollers 1\n-! ........\nP ....T...\n");
    expect(reset[0]).toMatchObject({ polled: false, reset: true });
    expect(reset[1]).toMatchObject({ polled: true, reset: false });
  });
});
