// The PDF perf gate's verdict rule (scripts/perf-pdf/verdict.mjs,
// docs/pdf-perf-harness.md): a regression needs the candidate's best session
// to be worse than the base's worst by more than max(base range, floor,
// rel × base median); same-binary noise must never fail the gate.
import { describe, expect, it } from "vitest";
import { compare, FAMILIES, verdict } from "../scripts/perf-pdf/verdict.mjs";

describe("PDF perf gate verdict", () => {
  it("flags a shift beyond the base's spread and the floor", () => {
    expect(compare([0, 5, 10], [500, 800, 1500], "blank").status).toBe("regression");
  });

  it("does not flag overlapping run-to-run ranges, even with a large median shift", () => {
    const r = compare([100, 200, 300], [150, 700, 1000], "blank");
    expect(r.status).toBe("watch");
  });

  it("does not flag a non-overlapping shift smaller than the floor", () => {
    const { floor } = FAMILIES.blank;
    expect(compare([0, 1, 2], [2 + floor - 1, 2 + floor, 2 + floor], "blank").status).not.toBe("regression");
  });

  it("does not flag a non-overlapping shift smaller than the base's own range", () => {
    // base range 300 > gap 150 > floor and > 25 % of the base median (1150).
    expect(compare([1000, 1150, 1300], [1450, 1460, 1470], "blank").status).not.toBe("regression");
  });

  it("scales the threshold with the base median", () => {
    // gap 300 clears the 40 ms floor and the base range but not 25 % of 2000.
    expect(compare([1990, 2000, 2010], [2310, 2320, 2330], "blank").status).toBe("same");
    expect(compare([1990, 2000, 2010], [2600, 2610, 2620], "blank").status).toBe("regression");
  });

  it("does not flag the same-binary spreads measured on the reference machine", () => {
    // Same fixed-viewer binary, session values from the validation runs
    // (evidence/pdf-perf-gate): S1a fast blank at DPR 1 across two runs, and
    // the bimodal max frame of a DPR 2 zoom burst within one A/A run.
    expect(compare([46.1, 95.3, 75.8], [472, 195.6, 213.6], "blank").status).not.toBe("regression");
    expect(compare([76, 78, 79], [151, 144, 143], "max").status).not.toBe("regression");
  });

  it("flags the old viewer's zoom regressions against the fixed one", () => {
    // DPR 2 button zoom-in blank, and Ctrl+wheel anchor drift (validation run v2).
    expect(compare([0, 0, 0], [991, 656, 1090], "blank").status).toBe("regression");
    expect(compare([4, 4, 4], [403, 403, 403], "drift").status).toBe("regression");
  });

  it("reports improvements against the candidate's range", () => {
    expect(compare([500, 800, 1500], [0, 5, 10], "blank").status).toBe("improvement");
  });

  it("counts a candidate that lost values (never sharp) as a regression", () => {
    expect(compare([100, 110, 120], [100, 110], "sharp", 0, 1).status).toBe("regression");
  });

  it("gives no verdict with fewer than two sessions per side", () => {
    expect(compare([], [100, 110, 120], "sharp").status).toBe("n/a");
  });

  it("reduces each session to one value and pools S1c cycles inside the session", () => {
    const row = (tag: string, name: string, blankMs: number) => ({
      tag, dpr: 2, viewer: "ours", scenario: "S1c", name, anchorDriftPx: 0,
      whole: { blankMs, p95: 20, max: 40, over100: 0 }, sharpFromLastMs: 300, rss: { webMiB: 1000 },
    });
    const rows = [];
    for (const r of [1, 2, 3]) {
      for (const c of [1, 2, 3]) {
        // One wild base cycle per session is absorbed by the per-session median.
        rows.push(row(`base-dpr2-r${r}`, `ctrlbutton-in-${c}`, c === 1 ? 900 : 0));
        rows.push(row(`cand-dpr2-r${r}`, `ctrlbutton-in-${c}`, 600 + r * 10));
      }
    }
    const v = verdict(rows);
    const blank = v.results.find((x) => x.cell === "S1c button in" && x.family === "blank");
    expect(blank?.base).toEqual([0, 0, 0]);
    expect(blank?.status).toBe("regression");
    expect(v.regressions.map((x) => `${x.cell} ${x.family}`)).toEqual(["S1c button in blank"]);
  });
});
