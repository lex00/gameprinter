// Annotations on the timeline: each note on a range of frames is a mark
// under the scrubber, and the notes covering the current frame are listed
// with a form to add, edit and delete them. Notes are kept by gp serve in
// a file per recording, outside the store (docs/formats/annotations.md).

import { useCallback, useEffect, useRef, useState } from "react";
import type { Api, Note } from "./api";
import { marks, notesAt, parseTags } from "./annotate";

interface Draft {
  from: number;
  to: number;
  text: string;
  tags: string;
  /** The note being edited, or undefined for a new one. */
  editing?: number;
}

export function Timeline({ notes, frames, frame, setFrame }: { notes: Note[]; frames: number; frame: number; setFrame: (f: number) => void }) {
  const ms = marks(notes, frames);
  const rows = Math.max(1, ...ms.map((m) => m.row + 1));
  return (
    <div className="timeline" style={{ height: rows * 8 + 4 }} data-testid="timeline">
      <div className="timeline-now" style={{ left: `${(frame / Math.max(1, frames + 1)) * 100}%` }} />
      {ms.map((m) => (
        <button
          key={m.id}
          className="timeline-mark"
          style={{ left: `${m.left}%`, width: `${m.width}%`, top: m.row * 8 + 2 }}
          title={m.title}
          aria-label={m.title}
          onClick={() => setFrame(notes.find((n) => n.id === m.id)!.from)}
        />
      ))}
    </div>
  );
}

export function Annotations({ api, id, frames, frame, setFrame }: { api: Api; id: string; frames: number; frame: number; setFrame: (f: number) => void }) {
  const [notes, setNotes] = useState<Note[]>([]);
  const [error, setError] = useState<string>();
  const [draft, setDraft] = useState<Draft>();

  // The recording the shown notes belong to. A reply for another one, from
  // before a switch, is dropped, so an edit never sends one recording's
  // note numbers to another.
  const current = useRef(id);
  current.current = id;
  const reload = useCallback(
    () =>
      api
        .annotations(id)
        .then((r) => {
          if (current.current !== id) return;
          setNotes(r.annotations);
          setError(undefined);
        })
        .catch((e: Error) => current.current === id && setError(e.message)),
    [api, id],
  );
  useEffect(() => {
    setNotes([]);
    setDraft(undefined);
    setError(undefined);
    void reload();
  }, [reload]);

  const save = async () => {
    if (!draft) return;
    const body = { from: draft.from, to: draft.to, text: draft.text, tags: parseTags(draft.tags) };
    try {
      if (draft.editing === undefined) await api.addNote(id, body);
      else await api.updateNote(id, draft.editing, body);
      setDraft(undefined);
      await reload();
    } catch (e) {
      setError((e as Error).message);
    }
  };
  const remove = async (n: number) => {
    try {
      await api.deleteNote(id, n);
      await reload();
    } catch (e) {
      setError((e as Error).message);
    }
  };

  const here = notesAt(notes, frame);
  const num = (v: string, fallback: number) => (v.trim() === "" || !Number.isFinite(Number(v)) ? fallback : Math.max(0, Math.min(frames, Math.floor(Number(v)))));

  return (
    <section className="annotations" data-testid="annotations">
      <Timeline notes={notes} frames={frames} frame={frame} setFrame={setFrame} />
      <div className="notes-here">
        {here.map((n) => (
          <div key={n.id} className="note">
            <span className="hint">
              #{n.id} {n.from === n.to ? `frame ${n.from}` : `frames ${n.from} to ${n.to}`}
            </span>{" "}
            {n.text}
            {n.tags?.map((t) => (
              <span key={t} className="tag">
                {t}
              </span>
            ))}{" "}
            <button className="link" onClick={() => setDraft({ from: n.from, to: n.to, text: n.text, tags: (n.tags ?? []).join(" "), editing: n.id })}>
              edit
            </button>{" "}
            <button className="link" onClick={() => void remove(n.id)}>
              delete
            </button>
          </div>
        ))}
        {!draft && (
          <button className="link" onClick={() => setDraft({ from: frame, to: frame, text: "", tags: "" })} data-testid="annotate">
            annotate frame {frame}
          </button>
        )}
        <span className="hint"> {notes.length} {notes.length === 1 ? "note" : "notes"}</span>
      </div>
      {draft && (
        <form
          className="note-form"
          onSubmit={(e) => {
            e.preventDefault();
            void save();
          }}
        >
          <label>
            from <input type="number" min={0} max={frames} value={draft.from} onChange={(e) => setDraft({ ...draft, from: num(e.target.value, draft.from) })} />
          </label>
          <button type="button" className="link" onClick={() => setDraft({ ...draft, from: frame })}>
            here
          </button>
          <label>
            to <input type="number" min={0} max={frames} value={draft.to} onChange={(e) => setDraft({ ...draft, to: num(e.target.value, draft.to) })} />
          </label>
          <button type="button" className="link" onClick={() => setDraft({ ...draft, to: frame })}>
            here
          </button>
          <input type="text" className="note-text" placeholder="note" value={draft.text} onChange={(e) => setDraft({ ...draft, text: e.target.value })} data-testid="note-text" />
          <input type="text" className="note-tags" placeholder="tags" value={draft.tags} onChange={(e) => setDraft({ ...draft, tags: e.target.value })} />
          <button type="submit" disabled={draft.text.trim() === "" || draft.from > draft.to}>
            {draft.editing === undefined ? "add" : "save"}
          </button>
          <button type="button" className="link" onClick={() => setDraft(undefined)}>
            cancel
          </button>
        </form>
      )}
      {error && <p className="error">{error}</p>}
    </section>
  );
}
