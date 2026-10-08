import { afterEach, describe, expect, it } from "vitest";
import { createSignal } from "solid-js";
import { render } from "solid-js/web";
import {
  DiffRowView,
  differingLineFlags,
  firstDifferingLine,
  firstLine,
  mergedTitle,
  humanizeSideLabel,
  needsExpander,
  previewLine,
  seedSuggestedExceptArtifact,
  seedSuggestedOrNoLoss,
  collectRows,
  countSuggestions,
  visibleDiffRows,
} from "./DiffRows";
import type { DiffRow, MergeDecision } from "../types";

// Concord's fourth row outcome (ported from master for og 8c). Two things are
// asserted here that nothing else covers:
//
//  - the SUGGESTED MERGED BODY: a row whose two edits touched different parts of
//    one block gets a fourth segment and a full-width preview strip, seeds to
//    "merged" like any other suggestion, and still applies nothing on its own;
//  - the PREVIEW FIX: a collapsed row previews the first line that actually
//    DIFFERS, not line 0. Before this, two multi-line bodies differing on line 7
//    showed the user two identical strings next to a decision.
//
// All content is synthetic.

const flush = () => new Promise((resolve) => setTimeout(resolve, 0));

afterEach(() => {
  document.body.innerHTML = "";
});

const view = (text: string) => ({ uuid: "", text, child_count: 0 });

/** A `both-changed` row whose disjoint edits the backend could merge. */
const mergedRow: DiffRow = {
  id: "1",
  kind: "modified",
  mine: view("Desktop"),
  theirs: view("Desktop 5 kk"),
  children: [],
  verdict: "both-changed",
  suggestion: "merged",
  merged: { text: "Desktop kk", source: "computed" },
};

/** The same row filled from the merge tool's own suggested resolution instead
 *  (Phase 2). The UI slot is identical; only the provenance differs. */
const artifactRow: DiffRow = {
  ...mergedRow,
  mine: view("the quick brown fox jumped"),
  theirs: view("the quick brown fox leaped"),
  merged: { text: "the quick brown fox leapt", source: "artifact" },
};

function mountRow(row: DiffRow, seeded: Record<string, MergeDecision>): {
  host: HTMLElement;
  dispose: () => void;
} {
  const host = document.createElement("div");
  document.body.appendChild(host);
  const [decisions, setDecisions] = createSignal(seeded);
  const dispose = render(
    () => (
      <div class="page-conflict">
        <DiffRowView
          row={row}
          depth={0}
          decisions={decisions()}
          setDecision={(id, d) => setDecisions((m) => ({ ...m, [id]: d }))}
          fallback="both"
          labels={{ mine: "This device", theirs: "Copy" }}
        />
      </div>
    ),
    host,
  );
  return { host, dispose };
}

describe("first-differing-line preview", () => {
  it("is exactly today's firstLine when the first non-blank lines already differ", () => {
    expect(firstDifferingLine("alpha", "beta")).toBe(0);
    expect(previewLine("alpha", 0)).toBe(firstLine("alpha"));
    // Leading blank lines are skipped by firstLine, so k stays 0 and the
    // rendering is unchanged — no elision marker on the common case.
    expect(firstDifferingLine("\n\n  alpha", "\n\nbeta")).toBe(0);
    expect(previewLine("\n\n  alpha", 0)).toBe("alpha");
  });

  it("walks down to the first raw line that differs once the visible lines agree", () => {
    const mine = "shared\nsecond\nmine here";
    const theirs = "shared\nsecond\ntheirs here";
    expect(firstDifferingLine(mine, theirs)).toBe(2);
    expect(previewLine(mine, 2)).toBe("mine here");
    expect(previewLine(theirs, 2)).toBe("theirs here");
  });

  it("indexes RAW lines, so leading blanks are counted once the sides agree", () => {
    expect(firstDifferingLine("\n\nalpha\nmine", "\n\nalpha\ntheirs")).toBe(3);
    expect(previewLine("\n\nalpha\nmine", 3)).toBe("mine");
  });

  it("reports the absent marker's null for the shorter side", () => {
    expect(firstDifferingLine("alpha", "alpha\nextra")).toBe(1);
    expect(previewLine("alpha", 1)).toBe(null);
    expect(previewLine("alpha\nextra", 1)).toBe("extra");
  });

  it("treats a blank line as a difference, and identical bodies as k = 0", () => {
    expect(firstDifferingLine("alpha\n\nbeta", "alpha\nbeta")).toBe(1);
    expect(firstDifferingLine("alpha\nbeta", "alpha\nbeta")).toBe(0);
  });
});

