import { beforeEach, describe, expect, it, vi } from "vitest";

// GH #254 increment 2, adversarial implementation verification, finding 3.
//
// A force ("Keep mine") consumes its one-shot authority BEFORE every fallible
// validation, by design — a failed or replayed attempt must not be able to
// reuse it. If the backend then cannot coherently observe the winner it mints
// nothing and returns a `conflict_retry.*` code, which is deliberately NOT
// banner class.
//
// But the page already had a banner, and the force spent its token. Leaving
// that banner up strands the user completely: "Keep mine" can never work again,
// the transient retry refuses to run while a page is conflicted, and the only
// live button — "Use disk version" — discards their unsaved edits.

const calls: { name: string; force: boolean }[] = [];
let nextResult: (() => Promise<string>) | null = null;

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
    name, kind: "page", title: name, pre_block: null, blocks: [],
    format: "markdown", path: `pages/${name}.md`, guide: false, read_only: false,
  }),
}));

vi.mock("./backend", () => ({
  backend: () => ({
    savePage: (page: { name: string }, _baseRev: string | null, force: boolean) => {
      calls.push({ name: page.name, force });
      const result = nextResult;
      nextResult = null;
      return result ? result() : Promise.resolve("rev-after");
    },
  }),
}));

const conflicted = new Set<string>();
const toasts: string[] = [];
vi.mock("./ui", () => ({
  markConflict: (name: string) => conflicted.add(name),
  clearConflict: (name: string) => conflicted.delete(name),
  isConflicted: (name: string) => conflicted.has(name),
  conflicts: () => [...conflicted],
  bumpDataRev: () => {},
  bumpPageInventoryRev: () => {},
  pushToast: (message: string) => toasts.push(message),
}));

const { forceSave, markDirty, resetSaveState } = await import("./persistence");

// GH #254 increment 2, fourth correction-delta re-verification, HIGH. A Direct
// save error is "{bounded code}: {raw error}", and raw errors carry graph
// PATHS — so a page the user is entitled to name `conflict_authority.notes` put
// that family marker inside an unrelated failure's message. A substring test
// then routed a permanent precheck failure into the authority handler, which
// deletes the epoch, re-dirties the page and fire-and-forgets another save: one
// user request became four backend calls and would have kept feeding the queue
// instead of reaching the bounded retry/toast path.
describe("a failure is classified by its code, not by the page's name", () => {
  beforeEach(() => {
    calls.length = 0;
    toasts.length = 0;
    conflicted.clear();
    nextResult = null;
    resetSaveState();
  });

  // Only the `conflict_authority` case fails against the pre-fix substring test:
  // its handler re-enqueues, so one request multiplied. The `conflict_retry` case
  // passed either way, because that handler schedules a BOUNDED retry — it is
  // here as a specification of the same rule for a family with the same shape,
  // not as fail-before evidence.
  for (const family of ["conflict_authority", "conflict_retry"]) {
    it(`does not read "${family}." out of a page path`, async () => {
      const name = `${family}.notes`;
      markDirty(name);
      nextResult = () => Promise.reject(new Error(
        `precheck.symlink: managed text entry is a symlink or reparse point: pages/${name}.md`
      ));

      expect(await forceSave(name)).toBe(false);

      // Exactly the one request, then the bounded transient path — not a
      // self-feeding chain of re-observations.
      await new Promise((resolve) => setTimeout(resolve, 50));
      expect(calls.length).toBe(1);
    });
  }

  // Fifth re-verification, LOW. An error with no bounded prefix used to be
  // returned whole, so prose that merely OPENS with a code-shaped token was
  // accepted as that family.
  it("does not read a family out of unprefixed prose", async () => {
    markDirty("Notes");
    nextResult = () => Promise.reject(new Error(
      "conflict_authority.spent while reporting an unrelated raw failure"
    ));

    expect(await forceSave("Notes")).toBe(false);

    await new Promise((resolve) => setTimeout(resolve, 50));
    expect(calls.length).toBe(1);
  });
});

describe("a tokenless force does not strand the page behind a spent banner", () => {
  beforeEach(() => {
    calls.length = 0;
    toasts.length = 0;
    conflicted.clear();
    nextResult = null;
    resetSaveState();
  });

  it("retracts the spent banner and lets the retry reach the backend", async () => {
    markDirty("Notes");
    conflicted.add("Notes"); // the banner the user is looking at
    nextResult = () => Promise.reject(new Error("conflict_retry.replace_pre_retirement: ..."));

    expect(await forceSave("Notes")).toBe(false);

    // The banner is gone: it stood on authority this attempt already spent.
    expect(conflicted.has("Notes")).toBe(false);

    // …and the retry actually calls the backend, which the conflicted gate
    // would otherwise forbid.
    await vi.waitFor(() => expect(calls.length).toBe(2));
    expect(calls[0]).toEqual({ name: "Notes", force: true });
    expect(calls[1].force).toBe(false);
  });

  it("re-raises a real conflict from the retry, with a fresh banner", async () => {
    markDirty("Notes");
    conflicted.add("Notes");
    nextResult = () => Promise.reject(new Error("conflict_retry.commit_recheck: ..."));

    await forceSave("Notes");
    expect(conflicted.has("Notes")).toBe(false);

    nextResult = () => Promise.reject(new Error("conflict"));
    await vi.waitFor(() => expect(calls.length).toBe(2));
    await vi.waitFor(() => expect(conflicted.has("Notes")).toBe(true));
  });

  // A banner-class conflict is unchanged: it mints authority, so its banner is
  // live and must stay up.
  it("leaves a banner-class conflict exactly as it was", async () => {
    markDirty("Notes");
    nextResult = () => Promise.reject(new Error("conflict"));

    expect(await forceSave("Notes")).toBe(false);

    expect(conflicted.has("Notes")).toBe(true);
    expect(calls.length).toBe(1);
  });
});
