import { beforeEach, afterEach, describe, expect, it, vi } from "vitest";

// Direct Files data-safety audit, finding 9. The save debounce re-arms on every
// keystroke and has no ceiling, so a fluent typing burst is never written to
// disk until the typist pauses. At ~7 chars/second — ordinary prose speed, well
// short of touch-typing — the 400 ms timer is re-armed before it ever fires, and
// a crash or power loss during a long paragraph loses the whole paragraph.
//
// The fix is a MAXIMUM delay measured from the FIRST dirty mark of a burst: the
// debounce still coalesces, but it can no longer be postponed indefinitely.

const saved: string[] = [];

vi.mock("./store", () => ({
  doc: { loaded: true, pages: [] },
  // The save path acquires the editor's activation before building the DTO
  // (GH #254 increment 3). These stubs say "this editor already holds one", so
  // these tests keep exercising the conflict/barrier behaviour they are about.
  editorActivationFor: () => 1,
  setEditorActivation: () => {},
  setProspectiveTarget: () => {},
  bumpEditGeneration: () => {},
  peekPageInstanceGeneration: () => 1,
  notifyPageBecameReplaceable: () => {},
  pageByName: (name: string) => ({ name }),
  pageInstanceGeneration: () => 1,
  pageToDto: (name: string) => ({
    name,
    kind: "page",
    title: name,
    pre_block: null,
    blocks: [],
    format: "markdown",
    path: `pages/${name}.md`,
    guide: false,
    read_only: false,
  }),
}));

vi.mock("./backend", () => ({
  backend: () => ({
    savePage: (page: { name: string }) => {
      saved.push(page.name);
      return Promise.resolve("rev-after");
    },
  }),
}));

vi.mock("./ui", () => ({
  markConflict: () => {},
  isConflicted: () => false,
  conflicts: () => [],
  bumpDataRev: () => {},
  bumpPageInventoryRev: () => {},
  pushToast: () => {},
}));

const { markDirty, resetSaveState } = await import("./persistence");

describe("the save debounce has a ceiling", () => {
  beforeEach(() => {
    saved.length = 0;
    resetSaveState();
    vi.useFakeTimers();
  });
  afterEach(() => {
    vi.useRealTimers();
  });

  // 300 ms between keystrokes is slower than most people type and still never
  // lets the 400 ms debounce elapse.
  it("writes during a continuous typing burst instead of waiting for a pause", async () => {
    for (let keystroke = 0; keystroke < 40; keystroke++) {
      markDirty("Notes");
      await vi.advanceTimersByTimeAsync(300);
    }

    // 12 seconds of uninterrupted typing.
    expect(saved).toContain("Notes");
  });

  // The ceiling must not defeat the debounce: a burst that fits inside it still
  // costs exactly one write, not one per keystroke.
  it("still coalesces a short burst into a single write", async () => {
    for (let keystroke = 0; keystroke < 4; keystroke++) {
      markDirty("Notes");
      await vi.advanceTimersByTimeAsync(100);
    }
    await vi.advanceTimersByTimeAsync(1000);

    expect(saved).toEqual(["Notes"]);
  });

  // And the clock restarts per burst: after a save, a later burst gets its own
  // full debounce rather than inheriting an already-expired ceiling and writing
  // on the very first keystroke.
  it("measures the ceiling from the first dirty mark of each burst", async () => {
    markDirty("Notes");
    await vi.advanceTimersByTimeAsync(1000);
    expect(saved).toEqual(["Notes"]);

    markDirty("Notes");
    await vi.advanceTimersByTimeAsync(100);
    expect(saved).toEqual(["Notes"]);
  });
});
