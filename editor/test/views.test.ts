import { describe, expect, it } from "vitest";
import { activity, activityColor, addressAt, valueColor } from "../src/views/heat";
import { formatBytes, labelBytes, labelIndex } from "../src/views/labels";
import { nametableTiles, scroll, sprites } from "../src/views/ppu";
import { countsByKind, framePosition } from "../src/views/lanes";
import { differingFields } from "../src/views/DiffFields";
import { differingRam, eventWindow } from "../src/pane";
import { int } from "../src/state";
import { readUrl, writeUrl } from "../src/App";

describe("heatmap", () => {
  it("counts changes per byte", () => {
    const s = [Uint8Array.from([0, 0, 1]), Uint8Array.from([1, 0, 1]), Uint8Array.from([2, 0, 0])];
    expect(Array.from(activity(s))).toEqual([2, 0, 1]);
    expect(activity([]).length).toBe(0);
  });
  it("maps colours and positions", () => {
    expect(valueColor(0)).not.toBe(valueColor(255));
    expect(activityColor(0, 64)).toBe(valueColor(0));
    expect(addressAt(0, 0, 7)).toBe(0);
    expect(addressAt(7 * 63 + 1, 7 * 31 + 1, 7)).toBe(2047);
    expect(addressAt(7 * 64, 0, 7)).toBeUndefined();
  });
});

describe("labels", () => {
  const labels = [
    { name: "X", addr: 0x10, size: 1, description: "" },
    { name: "ENEMY", addr: 0x20, size: 2, description: "" },
  ];
  it("indexes every byte a label covers", () => {
    const idx = labelIndex(labels);
    expect(idx.get(0x21)?.[0].name).toBe("ENEMY");
    expect(idx.get(0x22)).toBeUndefined();
  });
  it("reads and formats values", () => {
    const ram = new Uint8Array(2048);
    ram[0x10] = 0x2a;
    ram[0x21] = 0xff;
    expect(formatBytes(labelBytes(ram, labels[0]))).toBe("$2a (42)");
    expect(formatBytes(labelBytes(ram, labels[1]))).toBe("00 ff");
    expect(formatBytes(labelBytes(undefined, labels[0]))).toBe("-");
  });
});

describe("ppu", () => {
  it("reads sprites", () => {
    const oam = new Uint8Array(256).fill(0xff);
    oam.set([0x40, 0x01, 0x02, 0x80], 0);
    const s = sprites(oam);
    expect(s).toHaveLength(64);
    expect(s[0]).toMatchObject({ y: 0x40, tile: 1, attr: 2, x: 0x80, hidden: false });
    expect(s[1].hidden).toBe(true);
  });
  it("splits nametables and finds the scroll", () => {
    expect(nametableTiles(new Uint8Array(2048), 1)?.length).toBe(960);
    expect(nametableTiles(new Uint8Array(1024), 1)).toBeUndefined();
    // Nametable 1, coarse X 3, coarse Y 2, fine Y 5, fine X 6.
    const t = (5 << 12) | (1 << 10) | (2 << 5) | 3;
    expect(scroll(t, 6)).toEqual({ x: 256 + 24 + 6, y: 21 });
  });
});

describe("event lanes", () => {
  it("places events from vblank", () => {
    const e = (scanline: number, dot: number) => ({ frame: 1, scanline, dot, kind: "nmi", addr: 0, value: 0 });
    expect(framePosition(e(241, 1))).toBe(0);
    expect(framePosition(e(0, 0))).toBeGreaterThan(framePosition(e(250, 0)));
    expect(framePosition(e(241, 0))).toBeGreaterThan(0.99);
  });
  it("counts per kind and frame", () => {
    const ev = [
      { frame: 3, scanline: 0, dot: 0, kind: "nmi", addr: 0, value: 0 },
      { frame: 3, scanline: 0, dot: 9, kind: "nmi", addr: 0, value: 0 },
      { frame: 9, scanline: 0, dot: 0, kind: "nmi", addr: 0, value: 0 },
    ];
    expect(countsByKind(ev, 2, 5).get("nmi")).toEqual([0, 2, 0]);
  });
  it("windows events by 64 frames, never frame 0", () => {
    expect(eventWindow(0, 300)).toEqual([1, 64]);
    expect(eventWindow(130, 300)).toEqual([128, 192]);
    expect(eventWindow(300, 300)).toEqual([256, 301]);
  });
});

describe("diff", () => {
  it("finds differing fields and RAM bytes", () => {
    const a = new Map([
      ["ram", Uint8Array.from([1, 2, 3])],
      ["cpu.a", Uint8Array.from([7])],
    ]);
    const b = new Map([
      ["ram", Uint8Array.from([1, 9, 4])],
      ["cpu.a", Uint8Array.from([7])],
    ]);
    expect(differingFields(a, b)).toEqual([{ field: "ram", bytes: 2 }]);
    expect(Array.from(differingRam(a, b))).toEqual([1, 2]);
    expect(int(a, "cpu.a")).toBe(7);
  });
});

describe("url state", () => {
  it("round-trips the selection", () => {
    const s = { a: "abcd", b: "ef01", frame: 12 };
    expect(readUrl(writeUrl(s))).toEqual(s);
    expect(readUrl("?frame=-3").frame).toBe(0);
  });
});
