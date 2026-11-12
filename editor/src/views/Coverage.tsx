// Coverage: which PRG ROM bytes the recording ran, drawn over the game's
// source listing (the disassembly for Super Mario Bros., the source for
// Moss Hop) when the server has one, and by PRG ROM page and bank when it
// doesn't. A line is green when all its bytes ran, amber when some did and
// red when none did. The line holding the current frame's program counter
// is marked (docs/coverage.md).

import { useEffect, useLayoutEffect, useMemo, useRef, useState } from "react";
import type { Pane } from "../pane";
import { hex4, int } from "../state";
import {
  lineCounts,
  lineState,
  offsetOfPc,
  pageRows,
  toBytes,
  type CoverageJson,
  type ListingJson,
  type ReportJson,
} from "./executed";
import { registerView, type ViewProps } from "./registry";

const ROW = 16;
const VISIBLE = 28;

type Loaded = { report: ReportJson; listing: ListingJson } | string;

/**
 * Coverage and listing by recording, kept for the page once they're
 * complete: the server doesn't store what it measures, so fetching again
 * would replay again. A failed fetch, a recording that wasn't measured
 * (its ROM was missing, say) and a missing listing aren't kept, so fixing
 * the cause and coming back to the recording tries again.
 */
const fetched = new Map<string, Promise<{ report: ReportJson; listing: ListingJson }>>();

function load(props: ViewProps, id: string) {
  let p = fetched.get(id);
  if (!p) {
    // The listing failing doesn't hide coverage that loaded.
    const listing = props.api
      .json<ListingJson>(`/api/recordings/${id}/listing`)
      .catch((e: Error): ListingJson => ({ game: null, missing: e.message, listing: null }));
    p = Promise.all([props.api.json<ReportJson>(`/api/recordings/${id}/coverage`), listing]).then(([report, listing]) => ({ report, listing }));
    fetched.set(id, p);
    p.then(
      (got) => {
        if (got.report.groups.length === 0 || !got.listing.listing) fetched.delete(id);
      },
      () => fetched.delete(id),
    );
  }
  return p;
}

function useCoverage(props: ViewProps, pane: Pane): Loaded | undefined {
  const [loaded, setLoaded] = useState<Loaded>();
  useEffect(() => {
    setLoaded(undefined);
    let live = true;
    load(props, pane.id)
      .then((got) => live && setLoaded(got))
      .catch((e: Error) => live && setLoaded(e.message));
    return () => {
      live = false;
    };
  }, [pane.id, props.api]);
  return loaded;
}

type Filter = "all" | "code" | "unrun";

export function Listing({ listing, cov, pc }: { listing: NonNullable<ListingJson["listing"]>; cov: CoverageJson; pc?: number }) {
  const executed = useMemo(() => toBytes(cov.executed), [cov]);
  const counts = useMemo(() => lineCounts(listing, executed), [listing, executed]);
  const [file, setFile] = useState(0);
  const [filter, setFilter] = useState<Filter>("all");
  const [top, setTop] = useState(0);
  const [scrollTo, setScrollTo] = useState<number>();
  const box = useRef<HTMLDivElement>(null);
  // Files with no PRG ROM bytes (CHR data, say) have nothing to colour.
  const withCode = useMemo(() => listing.files.map((_, i) => i).filter((i) => (counts[i]?.size ?? 0) > 0), [listing, counts]);
  const shown = useMemo(() => {
    const lines = listing.files[file]?.lines ?? [];
    const out: number[] = [];
    for (let i = 0; i < lines.length; i++) {
      const c = counts[file]?.get(i + 1);
      if (filter === "all" || (c && (filter === "code" || c.executed < c.bytes))) out.push(i);
    }
    return out;
  }, [listing, counts, file, filter]);
  // The line the program counter is on, by the pages coverage saw it at.
  const pcOffset = pc === undefined ? undefined : offsetOfPc(pc, cov.windows);
  const pcLine = useMemo(() => {
    if (pcOffset === undefined) return undefined;
    for (const [f, line, offset, size] of listing.spans) {
      if (pcOffset >= offset && pcOffset < offset + size) return { file: f, line };
    }
    return undefined;
  }, [listing, pcOffset]);
  const first = Math.floor(top / ROW);
  const rows = shown.slice(first, first + VISIBLE + 2);
  // Scrolls once the rows it needs are laid out, so a change of file or
  // filter made at the same time can't clamp it.
  useLayoutEffect(() => {
    if (scrollTo === undefined) return;
    const row = shown.indexOf(scrollTo);
    const y = Math.max(0, (row - 5) * ROW);
    if (box.current) box.current.scrollTop = y;
    setTop(y);
    setScrollTo(undefined);
  }, [scrollTo, shown]);
  const show = (f: number, filt: Filter) => {
    setFile(f);
    setFilter(filt);
    setTop(0);
    if (box.current) box.current.scrollTop = 0;
  };
  const goToPc = () => {
    if (!pcLine) return;
    const stays = pcLine.file === file && shown.includes(pcLine.line - 1);
    if (!stays) show(pcLine.file, "all");
    setScrollTo(pcLine.line - 1);
  };
  return (
    <div>
      <div className="controls">
        {withCode.length > 1 && (
          <select value={file} onChange={(e) => show(Number(e.target.value), filter)} aria-label="file">
            {withCode.map((i) => (
              <option key={i} value={i}>
                {listing.files[i].name}
              </option>
            ))}
          </select>
        )}
        <select value={filter} onChange={(e) => show(file, e.target.value as Filter)} aria-label="lines">
          <option value="all">every line</option>
          <option value="code">lines with bytes</option>
          <option value="unrun">lines not wholly run</option>
        </select>
        <button onClick={goToPc} disabled={!pcLine}>
          {`go to PC${pc === undefined ? "" : ` $${hex4(pc)}`}`}
        </button>
      </div>
      <div
        className="listing"
        ref={box}
        style={{ height: VISIBLE * ROW }}
        onScroll={(e) => setTop((e.target as HTMLDivElement).scrollTop)}
        data-testid="coverage-listing"
      >
        <div style={{ height: shown.length * ROW, position: "relative" }}>
          {rows.map((i, k) => {
            const c = counts[file]?.get(i + 1);
            const here = pcLine?.file === file && pcLine.line === i + 1;
            return (
              <div
                key={i}
                className={`listing-line${c ? ` run-${lineState(c)}` : ""}${here ? " at-pc" : ""}`}
                style={{ top: (first + k) * ROW, height: ROW }}
              >
                <span className="listing-no">{i + 1}</span>
                <span className="listing-off">{c ? c.offset.toString(16).padStart(5, "0") : ""}</span>
                <span className="listing-text">{listing.files[file].lines[i]}</span>
              </div>
            );
          })}
        </div>
      </div>
      <div className="hint">
        {listing.files[file]?.name}: {shown.length} lines shown. Green ran, amber ran in part, red never ran; the number after the line is its PRG ROM offset.
      </div>
    </div>
  );
}

