// Annotation arithmetic for the timeline: where each note sits over the
// recording's frames, which notes cover a frame, and tags from text.

import type { Note } from "./api";

export interface Mark {
  id: number;
  /** Percent of the timeline's width. */
  left: number;
  width: number;
  /** Which row it's drawn on, so overlapping notes don't hide each other. */
  row: number;
  title: string;
}

/** The smallest mark, in percent, so a one-frame note stays clickable. */
const MIN_WIDTH = 0.4;

/** Each note as a mark over frames 0 to `frames`, in rows so none overlap. */
export function marks(notes: Note[], frames: number): Mark[] {
  const span = Math.max(1, frames + 1);
  const ends: number[] = [];
  return [...notes]
    .sort((a, b) => a.from - b.from || a.id - b.id)
    .map((n) => {
      let row = ends.findIndex((end) => end < n.from);
      if (row < 0) row = ends.length;
      ends[row] = n.to;
      return {
        id: n.id,
        left: (n.from / span) * 100,
        width: Math.max(MIN_WIDTH, ((n.to - n.from + 1) / span) * 100),
        row,
        title: `${n.from === n.to ? `frame ${n.from}` : `frames ${n.from} to ${n.to}`}: ${n.text}`,
      };
    });
}

/** The notes whose range holds `frame`. */
export function notesAt(notes: Note[], frame: number): Note[] {
  return notes.filter((n) => n.from <= frame && frame <= n.to);
}

/** Tags from comma- or space-separated text, lower-cased, without repeats. */
export function parseTags(text: string): string[] {
  const out: string[] = [];
  for (const t of text.split(/[\s,]+/)) {
    const tag = t.trim().toLowerCase();
    if (tag && !out.includes(tag)) out.push(tag);
  }
  return out;
}
