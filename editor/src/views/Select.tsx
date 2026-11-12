// Box selection: drag a box on the picture to see what's under it at this
// frame. The server answers from the recording's state: sprites from OAM,
// and, when a model of the game has a state mapping, the entities whose
// hitboxes the box touches, with their model paths, values and the RAM
// bytes (and labels) that hold them (docs/inspector.md, "Box selection").
// The box stays put as the frame changes, so stepping shows what passes
// through it.

import { useEffect, useRef, useState } from "react";
import type { Box, SelectJson } from "../api";
import { setShared } from "../shared";
import { hex2 } from "../state";
import { sprites } from "./ppu";
import { registerView, type ViewProps } from "./registry";
import { dragBox, SCREEN_H, SCREEN_W, toPicture } from "./boxsel";

function hex4(n: number): string {
  return n.toString(16).padStart(4, "0");
}

/** The selection's contents, as lists. Exported for the tests. */
export function SelectionResult({ result }: { result: SelectJson }) {
  return (
    <div className="selection" data-testid="selection">
      <div className="hint">
        {`${result.rect.w}x${result.rect.h} at ${result.rect.x},${result.rect.y}`}
        {result.world && result.camera ? `; world ${result.world.x},${result.world.y} (${result.camera.path} at ${result.camera.x ?? "?"},${result.camera.y ?? 0})` : ""}
      </div>
      <h3>Entities</h3>
      {result.entities.length === 0 ? (
        <p className="hint">{result.missing ?? "No mapped entity's hitbox is under the box."}</p>
      ) : (
        <ul className="entities">
          {result.entities.map((e) => (
            <li key={e.path}>
              <code>{e.path}</code> ({e.kind}){" "}
              <button className="link" onClick={() => setShared({ cardPath: e.path })}>
                card
              </button>{" "}
              <span className="hint">{e.box.y === null ? `x ${e.box.x}..${e.box.x + e.box.w}, y not mapped` : `${e.box.w}x${e.box.h} at ${e.box.x},${e.box.y}`}</span>
              <table>
                <tbody>
                  {e.fields.map((f) => (
                    <tr key={f.path}>
                      <td>
                        <code>{f.path}</code>
                      </td>
                      <td>{f.value}</td>
                      <td className="hint">{f.ram.map((a) => `$${hex4(a)}`).join(" ")}</td>
                      <td className="hint">{f.note ?? ""}</td>
                    </tr>
                  ))}
                </tbody>
              </table>
            </li>
          ))}
        </ul>
      )}
      {result.ram.length > 0 && (
        <>
          <h3>RAM</h3>
          <table className="ram-under">
            <tbody>
              {result.ram.map((r) => (
                <tr key={r.addr}>
                  <td>
                    <code>${hex4(r.addr)}</code>
                  </td>
                  <td>${hex2(r.value)}</td>
                  <td>{r.labels.join(", ")}</td>
                  <td className="hint">{r.paths.join(", ")}</td>
                </tr>
              ))}
            </tbody>
          </table>
        </>
      )}
      <h3>Sprites</h3>
      {result.sprites.length === 0 ? (
        <p className="hint">No sprite is under the box.</p>
      ) : (
        <p>
          {result.sprites.map((s) => (
            <span key={s.index} className="sprite-chip">
              {`sprite ${s.index} (OAM $${hex2(s.oam)}): tile $${hex2(s.tile)}, attr $${hex2(s.attr)} at ${s.box.x},${s.box.y}`}
            </span>
          ))}
        </p>
      )}
    </div>
  );
}

function Select({ api, a, frame }: ViewProps) {
  const [box, setBox] = useState<Box>();
  const [drag, setDrag] = useState<{ x: number; y: number }>();
  const [live, setLive] = useState<Box>();
  const [result, setResult] = useState<SelectJson>();
  const [error, setError] = useState<string>();
  const svg = useRef<SVGSVGElement>(null);

  useEffect(() => {
    setError(undefined);
    if (!box) {
      setResult(undefined);
      return;
    }
    const abort = new AbortController();
    api
      .select(a.id, frame, box, abort.signal)
      .then((r) => setResult(r))
      .catch((e: Error) => {
        if (!abort.signal.aborted) setError(e.message);
      });
    return () => abort.abort();
  }, [api, a.id, frame, box]);

  const at = (e: React.PointerEvent) => toPicture(e.clientX, e.clientY, svg.current!.getBoundingClientRect());
  const shown = live ?? box;
  const list = sprites(a.state?.get("oam")).filter((s) => !s.hidden);
  const tall = ((a.state?.get("ppu.ctrl")?.[0] ?? 0) & 0x20) !== 0;

  return (
    <div className="select-view">
      <div>
        <svg
          ref={svg}
          viewBox={`0 0 ${SCREEN_W} ${SCREEN_H}`}
          className="screen selectable"
          data-testid="select-screen"
          onPointerDown={(e) => {
            const p = at(e);
            svg.current?.setPointerCapture?.(e.pointerId);
            setDrag(p);
            setLive(dragBox(p, p));
          }}
          onPointerMove={(e) => drag && setLive(dragBox(drag, at(e)))}
          onPointerUp={(e) => {
            if (!drag) return;
            setBox(dragBox(drag, at(e)));
            setDrag(undefined);
            setLive(undefined);
          }}
        >
          <rect width={SCREEN_W} height={SCREEN_H} className="screen-bg" />
          {list.map((s) => (
            <rect key={s.index} x={s.x} y={s.y + 1} width="8" height={tall ? 16 : 8} className="sprite" />
          ))}
          {result?.entities.map((e) => (
            <rect
              key={e.path}
              x={e.box.x}
              y={e.box.y ?? 0}
              width={e.box.w}
              height={e.box.h ?? SCREEN_H}
              className={e.box.y === null ? "entity-box unknown-y" : "entity-box"}
            />
          ))}
          {shown && <rect x={shown.x} y={shown.y} width={shown.w} height={shown.h} className="select-box" />}
        </svg>
        <div className="hint">
          Drag a box on the picture.{" "}
          {box && (
            <button className="link" onClick={() => setBox(undefined)}>
              clear
            </button>
          )}
        </div>
      </div>
      <div>
        {error && <p className="error">{error}</p>}
        {result ? <SelectionResult result={result} /> : !error && <p className="hint">Nothing selected.</p>}
      </div>
    </div>
  );
}

registerView({ id: "select", title: "Box selection", order: 25, wide: true, component: Select });
