import { describe, expect, it, beforeEach, vi, afterEach } from "vitest";
import { backend } from "./backend";
import type { PageDto } from "./types";

/**
 * The deferred block-ref stamp, enumerated rather than sampled.
 *
 * GH #254 increment 3 spent verification rounds 4–11 on this one mechanism, and
 * every round had the same shape: a verifier constructed one interleaving, it
 * failed, I added one regression, and the fix opened the next corner. Nine
 * rounds produced nine hand-written tests, each pinning a single point in a
 * space nobody had enumerated. Sampling does not converge on a state machine; it
 * converges on the imagination of whoever is sampling.
 *
 * This enumerates the space instead. What made those rounds hard was never the
 * steady state — it was a mutation landing WHILE the retry's read was in flight,
 * so every case here holds the read open, applies the mutation, and only then
 * lets the read land. The incumbents are the three that actually refuse; a page
 * that does not refuse never defers anything and exercises none of this.
 *
 * The two invariants, stated once:
 *
 *   I1 NO RESURRECTION. If the target file is covered by a tombstone, or the
 *      graph moved on, its bytes must not be installed. Installing them puts a
 *      deleted page back, and `upsertPage` lifts the tombstone as it does so, so
 *      the stamp's own save then recreates the file the user deleted.
 *
 * WHAT THIS ACTUALLY CATCHES. A harness nobody has tested against real defects
 * is decoration, so each historical blocker was reintroduced and replayed:
 *
 *   round 3/5  no retry at all, drop on refusal ............... 336 cases fail
 *   round 8    name-level tombstone refuses the survivor ....... 14 cases fail
 *   round 9    request discarded under an unresolved delete .... 19 cases fail
 *   round 10   unknown file refuses at the WAIT boundary ........ 3 cases fail
 *   round 9b   stale tombstone path across a graph switch ...... NOT CAUGHT
 *
 * The last one spans TWO graphs, and this matrix models one; it is covered by
 * `does not carry a deleted file's path into the next graph` in
 * instanceReplacement.test.ts, which was necessity-gated separately. Three of
 * the four it does catch were invisible to earlier drafts of this file, and each
 * exposed a missing dimension rather than a missing assertion — the timing of
 * the mutation, the resolution of the delete, and progress-versus-activity.
 * That is the argument for replaying: the space you forgot to enumerate looks
 * exactly like a space with no bugs in it.
 *
 *   I2 NO SILENT LOSS. The `((uuid))` reference is ALREADY committed — the user
 *      typed it and it is on screen. If the stamp cannot happen now, the request
 *      must still be pending so it can happen later. Dropping it leaves a
 *      reference that resolves this session and dangles after a restart, with
 *      nothing to notice.
 */

const page = (name: string, path: string, raw: string): PageDto => ({
  name,
  kind: "page",
  title: name,
  pre_block: null,
  blocks: [{ raw, children: [] } as never],
  format: "md",
  read_only: false,
  path,
  rev: "rev-" + path,
});

const INCUMBENT = "pages/Target.md";
const TARGET = "pages/other/Target.md";
const UNRELATED = "pages/unrelated/Target.md";

/** How the request names the file it wants. Block autocomplete cannot name one. */
type Naming = "named" | "pathless";
/** Incumbent states that REFUSE — the only ones that defer anything. */
type Incumbent = "dirty" | "conflicted" | "leased";
/** What frees the incumbent, and so drives the retry. */
type Release = "lease" | "forget" | "save" | "clear-conflict";
/** What tombstone appears WHILE the retry's read is in flight. */
type Tomb = "none" | "pathless" | "exact-target" | "exact-other";
/** Whether the graph survives that same window. */
type Epoch = "same" | "switched";
/**
 * WHEN the mutation lands, relative to the retry. Two different guards decide:
 * `before-release` is judged by the pre-read check (may this even read?) and
 * `mid-read` by the post-read check (may these bytes install?). Their answers
 * for an unidentifiable file must differ, so a matrix covering only one ordering
 * misses every bug in the other — as this one did until it was replayed against
 * the defects rounds 9–11 actually found.
 */
