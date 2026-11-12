// Tiles: the recording's game's lifted tile sets, drawn in its colors. The
// lifted PNG keeps color indices; this view applies a palette as it draws,
// either one of the frame's palette RAM (the colors the game had loaded at
// this frame) or one from a lifted palettes file (docs/inspector.md,
// "Tiles"). The tiles come from `$GP_HOME/work/assets/<game>/`, which
// `gp assets lift` writes.

import { useEffect, useMemo, useRef, useState } from "react";
import type { AssetsJson, TileSetJson } from "../api";
import { hex2 } from "../state";
import { registerView, type ViewProps } from "./registry";
import { paletteChoices, parseColors, tileAt, tileRgba } from "./tilecolors";

function Tiles({ api, a }: ViewProps) {
  const game = a.labels.game;
  const [assets, setAssets] = useState<AssetsJson>();
  const [colors, setColors] = useState<[number, number, number][]>();
  const [setName, setSetName] = useState<string>();
  const [tileSet, setTileSet] = useState<TileSetJson>();
  const [paletteKey, setPaletteKey] = useState<string>("ram:0");
  const [error, setError] = useState<string>();
  const [hover, setHover] = useState<number>();
  const canvas = useRef<HTMLCanvasElement>(null);

  useEffect(() => {
    setAssets(undefined);
    setError(undefined);
    if (!game) return;
    let live = true;
    Promise.all([api.assets(game), api.colors()])
      .then(([as, c]) => {
        if (!live) return;
        setAssets(as);
        setColors(parseColors(c.colors));
        setSetName((n) => (n && as.tiles.some((t) => t.name === n) ? n : as.tiles[0]?.name));
      })
      .catch((e: Error) => live && setError(e.message));
    return () => {
      live = false;
    };
  }, [api, game]);

  useEffect(() => {
    setTileSet(undefined);
    setError(undefined);
    if (!game || !setName) return;
    let live = true;
    api
      .tileSet(game, setName)
      .then((t) => live && setTileSet(t))
      .catch((e: Error) => live && setError(e.message));
    return () => {
      live = false;
    };
  }, [api, game, setName]);

  const palettes = useMemo(() => paletteChoices(a.state?.get("palette"), assets?.palettes ?? []), [a.state, assets]);
  const palette = palettes.find((p) => p.key === paletteKey) ?? palettes[0];

  useEffect(() => {
    const c = canvas.current;
    if (!c || !tileSet || !colors || !palette) return;
    const ctx = c.getContext("2d");
    if (!ctx) return;
    const rgba = tileRgba(tileSet.pixels, tileSet.width, tileSet.height, palette.colors, colors);
    ctx.putImageData(new ImageData(rgba as Uint8ClampedArray<ArrayBuffer>, tileSet.width, tileSet.height), 0, 0);
  }, [tileSet, colors, palette]);

  if (!game) return <p className="hint">The recording's game isn't known, so it has no lifted tiles.</p>;
  if (error && !assets) return <p className="error">{error}</p>;
  if (!assets) return <p className="hint">loading</p>;
  if (assets.missing) return <p className="hint">{assets.missing}</p>;
  if (assets.tiles.length === 0) return <p className="hint">{game}'s lift has no tile sets.</p>;
  return (
    <div className="tiles-view">
      <div className="controls">
        <label>
          tiles{" "}
          <select value={setName} onChange={(e) => setSetName(e.target.value)} data-testid="tiles-set">
            {assets.tiles.map((t) => (
              <option key={t.name} value={t.name}>
                {t.name} ({t.tiles})
              </option>
            ))}
          </select>
        </label>{" "}
        <label>
          palette{" "}
          <select value={palette?.key} onChange={(e) => setPaletteKey(e.target.value)} data-testid="tiles-palette">
            {palettes.map((p) => (
              <option key={p.key} value={p.key}>
                {p.label}
              </option>
            ))}
          </select>
        </label>{" "}
        {palette && (
          <span className="swatches">
            {palette.colors.map((c, i) => (
              <span key={i} className="swatch" title={`$${hex2(c)}`} style={{ background: colors ? `rgb(${colors[c].join(",")})` : undefined }} />
            ))}
          </span>
        )}
      </div>
      {error && <p className="error">{error}</p>}
      {tileSet && (
        <canvas
          ref={canvas}
          width={tileSet.width}
          height={tileSet.height}
          className="tiles"
          style={{ width: tileSet.width * 2 }}
          data-testid="tiles-canvas"
          onMouseMove={(e) => {
            const r = e.currentTarget.getBoundingClientRect();
            const x = ((e.clientX - r.left) / r.width) * tileSet.width;
            const y = ((e.clientY - r.top) / r.height) * tileSet.height;
            setHover(tileAt(x, y, tileSet.tiles_per_row, tileSet.tiles));
          }}
          onMouseLeave={() => setHover(undefined)}
        />
      )}
      <div className="hint">
        {hover !== undefined ? `tile $${hex2(hover)} (${hover})` : `${tileSet?.tiles ?? "?"} tiles from ${assets.dir}`}
      </div>
    </div>
  );
}

registerView({ id: "tiles", title: "Tiles", order: 55, wide: true, component: Tiles });
