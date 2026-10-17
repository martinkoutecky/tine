import { afterEach, describe, expect, it, vi } from "vitest";
import { startEditing } from "../editorController";
import { openPage, route } from "../router";
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

describe("OccurrenceControls second gate (GH #200 option A: only what the surface cannot show)", () => {
  it("hides the circles when the surface already shows every mention, shows them past it", () => {
    const shown = document.createElement("div");
    render(() => <OccurrenceControls evidence={evidence(3)} onOccurrence={() => {}} visible={3} />, shown);
    expect(shown.querySelector(".reference-occurrence-controls")).toBeNull();
    const past = document.createElement("div");
    render(() => <OccurrenceControls evidence={evidence(4)} onOccurrence={() => {}} visible={3} />, past);
    expect(past.querySelectorAll(".reference-occurrence-jump")).toHaveLength(4);
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

const marks = (host: HTMLElement) => [...host.querySelectorAll<HTMLButtonElement>(".reference-excerpt-mark")];
const circle = (host: HTMLElement, ordinal: number) =>
  host.querySelectorAll<HTMLButtonElement>(".reference-occurrence-jump")[ordinal - 1];

// GH #200 option A (Martin 2026-09-16, re-affirmed 2026-10-17 over GH #556
// items 1 and 3): the marks are the affordance; numbered circles exist only for
// mentions the excerpt cannot show (total > what the surface already shows).
describe("Unlinked reference occurrences (GH #200 option A: the highlight is the affordance)", () => {
  it("retires the numbered circles when the excerpt already shows every mention", () => {
    // A SHORT block with four mentions used to render four marks AND four
    // numbered circles saying the same thing twice.
    const host = renderExcerpt(4, 4);
    expect(marks(host)).toHaveLength(4);
    expect(host.querySelectorAll(".reference-occurrence-jump")).toHaveLength(0);
    expect(host.querySelector(".reference-occurrence-controls")).toBeNull();
    expect(host.textContent ?? "").not.toContain("4 mentions");
  });

  it("makes each highlighted mention a real button naming the page and the mention's ordinal", () => {
    const host = renderExcerpt(3, 4);
    expect(marks(host).map((mark) => mark.tagName)).toEqual(["BUTTON", "BUTTON", "BUTTON"]);
    expect(marks(host).map((mark) => mark.getAttribute("aria-label"))).toEqual([
      "Open mention 1 in Journal",
      "Open mention 2 in Journal",
      "Open mention 3 in Journal",
    ]);
  });

  it("shows no circles at the cap: every mention fits the excerpt", () => {
    // The excerpt caps at three windows; three far-apart mentions all fit.
    const host = renderExcerpt(3, 400);
    expect(marks(host)).toHaveLength(3);
    expect(host.querySelectorAll(".reference-occurrence-jump")).toHaveLength(0);
  });

  it("shows the circles only once mentions fall past the excerpt windows", () => {
    // One mention more than the cap: the fourth is unreachable without a circle.
    const host = renderExcerpt(4, 400);
    expect(marks(host).length).toBeLessThan(4);
    expect(host.querySelector(".reference-occurrence-controls")).not.toBeNull();
    expect(host.textContent ?? "").toContain("4 mentions");
    expect(host.querySelectorAll(".reference-occurrence-jump")).toHaveLength(4);
  });

  it("a circle jumps to its mention in the source page (it does not re-pick a highlight)", () => {
    vi.useFakeTimers();
    const host = renderExcerpt(6, 400);
    circle(host, 6).click();
    vi.advanceTimersByTime(200);
    expect(route()).toMatchObject({ kind: "page", name: "Journal", pageKind: "journal" });
    expect(startEditing).toHaveBeenCalledOnce();
    expect(circle(host, 6).getAttribute("aria-pressed")).toBeNull();
  });

  it("marks every mention once the block is expanded, and retires the circles with it", () => {
    const host = renderExcerpt(6, 400);
    host.querySelector<HTMLButtonElement>(".reference-show-full")!.click();
    expect(marks(host)).toHaveLength(6);
    expect(host.querySelector(".reference-occurrence-controls")).toBeNull();
    expect(host.querySelector(".reference-excerpt-text")!.textContent).toBe(blockWith(6, 400).block.raw);
  });

  it("a single mention shows no numbered circles", () => {
    const host = renderExcerpt(1, 4);
    expect(host.querySelector(".reference-occurrence-controls")).toBeNull();
    expect(marks(host)).toHaveLength(1);
  });
});

// GH #556 item 7: the block's bullet is the block-level way into the source.
describe("Unlinked reference rows: the bullet opens the source block (GH #556)", () => {
  it("clicking the bullet navigates to the block without grabbing the editor", () => {
    vi.useFakeTimers();
    const host = renderExcerpt(2, 4);
    host.querySelector<HTMLButtonElement>(".reference-excerpt-bullet")!.click();
    vi.advanceTimersByTime(500);
    expect(route()).toMatchObject({ kind: "page", name: "Journal", pageKind: "journal" });
    expect(startEditing).not.toHaveBeenCalled();
  });
});

describe("buildFullMarkedSegments", () => {
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
  it("highlights a bare page-property atom with native source evidence (Kalem parity)", () => {
    const raw = "related:: Alpha，Beta";
    const item: ReferenceBlockEvidence = {
      block_id: "page-property:Page:source",
      occurrences: [{ matched_name: "Beta", canonical: "beta", kind: "explicit",
        span: { start: 16, end: 20 }, rule: "implicit_linkable_property" }],
    };
    const host = document.createElement("div");
    document.body.appendChild(host);
    const dispose = render(() => <ReferenceExcerptBlocks
      blocks={[{ id: item.block_id, raw, page_property: true, children: [] } as unknown as BlockDto]}
      evidence={[item]} page="Source" kind="page" />, host);
    try {
      expect(host.querySelector(".reference-excerpt-text")?.textContent).toBe(raw);
      expect(marks(host).map(mark => mark.textContent)).toEqual(["Beta"]);
    } finally { dispose(); }
  });

  it("shows an Org headline tag as an explicit reference and highlights its source name (Kalem parity)", () => {
    const raw = "Headline :Plugins:";
    const item: ReferenceBlockEvidence = {
      block_id: "org-headline",
      occurrences: [{ matched_name: "Plugins", canonical: "plugins", kind: "explicit",
        span: { start: 10, end: 17 }, rule: "explicit_tag" }],
    };
    const host = document.createElement("div");
    document.body.appendChild(host);
    const dispose = render(() => <ReferenceExcerptBlocks
      blocks={[{ id: "org-headline", raw, children: [] } as unknown as BlockDto]}
      evidence={[item]} page="Source" kind="page" />, host);
    try {
      expect(host.querySelector(".reference-excerpt-text")?.textContent).toBe(raw);
      expect(marks(host).map(mark => mark.textContent)).toEqual(["Plugins"]);
    } finally { dispose(); }
  });

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