type Timing = "before-release" | "mid-read";
/**
 * How the deletion RESOLVES. A tombstone goes up before the backend delete and
 * comes back down if it fails — core rejects an ambiguous by-name delete of a
 * duplicated page name. `lifted` is that failure, and also a page recreated.
 * Without this dimension the matrix cannot see a request discarded under a
 * tombstone that was about to disappear, which is exactly the round-9 defect it
 * failed to catch on its first replay.
 */
type Resolution = "stays" | "lifted";

type Case = {
  naming: Naming;
  incumbent: Incumbent;
  release: Release;
  tomb: Tomb;
  epoch: Epoch;
  timing: Timing;
  resolution: Resolution;
};

const NAMINGS: Naming[] = ["named", "pathless"];
const INCUMBENTS: Incumbent[] = ["dirty", "conflicted", "leased"];
const RELEASES: Release[] = ["lease", "forget", "save", "clear-conflict"];
const TOMBS: Tomb[] = ["none", "pathless", "exact-target", "exact-other"];
const EPOCHS: Epoch[] = ["same", "switched"];
const TIMINGS: Timing[] = ["before-release", "mid-read"];
const RESOLUTIONS: Resolution[] = ["stays", "lifted"];

/** Which releases each incumbent can actually undergo. */
function releasable(incumbent: Incumbent, release: Release): boolean {
  if (release === "forget") return true; // any page can be forgotten
  if (release === "lease") return incumbent === "leased";
  if (release === "save") return incumbent === "dirty";
  return incumbent === "conflicted"; // clear-conflict
}

const cases: Case[] = [];
for (const naming of NAMINGS)
  for (const incumbent of INCUMBENTS)
    for (const release of RELEASES)
      for (const tomb of TOMBS)
        for (const epoch of EPOCHS)
          for (const timing of TIMINGS)
            for (const resolution of RESOLUTIONS) {
              if (!releasable(incumbent, release)) continue;
              // Nothing to resolve when no tombstone was ever raised.
              if (tomb === "none" && resolution === "lifted") continue;
              cases.push({ naming, incumbent, release, tomb, epoch, timing, resolution });
            }

const label = (c: Case) =>
  `${c.naming} | ${c.incumbent} freed by ${c.release} | ${c.tomb} tombstone ${c.timing}, ${c.resolution} | graph ${c.epoch}`;

