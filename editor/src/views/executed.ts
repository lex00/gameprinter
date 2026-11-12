// The arithmetic behind the coverage view: what `/api/recordings/{id}/coverage`
// (the JSON of `gp coverage <id> --no-save --json`) and
// `/api/recordings/{id}/listing` say, turned into per-line and per-page
// counts of executed bytes (docs/coverage.md).

/** A set of bytes, as gp-coverage serializes one: its length and its runs. */
export interface BitsJson {
  len: number;
  ranges: [number, number][];
}

export interface CoverageJson {
  sha256: string;
  mapper: number;
  frames: number;
  executed: BitsJson;
  starts: BitsJson;
  windows: number[][];
  ram_written: BitsJson;
  screens: Record<string, unknown>;
  levels: Record<string, unknown>;
}

export interface SummaryJson {
  recordings: number;
  frames: number;
  executed_bytes: number;
  executed_percent: number;
  prg_rom_bytes: number;
  bank_size: number;
  ram_written_bytes: number;
  ram_bytes: number;
  screens: number;
  levels: number;
  objects: number;
  input_combinations: number;
}

export interface ReportJson {
  groups: { summary: SummaryJson; coverage: CoverageJson; definition?: string }[];
  recordings: { id: string; name: string; result: string; message?: string }[];
}

export interface ListingJson {
  game: string | null;
  missing: string | null;
  listing: { files: { name: string; lines: string[] }[]; spans: [number, number, number, number][] } | null;
}

/** One byte per offset: 1 where the set has it. */
export function toBytes(bits: BitsJson): Uint8Array {
  const out = new Uint8Array(bits.len);
  for (const [start, end] of bits.ranges) out.fill(1, start, Math.min(end, bits.len));
  return out;
}

/** A source line's PRG ROM bytes and how many of them ran. */
export interface LineCount {
  bytes: number;
  executed: number;
  /** The line's first PRG ROM offset. */
  offset: number;
}

/**
 * Per file, per line number, the line's bytes within PRG ROM and how many
 * ran. Spans past PRG ROM (CHR, say) are left out.
 */
export function lineCounts(listing: NonNullable<ListingJson["listing"]>, executed: Uint8Array): Map<number, LineCount>[] {
  const out = listing.files.map(() => new Map<number, LineCount>());
  for (const [file, line, offset, size] of listing.spans) {
    const map = out[file];
    if (!map) continue;
    for (let o = offset; o < offset + size && o < executed.length; o++) {
      const c = map.get(line) ?? { bytes: 0, executed: 0, offset: o };
      c.bytes++;
      c.executed += executed[o];
      c.offset = Math.min(c.offset, o);
      map.set(line, c);
    }
  }
  return out;
}

export type LineState = "none" | "some" | "all";

export function lineState(c: LineCount): LineState {
  return c.executed === 0 ? "none" : c.executed === c.bytes ? "all" : "some";
}

/**
 * Where a CPU address runs from: its PRG ROM offset, by the pages coverage
 * saw mapped there. When more than one PRG page ran at that address (a
 * bank-switching board), which one is mapped now isn't known, so there's
 * no answer.
 */
export function offsetOfPc(pc: number, windows: number[][]): number | undefined {
  const page = pc & 0xfc00;
  const pages = windows.flatMap((w, i) => (w.includes(page) ? [i] : []));
  return pages.length === 1 ? pages[0] * 1024 + (pc & 0x3ff) : undefined;
}

/** A row of the bank view: one 1KB page of PRG ROM, in cells of `cell` bytes. */
export interface PageRow {
  page: number;
  bank: number;
  /** The CPU addresses the page ran at. */
  at: number[];
  /** Executed bytes in each cell. */
  cells: number[];
  executed: number;
}

export function pageRows(executed: Uint8Array, windows: number[][], bankSize: number, cell = 64): PageRow[] {
  const rows: PageRow[] = [];
  for (let page = 0; page * 1024 < executed.length; page++) {
    const cells: number[] = [];
    let total = 0;
    for (let c = 0; c < 1024 / cell; c++) {
      let n = 0;
      for (let o = page * 1024 + c * cell; o < page * 1024 + (c + 1) * cell && o < executed.length; o++) n += executed[o];
      cells.push(n);
      total += n;
    }
    rows.push({ page, bank: Math.floor((page * 1024) / bankSize), at: windows[page] ?? [], cells, executed: total });
  }
  return rows;
}