describe("expander visibility", () => {
  it("stays away when every body is a single line", () => {
    expect(needsExpander(["alpha", "beta", undefined], 0)).toBe(false);
    expect(needsExpander(["alpha", "beta", "alpha beta"], 0)).toBe(false);
  });

  it("appears as soon as any involved body is multiline", () => {
    expect(needsExpander(["alpha\nlog", "alpha", undefined], 0)).toBe(true);
    expect(needsExpander(["alpha", "beta", "alpha\nbeta"], 0)).toBe(true);
  });

  it("appears when the bodies differ on a line the preview does not show", () => {
    expect(needsExpander(["alpha", "beta"], 1)).toBe(true);
  });

  it("needs two bodies to have anything to compare", () => {
    expect(needsExpander(["alpha\nbeta", null, undefined], 0)).toBe(false);
  });
});

describe("per-line emphasis in the expanded view", () => {
  it("flags every line that is not identical across all shown bodies", () => {
    expect(
      differingLineFlags([
        ["same", "mine", "tail"],
        ["same", "theirs", "tail"],
      ]),
    ).toEqual([
      [false, true, false],
      [false, true, false],
    ]);
  });

  it("flags a line one body simply does not have", () => {
    expect(differingLineFlags([["same"], ["same", "extra"]])).toEqual([[false], [false, true]]);
  });
});

describe("seeding a merged suggestion", () => {
  it("pre-selects 'merged' the same way any other suggestion is pre-selected", () => {
    expect(seedSuggestedOrNoLoss([mergedRow])).toEqual({ "1": "merged" });
  });

  it("still falls back to no-loss where the backend offered no merge", () => {
    const noOffer: DiffRow = { ...mergedRow, suggestion: null, merged: null };
    expect(seedSuggestedOrNoLoss([noOffer])).toEqual({ "1": "both" });
  });
});

describe("humanizing sync-copy side labels", () => {
  it("renders a Syncthing tag as a dated 'Sync copy', keeping the tag as tooltip", () => {
    const y = new Date().getFullYear();
    const tag = `sync-conflict-${y}0705-141233-A2B2C3D`;
    expect(humanizeSideLabel(tag)).toEqual({ text: "Sync copy · Jul 5", title: tag });
  });

  it("appends the year only when it is not the current one", () => {
    expect(humanizeSideLabel("sync-conflict-20190705-141233-A2B2C3D").text).toBe(
      "Sync copy · Jul 5 2019",
    );
  });

  it("recognizes the Dropbox 'conflicted copy' wording", () => {
    const y = new Date().getFullYear();
    expect(humanizeSideLabel(`Martin's conflicted copy ${y}-12-31`).text).toBe(
      "Sync copy · Dec 31",
    );
  });

  it("passes every other label through untouched (git refs, device names)", () => {
    expect(humanizeSideLabel("HEAD")).toEqual({ text: "HEAD" });
    expect(humanizeSideLabel("Phone")).toEqual({ text: "Phone" });
  });
});

