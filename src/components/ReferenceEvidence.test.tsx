import { afterEach, describe, expect, it, vi } from "vitest";
import { startEditing } from "../editorController";
import { openPage } from "../router";
import { resetPaneLayoutToSingle } from "../panes";

vi.mock("../editorController", async (original) => ({ ...(await original<object>()), startEditing: vi.fn() }));
vi.mock("../document", async (original) => ({
  ...(await original<object>()),
  resolveBlockRef: vi.fn(() => "runtime-1"),
  node: vi.fn(() => ({ raw: "aBooksbBooksc" })),
}));
import { render } from "solid-js/web";
import { OccurrenceControls, ReferenceExcerptBlocks, buildFullMarkedSegments, occurrenceSelection } from "./ReferenceEvidence";
import type { BlockDto, ReferenceBlockEvidence, ReferenceOccurrence } from "../types";

afterEach(() => {
  document.body.innerHTML = "";
  vi.useRealTimers();
  vi.mocked(startEditing).mockClear();
  resetPaneLayoutToSingle({ tabs: [{ history: [{ kind: "journals" }], pos: 0, pinned: false }], activeIndex: 0 });
});

const occ = (start: number): ReferenceOccurrence => ({
  matched_name: "Books",
  canonical: "books",
  kind: "explicit",
  span: { start, end: start + 5 },
  rule: "page-ref",
});
const evidence = (n: number): ReferenceBlockEvidence => ({
  block_id: "b1",
  occurrences: Array.from({ length: n }, (_, i) => occ(i * 10)),
});

describe("OccurrenceControls (GH #200: no redundant count/jump for a single mention)", () => {
  it("renders nothing when a block mentions the page once", () => {
    const host = document.createElement("div");
    render(() => <OccurrenceControls evidence={evidence(1)} onOccurrence={() => {}} />, host);
    expect(host.querySelector(".reference-occurrence-controls")).toBeNull();
    expect(host.textContent ?? "").not.toContain("mention");
  });

  it("shows the count and one jump button per occurrence when a block mentions the page 2+ times", () => {
    const host = document.createElement("div");
    render(() => <OccurrenceControls evidence={evidence(3)} onOccurrence={() => {}} />, host);
    expect(host.querySelector(".reference-occurrence-controls")).not.toBeNull();
    expect(host.textContent ?? "").toContain("3 mentions");
    expect(host.querySelectorAll(".reference-occurrence-jump").length).toBe(3);
  });
});

const WORD = "Books";

function blockWith(mentions: number, gap: number): { block: BlockDto; evidence: ReferenceBlockEvidence } {
  const filler = "x".repeat(gap);
  const raw = Array.from({ length: mentions }, () => `${WORD}${filler}`).join("");
  const occurrences: ReferenceOccurrence[] = Array.from({ length: mentions }, (_, index) => ({
    matched_name: WORD,
    canonical: "books",
    kind: "plain",
    span: { start: index * (WORD.length + gap), end: index * (WORD.length + gap) + WORD.length },
    rule: "plain_unicode_boundary",
  }));
  return {
    block: { id: "b1", raw, children: [] } as unknown as BlockDto,
    evidence: { block_id: "b1", occurrences, total: mentions },
  };
}

function renderExcerpt(mentions: number, gap: number) {
  const { block, evidence: item } = blockWith(mentions, gap);
  const host = document.createElement("div");
  document.body.appendChild(host);
  render(() => <ReferenceExcerptBlocks blocks={[block]} evidence={[item]} page="Journal" kind="journal" />, host);
  return host;
}

describe("Unlinked reference occurrences (GH #200 round 2: the highlight is the affordance)", () => {
  it("retires the numbered jump row when the excerpt already shows every mention", () => {
    // A SHORT block with four mentions used to render four marks AND four
    // numbered circles saying the same thing twice.
    const host = renderExcerpt(4, 4);
    expect(host.querySelectorAll(".reference-excerpt-mark").length).toBe(4);
    expect(host.querySelector(".reference-occurrence-controls")).toBeNull();
    expect(host.textContent ?? "").not.toContain("4 mentions");
  });

  it("makes each highlighted mention a real button naming the page and the mention's ordinal", () => {
    const host = renderExcerpt(3, 4);
    const marks = [...host.querySelectorAll<HTMLButtonElement>(".reference-excerpt-mark")];
    expect(marks.map((mark) => mark.tagName)).toEqual(["BUTTON", "BUTTON", "BUTTON"]);
    expect(marks.map((mark) => mark.getAttribute("aria-label"))).toEqual([
      "Open mention 1 in Journal",
      "Open mention 2 in Journal",
      "Open mention 3 in Journal",
    ]);
  });

  it("keeps the jump row when mentions fall outside the excerpt windows", () => {
    // The excerpt caps at three windows, so later mentions are unreachable
    // without the numbered row.
    const host = renderExcerpt(6, 400);
    expect(host.querySelectorAll(".reference-excerpt-mark").length).toBeLessThan(6);
    expect(host.querySelector(".reference-occurrence-controls")).not.toBeNull();
    expect(host.textContent ?? "").toContain("6 mentions");
  });

  it("marks every mention once the block is expanded, and retires the row with it", () => {
    const host = renderExcerpt(6, 400);
    host.querySelector<HTMLButtonElement>(".reference-show-full")!.click();
    expect(host.querySelectorAll(".reference-excerpt-mark").length).toBe(6);
    expect(host.querySelector(".reference-occurrence-controls")).toBeNull();
  });

  it("marks every span over the whole block, in order, with no window", () => {
    const segments = buildFullMarkedSegments("aBooksbBooksc", [
      { start: 1, end: 6 },
      { start: 7, end: 12 },
    ]);
    expect(segments.map((segment) => segment.text)).toEqual(["a", "Books", "b", "Books", "c"]);
    expect(segments.filter((segment) => segment.marked).length).toBe(2);
    expect(segments.map((segment) => segment.text).join("")).toBe("aBooksbBooksc");
  });
});

describe("occurrence jumps land on a selection, not a collapsed caret", () => {
  // iOS paints no caret for a programmatic focus, so jump 1 and jump 4 looked
  // identical there; a selection is drawn everywhere (master GH #200).
  it("selects the whole mention in visible offsets", () => {
    expect(occurrenceSelection("see Books here", { start: 4, end: 9 }, "Some page")).toEqual({
      start: 4,
      end: 9,
      direction: "forward",
    });
  });
});

describe("I-20: a mention jump belongs to the surface the click opened", () => {
  const click = () => {
    const host = renderExcerpt(2, 4);
    host.querySelector<HTMLButtonElement>(".reference-excerpt-mark")!.click();
  };
  it("focuses the mention in the opened page", () => {
    vi.useFakeTimers();
    click();
    vi.advanceTimersByTime(200);
    expect(startEditing).toHaveBeenCalledOnce();
  });
  it("does not grab the editor after the user navigated elsewhere", () => {
    vi.useFakeTimers();
    click();
    vi.advanceTimersByTime(20);
    openPage("Somewhere else");
    vi.advanceTimersByTime(500);
    expect(startEditing).not.toHaveBeenCalled();
  });
});
