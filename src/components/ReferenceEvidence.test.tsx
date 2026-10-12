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

// GH #556 items 1 and 3 (the issue's "Expected behavior" list): one mention is
// highlighted by default, not every mention; the numbered circles choose which,
// in place, instead of navigating.
describe("Unlinked reference occurrences (GH #556: one highlighted mention, chosen by the circles)", () => {
  it("highlights only the first mention by default, however many fit the excerpt", () => {
    // Four short-gap mentions used to render four highlights.
    const host = renderExcerpt(4, 4);
    expect(marks(host).map((mark) => mark.getAttribute("aria-label"))).toEqual(["Open mention 1 in Journal"]);
    expect(host.querySelector(".reference-excerpt-text")!.textContent).toContain("Books");
  });

  it("offers a numbered circle per mention, with the highlighted one marked pressed", () => {
    const host = renderExcerpt(3, 4);
    expect(host.textContent ?? "").toContain("3 mentions");
    const circles = [...host.querySelectorAll<HTMLButtonElement>(".reference-occurrence-jump")];
    expect(circles.map((c) => c.getAttribute("aria-pressed"))).toEqual(["true", "false", "false"]);
  });

  it("a circle moves the highlight to that mention without leaving the view", () => {
    vi.useFakeTimers();
    const host = renderExcerpt(3, 4);
    circle(host, 3).click();
    expect(marks(host).map((mark) => mark.getAttribute("aria-label"))).toEqual(["Open mention 3 in Journal"]);
    expect(circle(host, 3).getAttribute("aria-pressed")).toBe("true");
    expect(circle(host, 1).getAttribute("aria-pressed")).toBe("false");
    vi.advanceTimersByTime(500);
    expect(startEditing).not.toHaveBeenCalled();
    expect(route()).toMatchObject({ kind: "journals" });
  });

  it("re-centres the excerpt on a mention that was outside every window", () => {
    // 6 mentions 400 characters apart: the excerpt shows only the first one's neighbourhood.
    const host = renderExcerpt(6, 400);
    expect(marks(host).map((mark) => mark.getAttribute("aria-label"))).toEqual(["Open mention 1 in Journal"]);
    circle(host, 6).click();
    expect(marks(host).map((mark) => mark.getAttribute("aria-label"))).toEqual(["Open mention 6 in Journal"]);
  });

  it("the full block keeps only the chosen mention highlighted", () => {
    const host = renderExcerpt(6, 400);
    circle(host, 4).click();
    host.querySelector<HTMLButtonElement>(".reference-show-full")!.click();
    expect(marks(host).map((mark) => mark.getAttribute("aria-label"))).toEqual(["Open mention 4 in Journal"]);
    expect(host.querySelector(".reference-excerpt-text")!.textContent).toBe(blockWith(6, 400).block.raw);
  });

  it("a single mention shows no numbered circles", () => {
    const host = renderExcerpt(1, 4);
    expect(host.querySelector(".reference-occurrence-controls")).toBeNull();
    expect(marks(host)).toHaveLength(1);
  });

  it("the highlighted mention still opens that mention in its page", () => {
    vi.useFakeTimers();
    const host = renderExcerpt(3, 4);
    circle(host, 2).click();
    marks(host)[0].click();
    vi.advanceTimersByTime(200);
    expect(route()).toMatchObject({ kind: "page", name: "Journal", pageKind: "journal" });
    expect(startEditing).toHaveBeenCalledOnce();
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
