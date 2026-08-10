import { beforeEach, describe, expect, it, vi } from "vitest";

// The cross-page move barrier is a data-loss guard, so its interaction with
// "keep mine" is worth driving through the real save path rather than a helper.
// persistence.ts depends on store/backend/ui only through narrow call-time
// seams, so mocking those three reaches `doSave` itself.

const saved: { name: string; force: boolean; observation: number | null }[] = [];
// The authority half of the real backend: one live token at a time, revoked by
// any external observation, minted by a guarded refusal, and spendable only by a
// force that names it.
let live: number | null = null;
let nextEpoch = 11;
const refuseGuarded = new Set<string>();

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
  retryPendingBlockRefStamps: () => {},
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
    savePage: (
      page: { name: string },
      _baseRev: string | null,
      force: boolean,
      observation: number | null,
    ) => {
      saved.push({ name: page.name, force: !!force, observation: observation ?? null });
      if (force) {
        if (observation === null || observation !== live) {
          return Promise.reject(new Error("conflict_authority.superseded: ..."));
        }
        live = null;
        return Promise.resolve("rev-forced");
      }
      if (refuseGuarded.has(page.name)) {
        live = nextEpoch;
        nextEpoch += 1;
        return Promise.reject(new Error(`conflict:${live}`));
      }
      return Promise.resolve("rev-after");
    },
  }),
}));

const conflicted = new Set<string>();
vi.mock("./ui", () => ({
  markConflict: (name: string) => conflicted.add(name),
  clearConflict: (name: string) => conflicted.delete(name),
  isConflicted: (name: string) => conflicted.has(name),
  conflicts: () => [...conflicted],
  bumpDataRev: () => {},
  bumpPageInventoryRev: () => {},
  pushToast: () => {},
}));

const { forceSave, holdSourcesForDest, markDirty, flushPage, reconcileExternalChange, resetSaveState } = await import("./persistence");

describe("cross-page move barrier vs keep-mine", () => {
  beforeEach(() => {
    saved.length = 0;
    conflicted.clear();
    live = null;
    nextEpoch = 11;
    refuseGuarded.clear();
    resetSaveState();
  });

  // Before GH-audit N1 was fixed, force-save could never reach the backend, so
  // `force` bypassing this barrier was latent. It is live now: a page can be
  // BOTH a source of an in-flight cross-page move and conflicted, and the two
  // rules guard different things. Overriding the conflict must not also
  // override the barrier — that writes the moved block out of existence in the
  // source before it is durable in the destination.
  it("defers a keep-mine on a held source, then applies it on release", async () => {
    holdSourcesForDest("Dest", ["Source"]);
    conflicted.add("Source");

    const overridden = await forceSave("Source");

    expect(overridden).toBe(false);
    expect(saved).toEqual([]);

    // The destination write lands; its sources are freed.
    markDirty("Dest");
    await flushPage("Dest");

    // Give the re-issued forced save its microtask.
    await new Promise((resolve) => setTimeout(resolve, 0));

    expect(saved.map((entry) => entry.name)).toContain("Source");
    expect(saved.find((entry) => entry.name === "Source")?.force).toBe(true);
  });

  // GH #254 increment 2, third correction-delta re-verification, HIGH. A held
  // source can be conflicted too: `prepareCrossPageSources` aborts a move on an
  // unresolved conflict but checks only dirty/saving, and a banner has already
  // taken the page out of `dirty`. While the barrier holds it, a later external
  // write revokes the token behind that visible banner and the re-observation
  // cannot mint a replacement — it returns at the barrier before reaching the
  // backend, by design, because the barrier is absolute.
  //
  // So the re-arm has to happen on the other side of the barrier: the deferred
  // click is re-issued with the epoch the USER clicked under, the backend refuses
  // it as superseded, and that refusal re-observes and raises a banner the user
  // can act on. What must never happen is the click silently adopting whatever
  // epoch is current by release time — that would discard a winner nobody saw.
  it("re-arms a held source whose authority was revoked behind the barrier", async () => {
    refuseGuarded.add("Source");
    markDirty("Source");
    await flushPage("Source"); // refused: banner up, epoch 11
    expect(conflicted.has("Source")).toBe(true);
    expect(live).toBe(11);

    holdSourcesForDest("Dest", ["Source"]);
    live = null; // a later external write; the native watcher revokes

    // The re-observation cannot help while the barrier holds — and must not try.
    await reconcileExternalChange("Source");
    expect(saved.filter((entry) => entry.name === "Source").length).toBe(1);

    expect(await forceSave("Source")).toBe(false); // deferred by the barrier

    markDirty("Dest");
    await flushPage("Dest"); // the destination lands; sources are freed

    await vi.waitFor(() => {
      const forced = saved.filter((entry) => entry.name === "Source" && entry.force);
      expect(forced.length).toBeGreaterThanOrEqual(1);
      // It presented the epoch of the banner the user clicked, not a newer one.
      expect(forced[0].observation).toBe(11);
    });
    // Refused as superseded — and the refusal re-observed, so there is a live
    // banner again rather than a dead one.
    await vi.waitFor(() => expect(live).not.toBeNull());
    expect(conflicted.has("Source")).toBe(true);
    expect(await forceSave("Source")).toBe(true);
  });
});
