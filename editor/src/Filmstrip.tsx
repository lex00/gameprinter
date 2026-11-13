// The save state filmstrip (docs/show.md, "The filmstrip"): one card per
// save point, grouped by session and branch with the newest at the end,
// then one card per clip the tool made, grouped by level, each start's
// picture on its clips' cards. The filters show clips by state and by
// where they came from: played by you, or made by the tool.
// Only the cards in view are drawn and their pictures load lazily, so a
// strip of hundreds stays quick. Clicking a card loads it (F8 on it); the
// arrow keys step through the strip while it has focus and Enter opens (a clip) or loads
// the one chosen. A card with a clip shows a small mark with the clip's
// state; clicking the mark opens the clip.

import { useEffect, useLayoutEffect, useMemo, useRef, useState } from "react";
import { clock } from "./freefold";
import { FILTERS, SOURCES, type Filter, type SourceFilter, type StripClip, type StripItem, clipState, offsets, scrollFor, stepTo, visibleRange, widthOf } from "./stripfold";

export interface FilmstripProps {
  items: StripItem[];
  /** The chosen save point: F8 loads it. */
  current?: string;
  /** The save point play last loaded or saved. */
  playing?: string;
  picture: (i: StripItem & { kind: "save" }) => string;
  choose: (key: string) => void;
  load: (key: string) => void;
  openClip: (c: StripClip) => void;
  filter: Filter;
  setFilter: (f: Filter) => void;
  /** Which clips by where they came from. */
  source?: SourceFilter;
  setSource?: (s: SourceFilter) => void;
  /** The picture at a start the tool found. */
  madePicture?: (i: StripItem & { kind: "made" }) => string;
  /** Save points in all, before the filter. */
  total: number;
  /** Clips the tool made, in all. */
  made?: number;
  /** A level name for a save point when one is known. */
  level?: (i: StripItem & { kind: "save" }) => string | undefined;
}

const MARK_WORDS: Record<ReturnType<typeof clipState>, string> = {
  waiting: "clip",
  waits: "waits",
  matches: "matches",
  "not matching": "differs",
  fixed: "fixed",
  skipped: "skipped",
  "no start": "can't start",
};

