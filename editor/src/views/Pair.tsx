// The original and a model run side by side (docs/inspector.md, "Original
// and model side by side"): the original's picture with its mapped
// entities, the model's entities at the matching model frame, and each
// mapped path's two values with the ones that differ marked. Pick the
// model run in the header; a run made with `gp record --model DIR
// --polled-of ID` lines up with the recording it came from.

import { useEffect, useState } from "react";
import type { EntityBox, PairFrameJson } from "../api";
import { setShared } from "../shared";
import { SCREEN_H, SCREEN_W } from "./boxsel";
import { differingFrames, differingInstances, instanceOf, nextDifference, styled } from "./pairing";
import { sprites } from "./ppu";
import { registerView, type ViewProps } from "./registry";

function Boxes({ boxes, differs, side }: { boxes: EntityBox[]; differs: Set<string>; side: string }) {
  return (
    <>
      {styled(boxes, differs).map(({ box, differs }) => (
        <rect
          key={box.path}
          x={box.box.x}
          y={box.box.y ?? 0}
          width={box.box.w}
          height={box.box.h ?? SCREEN_H}
          className={`entity-box${box.box.y === null ? " unknown-y" : ""}${differs ? " differs" : ""}`}
          data-testid={`${side}-entity`}
          onClick={() => setShared({ cardPath: box.path })}
        >
          <title>{`${box.path} (${box.kind})${differs ? ": a mapped path differs" : ""}`}</title>
        </rect>
      ))}
    </>
  );
}

/** The two sides' pictures and the path table for one pair frame. Exported for the tests. */
export function PairFrame({ data, oam, tall }: { data: PairFrameJson; oam?: Uint8Array; tall?: boolean }) {
  const differs = differingInstances(data.paths);
  const list = sprites(oam).filter((s) => !s.hidden);
  return (
    <div className="pair-frame" data-testid="pair-frame">
      <div className="side-by-side">
        <div>
          <div className="pane-name">original, frame {data.original.frame}</div>
          <svg viewBox={`0 0 ${SCREEN_W} ${SCREEN_H}`} className="screen" data-testid="pair-original">
            <rect width={SCREEN_W} height={SCREEN_H} className="screen-bg" />
            {list.map((s) => (
              <rect key={s.index} x={s.x} y={s.y + 1} width="8" height={tall ? 16 : 8} className="sprite" />
            ))}
            <Boxes boxes={data.original_entities} differs={differs} side="original" />
          </svg>
        </div>
        <div>
          <div className="pane-name">model, frame {data.model.frame}</div>
          <svg viewBox={`0 0 ${SCREEN_W} ${SCREEN_H}`} className="screen" data-testid="pair-model">
            <rect width={SCREEN_W} height={SCREEN_H} className="screen-bg" />
            <Boxes boxes={data.model_entities} differs={differs} side="model" />
          </svg>
        </div>
      </div>
      <p className="hint">
        {data.differing === 0 ? "Every mapped path agrees at this frame." : `${data.differing} of ${data.paths.length} mapped paths differ at this frame.`}
        {data.aligned ? "" : ` Frame ${data.asked} is a lag frame; this is frame ${data.original.frame}.`}
      </p>
      <table className="paths" data-testid="pair-paths">
        <thead>
          <tr>
            <th>path</th>
            <th>original</th>
            <th>model</th>
            <th>RAM</th>
          </tr>
        </thead>
        <tbody>
          {data.paths.map((r) => (
            <tr key={r.path} className={r.differs ? "differs" : ""}>
              <td>
                <button className="link" onClick={() => setShared({ cardPath: instanceOf(r.path) ?? r.path.split(".").slice(0, 2).join(".") })}>
                  {r.path}
                </button>
              </td>
              <td>{r.original}</td>
              <td>{r.model ?? "not in the model"}</td>
              <td className="hint">{r.labels.join(", ")}</td>
            </tr>
          ))}
        </tbody>
      </table>
    </div>
  );
}

function Pair({ api, frame, a, m, pair, setFrame }: ViewProps) {
  const [data, setData] = useState<PairFrameJson>();
  const [error, setError] = useState<string>();
  useEffect(() => {
    setError(undefined);
    if (!m || !pair) {
      setData(undefined);
      return;
    }
    const abort = new AbortController();
    api
      .pairFrame(a.id, m.id, frame, abort.signal)
      .then(setData)
      .catch((e: Error) => {
        if (!abort.signal.aborted) setError(e.message);
      });
    return () => abort.abort();
  }, [api, a.id, m?.id, frame, pair]);

  if (!m) return <p className="hint">Pick a model run in the header. `gp record --model DIR --polled-of ID` makes one that lines up with this recording.</p>;
  if (!pair) return <p className="hint">Lining the runs up.</p>;
  const tall = ((a.state?.get("ppu.ctrl")?.[0] ?? 0) & 0x20) !== 0;
  const next = nextDifference(pair.differing, frame);
  return (
    <div className="pair-view">
      <p className="hint" data-testid="pair-summary">
        {pair.alignment === "polled" ? "Lined up by the mapping's polled alignment" : "Lined up frame for frame"}
        {pair.input_matches ? "" : `; the model's input differs from the original's from model frame ${pair.first_input_mismatch}`}
        {pair.differing.length === 0
          ? "; every mapped path agrees on every frame."
          : `; some path differs on ${differingFrames(pair.differing)} frames in ${pair.differing.length} stretches.`}{" "}
        {next !== undefined && (
          <button className="link" onClick={() => setFrame(next)}>
            next difference (frame {next})
          </button>
        )}
      </p>
      {error && <p className="error">{error}</p>}
      {data && <PairFrame data={data} oam={a.state?.get("oam")} tall={tall} />}
    </div>
  );
}

registerView({ id: "pair", title: "Original and model", order: 22, wide: true, component: Pair });
