// Nametables: the console's nametable RAM as grids of tile indices (the
// tiles' pixels are in CHR, which the trace holds only for CHR RAM), with
// the scroll position from the PPU's t and x registers as a frame.

import type { Pane } from "../pane";
import { int } from "../state";
import { nametableTiles, scroll } from "./ppu";
import { registerView, type ViewProps } from "./registry";
import { valueColor } from "./heat";

function Tables({ pane }: { pane: Pane }) {
  const ram = pane.state?.get("nametables");
  const count = ram ? Math.min(4, Math.floor(ram.length / 1024)) : 0;
  const mirroring = int(pane.state, "mapper.mirroring");
  // Two tables side by side for vertical mirroring (0), stacked for
  // horizontal (1); one-screen (2, 3) and four-screen (4) side by side.
  const across = mirroring === 1 ? 1 : 2;
  const named = ["vertical", "horizontal", "one-screen (lower)", "one-screen (upper)", "four-screen"][mirroring ?? 0] ?? `mirroring ${mirroring}`;
  const s = scroll(int(pane.state, "ppu.t"), int(pane.state, "ppu.x"));
  const w = across * 256;
  const h = Math.ceil(count / across) * 240;
  if (count === 0) return <p className="hint">No nametable RAM in this state.</p>;
  return (
    <div className="nametables">
      <div className="pane-name">{pane.name}</div>
      <svg viewBox={`0 0 ${w} ${h}`} className="screen" data-testid={`nametables-${pane.id.slice(0, 8)}`}>
        {Array.from({ length: count }, (_, t) => {
          const tiles = nametableTiles(ram, t)!;
          const ox = (t % across) * 256;
          const oy = Math.floor(t / across) * 240;
          return Array.from(tiles, (v, i) => (
            <rect key={`${t}-${i}`} x={ox + (i % 32) * 8} y={oy + Math.floor(i / 32) * 8} width="8" height="8" fill={valueColor(v)} />
          ));
        })}
        {s && <rect x={s.x % w} y={s.y % Math.max(h, 1)} width="256" height="240" className="scroll-box" />}
      </svg>
      <div className="hint">
        {count} tables, {named} mirroring{s ? `, scroll ${s.x},${s.y}` : ""}
      </div>
    </div>
  );
}

function Nametables({ a, b }: ViewProps) {
  return (
    <div className="side-by-side">
      <Tables pane={a} />
      {b && <Tables pane={b} />}
    </div>
  );
}

registerView({ id: "nametables", title: "Nametables", order: 40, component: Nametables });