export function Filmstrip({ items, current, playing, picture, choose, load, openClip, filter, setFilter, source = "all", setSource, madePicture, total, made = 0, level }: FilmstripProps) {
  const box = useRef<HTMLDivElement>(null);
  const [left, setLeft] = useState(0);
  const [width, setWidth] = useState(1200);
  const offs = useMemo(() => offsets(items), [items]);
  const [from, to] = visibleRange(offs, left, width);
  const stuck = useRef(true);

  useLayoutEffect(() => {
    const el = box.current;
    if (!el) return;
    setWidth(el.clientWidth || 1200);
    if (typeof ResizeObserver === "undefined") return;
    const ro = new ResizeObserver(() => setWidth(el.clientWidth || 1200));
    ro.observe(el);
    return () => ro.disconnect();
  }, []);

  // The newest is at the end: the strip stays there while it was there.
  useEffect(() => {
    const el = box.current;
    if (!el || !stuck.current) return;
    el.scrollLeft = Math.max(0, offs[offs.length - 1] - el.clientWidth);
    setLeft(el.scrollLeft);
  }, [offs]);

  // The chosen card comes into view.
  useEffect(() => {
    const el = box.current;
    const i = items.findIndex((x) => x.key === current);
    if (!el || i < 0) return;
    const l = scrollFor(offs, i, el.scrollLeft, el.clientWidth || width);
    if (l !== el.scrollLeft) {
      el.scrollLeft = l;
      setLeft(l);
    }
  }, [current]); // eslint-disable-line react-hooks/exhaustive-deps

  const onKey = (e: React.KeyboardEvent) => {
    let to: string | undefined;
    if (e.key === "ArrowRight") to = stepTo(items, current, e.shiftKey ? 10 : 1);
    else if (e.key === "ArrowLeft") to = stepTo(items, current, e.shiftKey ? -10 : -1);
    else if (e.key === "Home") to = stepTo(items, items.find((i) => i.kind === "save")?.key, 0);
    else if (e.key === "End") to = stepTo(items, undefined, 0);
    else if (e.key === "Enter" && current) {
      e.preventDefault();
      const it = items.find((i) => i.key === current);
      if (it?.kind !== "group" && it?.clip) openClip(it.clip);
      else if (it?.kind === "save") load(current);
      return;
    } else return;
    e.preventDefault();
    e.stopPropagation();
    if (to) choose(to);
  };

  return (
    <section className="filmstrip" data-testid="filmstrip">
      <div className="strip-filters" role="group" aria-label="show">
        {FILTERS.map((f) => (
          <button key={f.id} className={f.id === filter ? "on" : undefined} onClick={() => setFilter(f.id)} data-testid={`strip-filter-${f.id}`}>
            {f.label}
          </button>
        ))}
        {setSource && (
          <span className="strip-sources" role="group" aria-label="made by">
            {SOURCES.map((f) => (
              <button key={f.id} className={f.id === source ? "on" : undefined} onClick={() => setSource(f.id)} data-testid={`strip-source-${f.id}`}>
                {f.label}
              </button>
            ))}
          </span>
        )}
        <span className="hint" data-testid="strip-count">
          {filter === "all" && source === "all" ? `${total} save ${total === 1 ? "state" : "states"}` : `${items.filter((i) => i.kind === "save").length} of ${total} save states`}
          {made > 0 && `, ${items.filter((i) => i.kind === "made" && i.clip).length} of ${made} clips made by the tool`}. Click one to load it; arrow keys step through them here.
        </span>
      </div>
      <div
        ref={box}
        className="strip-scroll"
        tabIndex={0}
        onKeyDown={onKey}
        onScroll={(e) => {
          const el = e.currentTarget;
          setLeft(el.scrollLeft);
          stuck.current = el.scrollLeft + el.clientWidth >= el.scrollWidth - 4;
        }}
        data-testid="strip-scroll"
      >
        <div className="strip-inner" style={{ width: offs[offs.length - 1] }} data-testid="strip-inner" data-drawn={to - from}>
          {items.slice(from, to).map((it, k) => {
            const i = from + k;
            const style = { left: offs[i], width: widthOf(it) };
            if (it.kind === "group") {
              return (
                <div key={it.key} className="strip-group" style={style} title={it.label} data-testid="strip-group">
                  <span>{it.label}</span>
                </div>
              );
            }
            if (it.kind === "made") {
              const c = it.clip;
              const st = c ? clipState(c) : undefined;
              const what = c?.name?.split(": ").slice(1).join(": ") ?? it.start.name;
              return (
                <div key={it.key} className={`strip-card made${it.key === current ? " current" : ""}`} style={style} data-testid="strip-made" data-key={it.key}>
                  <button className="strip-pick" onClick={() => (choose(it.key), c && openClip(c))} disabled={!c} title={c ? `${c.name}: made by the tool from ${it.start.name}. Open it to watch it and fix it.` : `${it.start.name}: the rebuilt game can't start here: ${it.start.cant ?? "no clips"}`}>
                    <img src={madePicture?.(it) ?? ""} loading="lazy" width={128} height={120} alt={it.start.name} />
                    <span>{c ? what : it.start.name}</span>
                  </button>
                  {c && st ? (
                    <button className={`clip-mark ${st.replace(" ", "-")}${c.mark === "starred" ? " starred" : ""}`} onClick={() => openClip(c)} title={c.score?.says ?? (st === "waits" ? "waits for the clips it needs to match first" : "made by the tool")} data-testid="clip-mark">
                      {c.mark === "starred" ? "* " : ""}
                      {MARK_WORDS[st]}
                    </button>
                  ) : (
                    <span className="clip-mark no-start" title={it.start.cant ?? undefined} data-testid="made-cant">
                      can't start
                    </span>
                  )}
                </div>
              );
            }
            const st = it.clip ? clipState(it.clip) : undefined;
            const lv = level?.(it);
            return (
              <div key={it.key} className={`strip-card${it.key === current ? " current" : ""}${it.key === playing ? " playing" : ""}`} style={style} data-testid="strip-card" data-key={it.key}>
                {/* A card with a clip opens the clip (to watch and fix it); one
                    without loads the save state to play on from it. The user,
                    2026-10-29: "i was clicking it to load it into the loop". */}
                <button className="strip-pick" onClick={() => (choose(it.key), it.clip ? openClip(it.clip) : load(it.key))} title={it.clip ? "open this clip: watch it and fix it" : "load this save state and play on from it"}>
                  <img src={picture(it)} loading="lazy" width={128} height={120} alt={`save state ${it.save.id}`} />
                  <span>
                    {clock(it.save.frame)}
                    {lv ? `, ${lv}` : ""}
                  </span>
                </button>
                {it.clip && st && (
                  <button className={`clip-mark ${st.replace(" ", "-")}${it.clip.mark === "starred" ? " starred" : ""}`} onClick={() => openClip(it.clip!)} title={it.clip.score?.says ?? "played by you: open the clip from here"} data-testid="clip-mark">
                    {it.clip.mark === "starred" ? "* " : ""}
                    {MARK_WORDS[st]}
                  </button>
                )}
                {it.save.native && !it.save.native.matches && <span className="error">the model made it differently</span>}
              </div>
            );
          })}
        </div>
      </div>
    </section>
  );
}
