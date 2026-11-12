// OAM: the 64 sprites, drawn as boxes where they sit on the 256x240 picture,
// and listed. Sprites at Y $EF or below the picture are parked off screen.

import type { Pane } from "../pane";
import { hex2 } from "../state";
import { sprites } from "./ppu";
import { registerView, type ViewProps } from "./registry";

function Screen({ pane }: { pane: Pane }) {
  const list = sprites(pane.state?.get("oam"));
  const tall = ((pane.state?.get("ppu.ctrl")?.[0] ?? 0) & 0x20) !== 0;
  return (
    <div className="oam">
      <div className="pane-name">{pane.name}</div>
      <svg viewBox="0 0 256 240" className="screen" data-testid={`oam-${pane.id.slice(0, 8)}`}>
        <rect width="256" height="240" className="screen-bg" />
        {list
          .filter((s) => !s.hidden)
          .map((s) => (
            <g key={s.index}>
              <rect x={s.x} y={s.y + 1} width="8" height={tall ? 16 : 8} className={s.index === 0 ? "sprite0" : "sprite"} />
              <title>{`sprite ${s.index}: tile $${hex2(s.tile)} at ${s.x},${s.y} attr $${hex2(s.attr)}`}</title>
            </g>
          ))}
      </svg>
      <div className="hint">{list.filter((s) => !s.hidden).length} of 64 sprites on screen{tall ? ", 8x16" : ""}</div>
    </div>
  );
}

function Oam({ a, b }: ViewProps) {
  return (
    <div className="side-by-side">
      <Screen pane={a} />
      {b && <Screen pane={b} />}
    </div>
  );
}

registerView({ id: "oam", title: "OAM", order: 30, component: Oam });
