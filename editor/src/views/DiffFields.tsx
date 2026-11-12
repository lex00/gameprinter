// Diff mode: where two recordings first differ (from GET /api/diff, the
// output of `gp trace diff`), and which fields differ at the current frame.

import type { FrameState } from "../state";
import { registerView, type ViewProps } from "./registry";

/** Fields whose bytes differ between two states, with how many bytes. */
export function differingFields(a?: FrameState, b?: FrameState): { field: string; bytes: number }[] {
  if (!a || !b) return [];
  const out: { field: string; bytes: number }[] = [];
  for (const [name, x] of a) {
    const y = b.get(name);
    if (!y) continue;
    let n = Math.abs(x.length - y.length);
    for (let i = 0; i < Math.min(x.length, y.length); i++) if (x[i] !== y[i]) n++;
    if (n) out.push({ field: name, bytes: n });
  }
  return out;
}

function DiffFields({ a, b, diff, frame, setFrame }: ViewProps) {
  if (!b) return null;
  const now = differingFields(a.state, b.state);
  return (
    <div data-testid="diff-fields">
      {diff && diff.frame !== null ? (
        <p>
          First difference at{" "}
          <button className="link" onClick={() => setFrame(diff.frame!)}>
            frame {diff.frame}
          </button>
          : {diff.ram_differs ? "internal RAM differs" : "RAM matches, other state differs"}
          {", "}
          {diff.fields?.length ?? 0} fields.
        </p>
      ) : (
        <p>{diff ? `No frame differs (frame counts ${diff.frames?.join(" and ")}).` : "Comparing..."}</p>
      )}
      <p className="hint">At frame {frame}: {now.length ? `${now.length} fields differ` : "every field matches"}.</p>
      <ul className="fields">
        {now.map((f) => (
          <li key={f.field}>
            <span className="mono">{f.field}</span>: {f.bytes} bytes
          </li>
        ))}
      </ul>
    </div>
  );
}

registerView({ id: "diff", title: "Differences", order: 5, diffOnly: true, component: DiffFields });