describe("deferred block-ref stamp — enumerated interleavings (GH #254 increment 3)", () => {
  beforeEach(async () => {
    const { resetStore, clearAllEditorLeases } = await import("./store");
    resetStore();
    clearAllEditorLeases();
  });

  afterEach(() => vi.restoreAllMocks());

  it("enumerates a space small enough to cover exhaustively", () => {
    // If this moves, the space changed: add the dimension deliberately rather
    // than discovering it through another verification round.
    expect(cases.length).toBe(336);
  });

  for (const c of cases) {
    it(label(c), async () => {
      const store = await import("./store");
      const persistence = await import("./persistence");
      const ui = await import("./ui");

      let release: (() => void) | undefined;
      store.loadRoutedPage(page("Target", INCUMBENT, "incumbent"));
      if (c.incumbent === "dirty") persistence.markDirty("Target");
      if (c.incumbent === "conflicted") {
        persistence.markDirty("Target");
        ui.markConflict("Target");
      }
      if (c.incumbent === "leased") release = store.takeEditorLease("Target");

      const targetDto = page("Target", TARGET, "target body");
      const byPath = vi.spyOn(backend(), "getPageByPath").mockResolvedValue(targetDto);
      const byName = vi.spyOn(backend(), "getPage").mockResolvedValue(targetDto as never);
      vi.spyOn(backend(), "savePage").mockResolvedValue("rev-2");
      vi.spyOn(backend(), "deletePage").mockResolvedValue(undefined as never);

      const uuid = "uuid-matrix";
      const namedPath = c.naming === "named" ? TARGET : undefined;
      await store.persistBlockRefTarget(uuid, "Target", "page", namedPath);

      // Every one of these incumbents refuses, so the request must be retained.
      // If this ever fails the matrix is testing nothing, so assert it outright.
      expect(
        store.hasPendingBlockRefStamp(uuid),
        `setup: a refusing incumbent must retain the request (${label(c)})`,
      ).toBe(true);

      // Hold the retry's read OPEN. This is the window every hard bug lived in.
      let land: (dto: unknown) => void = () => {};
      const pending = new Promise((r) => {
        land = r as (dto: unknown) => void;
      });
      byPath.mockReturnValue(pending as never);
      byName.mockReturnValue(pending as never);

      const mutate = () => {
        if (c.tomb === "pathless") persistence.tombstone("Target");
        if (c.tomb === "exact-target") persistence.tombstone("Target", TARGET);
        if (c.tomb === "exact-other") persistence.tombstone("Target", UNRELATED);
        if (c.epoch === "switched") {
          // A graph switch is BOTH, in this order, and graph.ts separates them
          // by synchronous code only — so no in-flight read can resume between
          // them. Modelling it as resetStore() alone tests a state production
          // never has.
          store.resetStore();
          ui.bumpGraphEpoch();
        }
      };

      if (c.timing === "before-release") mutate();

      // Free the incumbent → the retry fires and its read is now in flight.
      if (c.release === "lease") release?.();
      if (c.release === "forget") store.forgetPage("Target");
      if (c.release === "save") await persistence.flushPage("Target");
      if (c.release === "clear-conflict") {
        ui.clearConflict("Target");
        store.sweepReplaceable();
      }
      await new Promise((r) => setTimeout(r, 0));

      // ── the interleaving: mutate while that read is unresolved ──────────────
      if (c.timing === "mid-read") mutate();

      // ── and only now does it land ───────────────────────────────────────────
      land(targetDto);
      for (let i = 0; i < 4; i++) await new Promise((r) => setTimeout(r, 0));

      // The delete resolves: it failed, or the page came back. `deletePage`'s
      // failure path lifts the tombstone and announces, so mirror both.
      if (c.resolution === "lifted" && c.epoch === "same") {
        persistence.untombstone("Target");
        store.notifyPageBecameReplaceable("Target");
        for (let i = 0; i < 4; i++) await new Promise((r) => setTimeout(r, 0));
      }

      const installedTarget = store.doc.pages.find((p) => p.name === "Target")?.path === TARGET;
      const covered =
        c.resolution === "stays" && (c.tomb === "pathless" || c.tomb === "exact-target");

      // ── I1 ──────────────────────────────────────────────────────────────────
      if (covered && c.epoch === "same") {
        expect(installedTarget, `I1: installed a file the tombstone covers (${label(c)})`).toBe(
          false,
        );
      }
      if (c.epoch === "switched") {
        expect(
          installedTarget,
          `I1: installed into a graph that had already moved on (${label(c)})`,
        ).toBe(false);
      }

      // ── I2 ──────────────────────────────────────────────────────────────────
      // The request may only be gone if it was satisfied, its graph went away,
      // or its file is provably deleted.
      const stillPending = store.hasPendingBlockRefStamp(uuid);
      if (!installedTarget && c.epoch === "same" && !covered) {
        expect(
          stillPending,
          `I2: dropped a committed reference's request with nothing to resume it (${label(c)})`,
        ).toBe(true);

        // And "pending" must mean RESUMABLE. A request retained but unable to
        // read again is lost as completely as one that was deleted; it just
        // fails silently instead of visibly. This is the shape of the round-10
        // defect, where an unidentifiable file counted as covered at the WAIT
        // boundary and the request stopped reading forever.
        //
        // Only demanded when the page is genuinely replaceable. A lease still
        // held, or a page still dirty, is a correct reason to keep waiting — the
        // release itself announces, so those resume without a sweep.
        if (store.mayReplaceInstance("Target")) {
          store.sweepReplaceable();
          for (let i = 0; i < 3; i++) await new Promise((r) => setTimeout(r, 0));
          // PROGRESS, not activity. A request that reads, is refused, retains,
          // reads again forever is lost just as surely as one that stopped — it
          // simply burns reads while doing it. Asserting that it merely read
          // again let the round-8 defect through, where a name-level tombstone
          // refused the SURVIVING file on every attempt.
          expect(
            store.doc.pages.find((p) => p.name === "Target")?.path,
            `I2: a replaceable page with nothing covering it must let the request COMPLETE (${label(c)})`,
          ).toBe(TARGET);
        }
      }

      release?.();
    });
  }
});
