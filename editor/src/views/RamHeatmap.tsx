// RAM heatmap: the console's 2KB of internal RAM, 64 bytes a row, coloured
// by value or by how often each byte changed over the trace chunk holding
// the frame. Bytes that changed since the frame before are outlined, and in
// diff mode, bytes where the two recordings differ.

import { useEffect, useRef, useState } from "react";
import { decodeChunk, splitState } from "../decode/trace";
import { differingRam, type Pane } from "../pane";
import { hex2, hex4 } from "../state";
import { activity, activityColor, addressAt, COLUMNS, type HeatMode, valueColor } from "./heat";
import { labelIndex } from "./labels";
import { registerView, type ViewProps } from "./registry";

const CELL = 7;

function useActivity(props: ViewProps, pane: Pane, on: boolean): Uint16Array | undefined | string {
  const [result, setResult] = useState<Uint16Array | string>();
  const trace = pane.manifest.trace;
  const chunk = trace ? Math.floor(props.frame / trace.chunk_frames) : -1;
  useEffect(() => {
    setResult(undefined);
    if (!on) return;
    if (!trace) {
      setResult("this recording has no trace, so activity can't be shown");
      return;
    }
    let live = true;
    props.api
      .chunk(pane.id, chunk)
      .then((bytes) => {
        const { states } = decodeChunk(bytes);
        const ram = states.map((s) => splitState(s, pane.fields).get("ram") ?? new Uint8Array());
        if (live) setResult(activity(ram));
      })
      .catch((e: Error) => live && setResult(e.message));
    return () => {
      live = false;
    };
  }, [on, pane.id, chunk, trace, pane.fields, props.api]);
  return on ? result : undefined;
}

function Grid({ props, pane, other, mode }: { props: ViewProps; pane: Pane; other?: Pane; mode: HeatMode }) {
  const canvas = useRef<HTMLCanvasElement>(null);
  const [hover, setHover] = useState<number>();
  const act = useActivity(props, pane, mode === "activity");
  const ram = pane.state?.get("ram");
  const prev = pane.prev?.get("ram");
  const differs = other ? differingRam(pane.state, other.state) : new Set<number>();
  const labels = labelIndex(pane.labels.labels);
  const chunkFrames = pane.manifest.trace?.chunk_frames ?? 64;

  useEffect(() => {
    const ctx = canvas.current?.getContext("2d");
    if (!ctx || !ram) return;
    ctx.clearRect(0, 0, COLUMNS * CELL, (2048 / COLUMNS) * CELL);
    for (let a = 0; a < Math.min(ram.length, 2048); a++) {
      const x = (a % COLUMNS) * CELL;
      const y = Math.floor(a / COLUMNS) * CELL;
      ctx.fillStyle =
        mode === "activity" && act instanceof Uint16Array ? activityColor(act[a], chunkFrames) : valueColor(ram[a]);
      ctx.fillRect(x, y, CELL - 1, CELL - 1);
      if (differs.has(a)) {
        ctx.strokeStyle = "#ff4d6d";
        ctx.strokeRect(x + 0.5, y + 0.5, CELL - 2, CELL - 2);
      } else if (prev && prev[a] !== ram[a]) {
        ctx.strokeStyle = "#f5f5f5";
        ctx.strokeRect(x + 0.5, y + 0.5, CELL - 2, CELL - 2);
      }
    }
  });

  const tip =
    hover !== undefined && ram
      ? `$${hex4(hover)} = $${hex2(ram[hover])}` +
        (labels.get(hover) ? `  ${labels.get(hover)!.map((l) => l.name).join(", ")}` : "") +
        (act instanceof Uint16Array ? `  changed ${act[hover]}x in this chunk` : "") +
        (other?.state?.get("ram") ? `  other: $${hex2(other.state.get("ram")![hover])}` : "")
      : "hover a byte";
  return (
    <div className="heatmap">
      <div className="pane-name">{pane.name}</div>
      <canvas
        ref={canvas}
        width={COLUMNS * CELL}
        height={(2048 / COLUMNS) * CELL}
        data-testid={`heatmap-${pane.id.slice(0, 8)}`}
        onMouseMove={(e) => {
          const r = e.currentTarget.getBoundingClientRect();
          setHover(addressAt(e.clientX - r.left, e.clientY - r.top, CELL));
        }}
        onMouseLeave={() => setHover(undefined)}
      />
      <div className="tip">{typeof act === "string" ? act : mode === "activity" && !act ? "decoding the chunk..." : tip}</div>
    </div>
  );
}

function RamHeatmap(props: ViewProps) {
  const [mode, setMode] = useState<HeatMode>("value");
  return (
    <div>
      <div className="toolbar">
        <label>
          <input type="radio" checked={mode === "value"} onChange={() => setMode("value")} /> value
        </label>
        <label>
          <input type="radio" checked={mode === "activity"} onChange={() => setMode("activity")} /> changes over the
          chunk
        </label>
        <span className="hint">white outline: changed since the frame before{props.b ? "; red: differs from the other recording" : ""}</span>
      </div>
      <div className="side-by-side">
        <Grid props={props} pane={props.a} other={props.b} mode={mode} />
        {props.b && <Grid props={props} pane={props.b} other={props.a} mode={mode} />}
      </div>
    </div>
  );
}

registerView({ id: "ram-heatmap", title: "RAM heatmap", order: 10, wide: true, component: RamHeatmap });