describe("narrow-container segment variants", () => {
  it("side-labeled segments carry a dot + generic short word; Both/Merged do not", async () => {
    const { host, dispose } = mountRow(mergedRow, seedSuggestedOrNoLoss([mergedRow]));
    try {
      await flush();
      const mine = host.querySelector('.sync-merge-seg[data-decision="mine"]')!;
      expect(mine.querySelector('.sync-merge-seg-dot[data-side="mine"]')).not.toBeNull();
      expect(mine.querySelector(".sync-merge-seg-long")!.textContent).toBe("This device");
      expect(mine.querySelector(".sync-merge-seg-short")!.textContent).toBe("Mine");
      expect(mine.getAttribute("title")).toBe("This device");
      const theirs = host.querySelector('.sync-merge-seg[data-decision="theirs"]')!;
      expect(theirs.querySelector(".sync-merge-seg-short")!.textContent).toBe("Theirs");
      // Both/Merged are already short: single-span content, no dot.
      const both = host.querySelector('.sync-merge-seg[data-decision="both"]')!;
      expect(both.textContent).toBe("Both");
      expect(both.querySelector(".sync-merge-seg-dot")).toBeNull();
      expect(
        host.querySelector('.sync-merge-seg[data-decision="merged"]')!.textContent,
      ).toBe("Merged");
    } finally {
      dispose();
    }
  });
});

describe("the Apply-all-suggested sweep vs artifact proposals", () => {
  it("re-applies a COMPUTED merged suggestion like any other", () => {
    expect(seedSuggestedExceptArtifact([mergedRow], { "1": "mine" })).toEqual({ "1": "merged" });
  });

  it("never flips a row back to the merge tool's own text", () => {
    // The user moved the artifact row to "mine"; the sweep leaves it there.
    expect(seedSuggestedExceptArtifact([artifactRow], { "1": "mine" })).toEqual({ "1": "mine" });
  });

  it("leaves an untouched artifact pre-selection standing", () => {
    // Initial seed picked "merged" (pre-selection is per-row consent, kept);
    // the sweep neither clears nor re-asserts it.
    const seeded = seedSuggestedOrNoLoss([artifactRow]);
    expect(seedSuggestedExceptArtifact([artifactRow], { ...seeded })).toEqual({ "1": "merged" });
  });
});

