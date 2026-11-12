// Labeled RAM watch: each label's bytes at this frame, marked when they
// changed since the frame before, with the second recording's beside them
// in diff mode. Labels come from the recording's game (docs/inspector.md).

import { useState } from "react";
import type { Label } from "../api";
import { formatBytes, labelBytes } from "./labels";
import { registerView, type ViewProps } from "./registry";

function same(a?: number[], b?: number[]): boolean {
  return a !== undefined && b !== undefined && a.length === b.length && a.every((x, i) => x === b[i]);
}

function RamWatch({ a, b }: ViewProps) {
  const [filter, setFilter] = useState("");
  const [changedOnly, setChangedOnly] = useState(false);
  if (a.labels.labels.length === 0) {
    return <p className="hint">No labels for this recording{a.labels.missing ? `: ${a.labels.missing}` : ""}.</p>;
  }
  const ramA = a.state?.get("ram");
  const prevA = a.prev?.get("ram");
  const ramB = b?.state?.get("ram");
  const rows = a.labels.labels
    .filter((l: Label) => !filter || l.name.toLowerCase().includes(filter.toLowerCase()))
    .map((l) => {
      const now = labelBytes(ramA, l);
      const before = labelBytes(prevA, l);
      const other = b ? labelBytes(ramB, l) : undefined;
      return { l, now, changed: before !== undefined && !same(now, before), differs: b !== undefined && now !== undefined && other !== undefined && !same(now, other), other };
    })
    .filter((r) => !changedOnly || r.changed || r.differs);
  return (
    <div>
      <div className="toolbar">
        <input placeholder="filter labels" value={filter} onChange={(e) => setFilter(e.target.value)} />
        <label>
          <input type="checkbox" checked={changedOnly} onChange={(e) => setChangedOnly(e.target.checked)} /> changed only
        </label>
        <span className="hint">{a.labels.game} labels from {a.labels.file}</span>
      </div>
      <div className="scroll">
        <table className="watch" data-testid="ram-watch">
          <thead>
            <tr>
              <th>Label</th>
              <th>Addr</th>
              <th>{b ? a.name : "Value"}</th>
              {b && <th>{b.name}</th>}
              {!b && <th>Description</th>}
            </tr>
          </thead>
          <tbody>
            {rows.map(({ l, now, changed, differs, other }) => (
              <tr key={`${l.name}-${l.addr}`} className={differs ? "differs" : changed ? "changed" : ""}>
                <td title={l.description}>{l.name}</td>
                <td>${l.addr.toString(16).padStart(4, "0")}</td>
                <td className="mono">{formatBytes(now)}</td>
                {b && <td className="mono">{formatBytes(other)}</td>}
                {!b && <td className="desc">{l.description}</td>}
              </tr>
            ))}
          </tbody>
        </table>
      </div>
    </div>
  );
}

registerView({ id: "ram-watch", title: "RAM watch", order: 20, component: RamWatch });
