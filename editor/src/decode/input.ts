// Input logs (docs/formats/recording.md, "Input log"): a version line, a
// controller count, then one line per frame. Entry N-1 is the input frame N
// ran with. Format 1 adds `P!` and `-!`: the reset button pressed at the
// start of that frame.

export interface InputFrame {
  polled: boolean;
  /** Whether the reset button is pressed at the start of the frame. */
  reset: boolean;
  /** One string per controller, in RLDUTSBA order, `.` when not held. */
  pads: string[];
}

export const BUTTONS = "RLDUTSBA";

export function parseInputLog(text: string): InputFrame[] {
  const lines = text.split("\n").filter((l) => l.length > 0);
  if (!lines[0]?.startsWith("gp-input-log ")) throw new Error("not an input log");
  return lines.slice(2).map((line) => {
    const [flag, ...pads] = line.split(" ");
    return { polled: flag.startsWith("P"), reset: flag.endsWith("!"), pads };
  });
}

/** The buttons held on one pad, as names: `Right+B`. */
export function heldNames(pad: string): string {
  const names = ["Right", "Left", "Down", "Up", "Start", "Select", "B", "A"];
  const held = names.filter((_, i) => pad[i] !== undefined && pad[i] !== ".");
  return held.length ? held.join("+") : "none";
}