describe("the merged row in the resolver", () => {
  it("renders a fourth segment and a full-width merged strip", async () => {
    const { host, dispose } = mountRow(mergedRow, seedSuggestedOrNoLoss([mergedRow]));
    try {
      await flush();
      const seg = host.querySelector('.sync-merge-seg[data-decision="merged"]')!;
      expect(seg.textContent).toBe("Merged");
      expect(seg.getAttribute("data-side")).toBe("merged");
      // Fourth: the three original choices are untouched and still come first.
      const order = [...host.querySelectorAll(".sync-merge-seg")].map((b) =>
        b.getAttribute("data-decision"),
      );
      expect(order).toEqual(["mine", "theirs", "both", "merged"]);
      // The proposal is a strip below the columns, not a third column.
      const strip = host.querySelector(".sync-merge-cell.merged")!;
      expect(strip.parentElement!.className).toBe("sync-merge-row");
      expect(host.querySelectorAll(".sync-merge-cols .sync-merge-cell").length).toBe(2);
      expect(strip.textContent).toContain("Desktop kk");
    } finally {
      dispose();
    }
  });

  it("names the merge tool as the source of an artifact proposal", async () => {
    const { host, dispose } = mountRow(artifactRow, seedSuggestedOrNoLoss([artifactRow]));
    try {
      await flush();
      // Same slot, same decision value — only the provenance wording differs.
      const strip = host.querySelector(".sync-merge-cell.merged")!;
      expect(strip.getAttribute("data-source")).toBe("artifact");
      expect(strip.getAttribute("title")).toBe(mergedTitle("artifact"));
      expect(strip.getAttribute("title")).not.toBe(mergedTitle("computed"));
      expect(strip.querySelector(".sync-merge-mergedtag")!.textContent).toBe("Merged (tool)");
      expect(strip.textContent).toContain("the quick brown fox leapt");
      // It seeds "merged" exactly like a computed one, and applies nothing.
      expect(seedSuggestedOrNoLoss([artifactRow])).toEqual({ "1": "merged" });
      expect(
        host.querySelector('.sync-merge-seg[data-decision="merged"]')!.classList.contains("active"),
      ).toBe(true);
    } finally {
      dispose();
    }
  });

  it("keeps today's wording for a computed proposal", async () => {
    const { host, dispose } = mountRow(mergedRow, seedSuggestedOrNoLoss([mergedRow]));
    try {
      await flush();
      const strip = host.querySelector(".sync-merge-cell.merged")!;
      expect(strip.getAttribute("data-source")).toBe("computed");
      expect(strip.getAttribute("title")).toBe(mergedTitle("computed"));
      expect(strip.querySelector(".sync-merge-mergedtag")!.textContent).toBe("Merged");
    } finally {
      dispose();
    }
  });

  it("marks the strip chosen only while 'merged' is the decision", async () => {
    const { host, dispose } = mountRow(mergedRow, { "1": "both" });
    try {
      await flush();
      const strip = () => host.querySelector(".sync-merge-cell.merged")!;
      expect(strip().classList.contains("chosen")).toBe(false);
      (host.querySelector('.sync-merge-seg[data-decision="merged"]') as HTMLElement).click();
      await flush();
      expect(strip().classList.contains("chosen")).toBe(true);
      (host.querySelector('.sync-merge-seg[data-decision="mine"]') as HTMLElement).click();
      await flush();
      expect(strip().classList.contains("chosen")).toBe(false);
    } finally {
      dispose();
    }
  });

  it("shows the suggested tag exactly while the decision matches the suggestion", async () => {
    const { host, dispose } = mountRow(mergedRow, seedSuggestedOrNoLoss([mergedRow]));
    try {
      await flush();
      expect(host.querySelector(".sync-merge-suggested-tag")).not.toBe(null);
      (host.querySelector('.sync-merge-seg[data-decision="theirs"]') as HTMLElement).click();
      await flush();
      expect(host.querySelector(".sync-merge-suggested-tag")).toBe(null);
    } finally {
      dispose();
    }
  });

  it("leaves a row with no proposal exactly as it was", async () => {
    const plain: DiffRow = {
      id: "2",
      kind: "modified",
      mine: view("gamma my way"),
      theirs: view("gamma their way"),
      children: [],
      verdict: "both-changed",
    };
    const { host, dispose } = mountRow(plain, { "2": "both" });
    try {
      await flush();
      expect(host.querySelector(".sync-merge-cell.merged")).toBe(null);
      expect(host.querySelector('.sync-merge-seg[data-decision="merged"]')).toBe(null);
      expect(host.querySelector(".sync-merge-expand")).toBe(null);
      expect(host.querySelector(".sync-merge-elided")).toBe(null);
    } finally {
      dispose();
    }
  });
});

