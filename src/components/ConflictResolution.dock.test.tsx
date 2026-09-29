import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { render } from "solid-js/web";
import { backend } from "../backend";
import type { ConflictObject, DiffRow } from "../types";

// Master 61ea6600c, ported in meaning: the in-page review sits at the top of
// the page and scrolled away with it, so on a phone a conflict was invisible
// until the user happened to scroll up. Once the panel is ENTIRELY above the
// viewport a slim bar pins to the pane; tapping it unrolls the SAME panel node
// (decisions survive) as a sheet; Escape or scrolling back folds it. jsdom has
// no IntersectionObserver, so these fire one by hand.

vi.mock("../document", async (importOriginal) => ({
  ...await importOriginal<typeof import("../document")>(),
  isDirty: () => false,
  isSaving: () => false,
  isConflicted: () => false,
}));
import { PageConflictResolution } from "./ConflictResolution";

const tick = () => new Promise((resolve) => setTimeout(resolve, 0));
const settle = async () => { for (let i = 0; i < 6; i++) await tick(); };
const view = (text: string) => ({ uuid: "", text, child_count: 0 });

const markers: ConflictObject = {
  id: "markers:pages/Merged.md",
  source: "vcs-markers",
  page_name: "Merged",
  page_path: "pages/Merged.md",
  kind: "page",
  sides: [{ role: "mine", label: "HEAD" }, { role: "theirs", label: "feature" }],
  block_conflicts: 2,
  markers: ["<<<<<<<", "=======", ">>>>>>>"],
};
const rows: DiffRow[] = [
  { id: "0", kind: "modified", mine: view("A mine"), theirs: view("A theirs"), children: [], verdict: "both-changed" },
  { id: "1", kind: "modified", mine: view("B mine"), theirs: view("B theirs"), children: [], verdict: "both-changed" },
];

class ManualIO {
  static instances: ManualIO[] = [];
  constructor(readonly callback: IntersectionObserverCallback) { ManualIO.instances.push(this); }
  observe(): void {}
  unobserve(): void {}
  disconnect(): void {}
  takeRecords(): IntersectionObserverEntry[] { return []; }
  fire(isIntersecting: boolean, top: number): void {
    this.callback([{ isIntersecting, boundingClientRect: { top } } as unknown as IntersectionObserverEntry], this as unknown as IntersectionObserver);
  }
}

const realIO = globalThis.IntersectionObserver;
beforeEach(() => {
  (globalThis as { IntersectionObserver?: unknown }).IntersectionObserver = ManualIO as unknown as typeof IntersectionObserver;
  vi.spyOn(backend(), "vcsMarkerConflictDiff").mockResolvedValue({
    mine_label: "HEAD", theirs_label: "feature", regions: 2,
    diff: { base_rev: "r", conflict_rev: "c", rows, mine_pre: null, theirs_pre: null, pre_differs: false, blocks_identical: false },
  });
});
afterEach(() => {
  document.body.innerHTML = "";
  ManualIO.instances = [];
  (globalThis as { IntersectionObserver?: unknown }).IntersectionObserver = realIO;
  vi.restoreAllMocks();
});

async function mounted() {
  const host = document.createElement("div");
  document.body.append(host);
  const dispose = render(() => <PageConflictResolution conflict={markers} />, host);
  await settle();
  const io = ManualIO.instances.at(-1);
  expect(io, "the panel observes whether it is on screen").toBeDefined();
  return { host, dispose, io: io! };
}
const bar = (host: HTMLElement) => host.querySelector<HTMLButtonElement>(".page-conflict-dockbar");

describe("the conflict dock (master 61ea6600c)", () => {
  it("shows no bar while the panel is in view", async () => {
    const { host, dispose, io } = await mounted();
    io.fire(true, 120);
    await settle();
    expect(host.querySelector(".page-conflict-dock")).toBeNull();
    expect(host.querySelector(".page-conflict-slot .page-conflict")).not.toBeNull();
    dispose();
  });

  it("pins a slim bar with the title and count once the panel is entirely above the viewport", async () => {
    const { host, dispose, io } = await mounted();
    io.fire(false, -40);
    await settle();
    expect(bar(host)!.getAttribute("aria-expanded")).toBe("false");
    expect(bar(host)!.textContent).toContain("Unresolved merge from your version-control tool");
    expect(bar(host)!.textContent).toContain("2 to review");
    expect(host.querySelector(".page-conflict-sheet")).toBeNull();
    expect(host.querySelector(".page-conflict-slot .page-conflict")).not.toBeNull();
    dispose();
  });

  it("does not dock for a panel still below the fold", async () => {
    const { host, dispose, io } = await mounted();
    io.fire(false, 900);
    await settle();
    expect(host.querySelector(".page-conflict-dock")).toBeNull();
    dispose();
  });

  it("unrolls the SAME panel node into the sheet and returns it on Escape", async () => {
    const { host, dispose, io } = await mounted();
    const panel = host.querySelector(".page-conflict")!;
    io.fire(false, -40);
    await settle();
    bar(host)!.click();
    await settle();
    const sheet = host.querySelector(".page-conflict-sheet")!;
    expect(sheet.querySelector(".page-conflict")).toBe(panel);
    expect(host.querySelector(".page-conflict-slot .page-conflict")).toBeNull();
    expect(bar(host)!.getAttribute("aria-expanded")).toBe("true");
    sheet.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape", bubbles: true }));
    await settle();
    expect(host.querySelector(".page-conflict-sheet")).toBeNull();
    expect(host.querySelector(".page-conflict-slot .page-conflict")).toBe(panel);
    dispose();
  });

  it("keeps a decision made in the sheet after scrolling back undocks it", async () => {
    const { host, dispose, io } = await mounted();
    io.fire(false, -40);
    await settle();
    bar(host)!.click();
    await settle();
    host.querySelector<HTMLButtonElement>('.page-conflict-sheet [data-row-id="1"] .sync-merge-seg[data-decision="theirs"]')!.click();
    await settle();
    io.fire(true, 60);
    await settle();
    expect(host.querySelector(".page-conflict-dock")).toBeNull();
    expect(host.querySelector('.page-conflict-slot [data-row-id="1"] .sync-merge-seg.active')!.getAttribute("data-decision")).toBe("theirs");
    dispose();
  });
});
