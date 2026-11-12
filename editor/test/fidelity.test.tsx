// The fidelity line under the progress headline (docs/fidelity.md): the
// score with its parts, the evidence behind it, the trend over models and
// the three things holding it down, on a made-up score.
import { describe, expect, it } from "vitest";
import { renderToString } from "react-dom/server";
import { type FidelityState, FidelityLine, partsWords, sparkPoints, trendScores } from "../src/Fidelity";

const part = (score: number) => ({ score, ppm: score * 10000 });

const f: FidelityState = {
  version: "fidelity 1",
  score: 71,
  behaviour: { ...part(78), clips: 185, exact: 71 },
  coverage: part(64),
  picture: part(70),
  confidence: { level: "medium", says: "Medium evidence: based on 185 clips in 29 of 32 levels" },
  holding: [
    { part: "behaviour", says: "in the jump, Mario's height goes wrong (31 clips)", milli_points: 9000 },
    { part: "coverage", says: "enemies aren't carried over yet (75 starts), so 36 of 37 kinds of thing are in no scored clip (blooper, bowser and 34 more)", milli_points: 8000 },
    { part: "picture", says: "the picture looks different: the background on 30% of frames (185 clips)", milli_points: 4000 },
  ],
  says: "Fidelity 71 / 100 (behaviour 78, coverage 64, picture 70)",
  trend: "62 -> 71 after fixes to the jump",
  history: [
    { model: "a", score: 62, behaviour: 70, coverage: 64, picture: 70, clips: 185, after: "first score" },
    { model: "b", score: 71, behaviour: 78, coverage: 64, picture: 70, clips: 185, after: "after fixes to the jump" },
  ],
};

describe("the fidelity line", () => {
  it("says the score with its parts, the evidence, the trend and what holds it down", () => {
    expect(partsWords(f)).toBe("behaviour 78, coverage 64, picture 70");
    const html = renderToString(<FidelityLine f={f} />);
    expect(html).toContain('data-testid="fidelity"');
    expect(html).toMatch(/data-testid="fidelity-score">71<\/strong>/);
    expect(html).toContain("/ 100 (behaviour 78, coverage 64, picture 70)");
    expect(html).toContain("Medium evidence: based on 185 clips in 29 of 32 levels");
    expect(html).toContain("62 -&gt; 71 after fixes to the jump");
    expect(html.match(/data-testid="fidelity-holding"/g)).toHaveLength(3);
    expect(html).toContain("in the jump, Mario&#x27;s height goes wrong (31 clips)");
    expect(html).toContain('data-testid="fidelity-trend"');
    expect(html).not.toContain("low evidence");
  });

  it("marks a low-evidence score and leaves out a trend of one model", () => {
    const low: FidelityState = {
      ...f,
      score: 0,
      picture: null,
      confidence: { level: "low", says: "Low evidence: based on 0 clips in 0 of 1 levels" },
      holding: [],
      trend: "",
      history: [],
    };
    const html = renderToString(<FidelityLine f={low} />);
    expect(html).toContain("low evidence");
    expect(html).toContain("picture not measured");
    expect(html).not.toContain('data-testid="fidelity-trend"');
    expect(html).not.toContain("Holding it down");
  });

  it("draws the trend from the last scores, 0 at the bottom and 100 at the top", () => {
    expect(trendScores(f)).toEqual([62, 71]);
    expect(sparkPoints([0, 100])).toBe("0,16 60,0");
    expect(sparkPoints([50])).toBe("");
  });
});