describe("the collapsed preview and the expander in the resolver", () => {
  const logbookRow: DiffRow = {
    id: "3",
    kind: "modified",
    mine: view("TODO write the notes\n:LOGBOOK:\nCLOCK: mine\n:END:"),
    theirs: view("TODO write the notes\n:LOGBOOK:\nCLOCK: theirs\n:END:"),
    children: [],
    verdict: "both-changed",
  };

  it("previews the first DIFFERING line, marked as elided, one line per column", async () => {
    const { host, dispose } = mountRow(logbookRow, { "3": "both" });
    try {
      await flush();
      const cells = [...host.querySelectorAll(".sync-merge-cols .sync-merge-cell")];
      expect(cells.map((c) => c.textContent)).toEqual(["…CLOCK: mine", "…CLOCK: theirs"]);
      expect(host.querySelectorAll(".sync-merge-elided").length).toBe(2);
      // Collapsed is still one line per column: nothing is expanded yet.
      expect(host.querySelector(".sync-merge-expanded")).toBe(null);
    } finally {
      dispose();
    }
  });

  it("expands to the full bodies on request, emphasising only the differing lines", async () => {
    const { host, dispose } = mountRow(logbookRow, { "3": "both" });
    try {
      await flush();
      const toggle = host.querySelector(".sync-merge-expand") as HTMLElement;
      expect(toggle.textContent).toBe("⌄ 4");
      toggle.click();
      await flush();
      const bodies = [...host.querySelectorAll(".sync-merge-fulltext")];
      expect(bodies.map((b) => b.getAttribute("data-side"))).toEqual(["mine", "theirs"]);
      expect(bodies[0].querySelectorAll(".sync-merge-fulltext-line").length).toBe(4);
      expect(
        [...bodies[0].querySelectorAll(".sync-merge-fulltext-line")].map((l) =>
          l.classList.contains("differs"),
        ),
      ).toEqual([false, false, true, false]);
      // And it collapses back.
      (host.querySelector(".sync-merge-expand") as HTMLElement).click();
      await flush();
      expect(host.querySelector(".sync-merge-expanded")).toBe(null);
    } finally {
      dispose();
    }
  });

  it("stacks the merged body with the two sides when a proposal exists", async () => {
    const multiline: DiffRow = {
      ...mergedRow,
      mine: view("Desktop\nsecond line"),
      theirs: view("Desktop 5 kk\nsecond line"),
      merged: { text: "Desktop kk\nsecond line", source: "computed" },
    };
    const { host, dispose } = mountRow(multiline, { "1": "merged" });
    try {
      await flush();
      (host.querySelector(".sync-merge-expand") as HTMLElement).click();
      await flush();
      const bodies = [...host.querySelectorAll(".sync-merge-fulltext")];
      expect(bodies.map((b) => b.getAttribute("data-side"))).toEqual(["mine", "theirs", "merged"]);
      expect(bodies.map((b) => b.querySelector(".sync-merge-fulltext-label")!.textContent)).toEqual([
        "This device",
        "Copy",
        "Merged",
      ]);
      // Line 0 differs across all three; line 1 is shared, so it stays quiet.
      expect(
        [...bodies[2].querySelectorAll(".sync-merge-fulltext-line")].map((l) =>
          l.classList.contains("differs"),
        ),
      ).toEqual([true, false]);
    } finally {
      dispose();
    }
  });
});

describe("row traversal at the outline cap (I-22)", () => {
  // og keeps the diff tree walk iterative: a chain deeper than any JS stack
  // would tolerate recursively still seeds, collects and lists in order.
  function chain(levels: number): DiffRow[] {
    let row: DiffRow | null = null;
    for (let level = levels; level >= 1; level--) {
      row = { id: String(level), kind: level % 2 ? "modified" : "unchanged", mine: view(`m${level}`), theirs: view(`t${level}`), children: row ? [row] : [] };
    }
    return [row!];
  }

  it("seeds and collects every decidable row of a 20,000-deep chain in document order", () => {
    const rows = chain(20_000);
    const collected = collectRows(rows);
    expect(collected.length).toBe(10_000);
    expect(collected[0].id).toBe("1");
    expect(collected[collected.length - 1].id).toBe("19999");
    const seeded = seedSuggestedOrNoLoss(rows);
    expect(Object.keys(seeded).length).toBe(10_000);
    expect(seeded["1"]).toBe("both");
    expect(countSuggestions(rows)).toBe(0);
  });

  it("lists visible rows with depth, hiding an unchanged row's subtree", () => {
    const row = (id: string, kind: DiffRow["kind"], children: DiffRow[] = []): DiffRow => ({ id, kind, mine: view(id), theirs: view(id), children });
    const tree = [row("A", "modified", [row("B", "unchanged", [row("C", "modified")]), row("E", "added")]), row("D", "removed")];
    expect(visibleDiffRows(tree, false).map((r) => `${r.depth}:${r.row.id}`)).toEqual(["0:A", "1:E", "0:D"]);
    expect(visibleDiffRows(tree, true).map((r) => `${r.depth}:${r.row.id}`)).toEqual(["0:A", "1:B", "2:C", "1:E", "0:D"]);
  });
});