export function Banks({ cov, bankSize }: { cov: CoverageJson; bankSize: number }) {
  const rows = useMemo(() => pageRows(toBytes(cov.executed), cov.windows, bankSize), [cov, bankSize]);
  return (
    <div className="banks" data-testid="coverage-banks">
      {rows.map((r) => (
        <div key={r.page} className="bank-row">
          <span className="bank-label">
            {`bank ${r.bank} +${((r.page * 1024) % bankSize).toString(16).padStart(4, "0")}${r.at.length > 0 ? ` at $${r.at.map(hex4).join(", $")}` : ""}`}
          </span>
          {r.cells.map((n, i) => (
            <span key={i} className="bank-cell" style={{ opacity: n === 0 ? 0.12 : 0.35 + (0.65 * n) / 64 }} title={`${n} of 64 bytes ran`} />
          ))}
          <span className="bank-count">{r.executed}</span>
        </div>
      ))}
      <div className="hint">Each cell is 64 bytes of PRG ROM; brighter cells ran more of their bytes.</div>
    </div>
  );
}

function Coverage(props: ViewProps) {
  const { a } = props;
  const loaded = useCoverage(props, a);
  if (loaded === undefined) return <div className="hint">Loading coverage (a recording measured for the first time is replayed, which takes a few seconds)...</div>;
  if (typeof loaded === "string") return <div className="hint">Coverage couldn't be loaded: {loaded}</div>;
  const group = loaded.report.groups[0];
  if (!group) {
    const r = loaded.report.recordings[0];
    return (
      <div className="hint">
        No coverage for this recording ({r?.result}
        {r?.message ? `: ${r.message}` : ""}).
      </div>
    );
  }
  const s = group.summary;
  const pc = int(a.state, "cpu.pc");
  return (
    <div className="coverage" data-testid="coverage">
      <div className="pane-name">{props.b ? `${a.name} (the first recording)` : a.name}</div>
      <div className="coverage-summary">
        {s.executed_bytes} of {s.prg_rom_bytes} PRG ROM bytes ran ({s.executed_percent}%), {s.ram_written_bytes} of {s.ram_bytes} RAM bytes written,{" "}
        {group.coverage.levels && s.levels > 0 ? `${s.screens} screens in ${s.levels} levels, ` : ""}
        {s.input_combinations} input combinations.
      </div>
      {loaded.listing.listing ? (
        <Listing listing={loaded.listing.listing} cov={group.coverage} pc={pc} />
      ) : (
        <>
          <div className="hint">No source listing: {loaded.listing.missing}. Executed bytes by PRG ROM page:</div>
          <Banks cov={group.coverage} bankSize={s.bank_size} />
        </>
      )}
    </div>
  );
}

registerView({ id: "coverage", title: "Coverage", order: 70, wide: true, component: Coverage });
