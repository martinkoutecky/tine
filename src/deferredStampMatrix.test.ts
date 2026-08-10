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
 * is decoration, so every historical blocker is reintroduced and replayed:
 *
 *   round 3/5  no retry at all, drop on refusal ............... 636 cases fail
 *   round 12b  stamp never made durable ...................... 312 cases fail
 *   round 12   render epoch treated as a graph change ........ 156 cases fail
 *   round 9    request discarded under an unresolved delete ... 76 cases fail
 *   round 8    name-level tombstone refuses the survivor ...... 56 cases fail
 *   round 10   unknown file refuses at the WAIT boundary ...... 12 cases fail
 *   round 9b   stale tombstone path across a graph switch ..... NOT CAUGHT
 *
 * The last spans TWO graphs and this models one; it keeps its own
 * necessity-gated test in instanceReplacement.test.ts.
 *
 * Every correction to this file came from a replay failing, never from reading
 * it. In order: a missing TIMING dimension (round 10's defect lives in the
 * pre-read guard, and only mid-read mutations were modelled); a missing
 * RESOLUTION dimension (no case ever lifted a tombstone, so a failed delete was
 * unreachable); an oracle asserting activity instead of PROGRESS (a request that
 * reads, is refused and reads again forever passed); an oracle that accepted
 * "the page loaded" as the outcome when the outcome is a DURABLE reference; 112
 * cases whose release did not actually free the page, so the retry they claimed
 * to hold open never started; a save-chain entry leaking across cases and
 * silently skipping the progress assertion suite-wide; and an oracle that asked
 * `isTombstonedFile` whether a file was deleted — the very predicate round 8's
 * defect corrupts, which took that replay from 18 failures to zero.
 *
 * The lesson worth keeping: a matrix that is green proves nothing until its
 * failures have been demonstrated. Each of those seven defects made it report
 * MORE coverage, not less.
 *
 *   I2 NO SILENT LOSS. The `((uuid))` reference is ALREADY committed — the user
 *      typed it and it is on screen. If the stamp cannot happen now, the request
 *      must still be pending so it can happen later. Dropping it leaves a
 *      reference that resolves this session and dangles after a restart, with
 *      nothing to notice.
 */

// A REAL uuid: `ensureStableBlockId` reuses the block's id only when it parses
// as one, and otherwise mints a fresh random uuid — so a placeholder makes the
// stamp unobservable and the oracle silently weak.
const UUID = "123e4567-e89b-12d3-a456-426614174000";

const page = (name: string, path: string, raw: string, blockId?: string): PageDto => ({
  name,
  kind: "page",
  title: name,
  pre_block: null,
  blocks: [{ raw, children: [], ...(blockId ? { id: blockId } : {}) } as never],
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
type Incumbent = "dirty" | "conflicted" | "leased" | "saving";
/** What frees the incumbent, and so drives the retry. */
type Release = "lease" | "forget" | "save" | "resolve-conflict" | "save-lands";
/** What tombstone appears WHILE the retry's read is in flight. */
type Tomb = "none" | "pathless" | "exact-target" | "exact-other";
/** Whether the graph survives that same window. */
type Epoch = "same" | "switched" | "render-bumped";
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
const INCUMBENTS: Incumbent[] = ["dirty", "conflicted", "leased", "saving"];
const RELEASES: Release[] = ["lease", "forget", "save", "resolve-conflict", "save-lands"];
const TOMBS: Tomb[] = ["none", "pathless", "exact-target", "exact-other"];
const EPOCHS: Epoch[] = ["same", "switched", "render-bumped"];
const TIMINGS: Timing[] = ["before-release", "mid-read"];
const RESOLUTIONS: Resolution[] = ["stays", "lifted"];

/**
 * Which releases each incumbent can actually undergo — and every one of these
 * must LEAVE THE PAGE REPLACEABLE, or the case tests nothing.
 *
 * Round 12 found 112 of the previous 336 cases doing exactly that: clearing the
 * conflict left the page still dirty, and forgetting a leased page left the
 * lease held, so the retry those cases claimed to hold open never started. A
 * matrix that silently does nothing is worse than no matrix, because it reports
 * coverage it does not have.
 */
const RELEASES_FOR: Record<Incumbent, Release[]> = {
  dirty: ["save", "forget"],
  conflicted: ["resolve-conflict", "forget"],
  leased: ["lease", "forget"],
  saving: ["save-lands", "forget"],
};
function releasable(incumbent: Incumbent, release: Release): boolean {
  return RELEASES_FOR[incumbent].includes(release);
}

/**
 * Combinations that cannot occur, with the reason each is impossible. Stated
 * rather than silently dropped: an exclusion is a claim about the product, and a
 * wrong one hides bugs exactly the way a missing dimension does.
 */
function impossible(c: Case): string | null {
  if (c.release === "save" && c.timing === "before-release" && c.tomb !== "none") {
    // `doSave` no-ops on a tombstoned page by design — you do not write a page
    // the user just deleted — so "freed by saving" cannot happen once one is up.
    //
    // NOTE: the tombstone is checked BY NAME there, so an `exact-other`
    // tombstone also blocks saving a surviving file that merely shares the name.
    // That is the round-8 defect class living on in the SAVE path; it predates
    // this increment and is recorded as a follow-up rather than fixed here.
    return "a tombstoned page's save is a no-op, so it cannot free the page";
  }
  return null;
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
              if (impossible({ naming, incumbent, release, tomb, epoch, timing, resolution })) continue;
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
    expect(cases.length).toBe(636);
  });

  for (const c of cases) {
    it(label(c), async () => {
      const store = await import("./store");
      const persistence = await import("./persistence");
      const ui = await import("./ui");

      let release: (() => void) | undefined;
      let landSave: (rev: unknown) => void = () => {};
      store.loadRoutedPage(page("Target", INCUMBENT, "incumbent"));
      if (c.incumbent === "dirty") persistence.markDirty("Target");
      if (c.incumbent === "conflicted") {
        // Conflicted is NOT dirty. `doSave` clears the dirty mark before it
        // awaits the IPC, so a save that comes back conflicted leaves the page
        // conflicted-and-clean; `reloadDisposition` refuses on `isConflicted`
        // alone. Setting both modelled a state the product never produces, and
        // made "Use disk version" leave the page still refusing.
        ui.markConflict("Target");
      }
      if (c.incumbent === "leased") release = store.takeEditorLease("Target");

      // The target block carries the referenced uuid AS ITS ID, so the ref can
      // actually resolve and the stamp can actually be observed. Without this
      // the suite proves only that a page loaded — round 12 showed deleting
      // `ensureStableBlockId` left all 337 cases passing.
      const targetDto = page("Target", TARGET, "target body", UUID);
      const byPath = vi.spyOn(backend(), "getPageByPath").mockResolvedValue(targetDto);
      const byName = vi.spyOn(backend(), "getPage").mockResolvedValue(targetDto as never);
      const saved = vi.spyOn(backend(), "savePage").mockResolvedValue("rev-2");
      vi.spyOn(backend(), "deletePage").mockResolvedValue(undefined as never);

      if (c.incumbent === "saving") {
        // A save in flight: `doSave` clears dirty BEFORE awaiting the IPC, so
        // the page is neither dirty nor conflicted yet its edit is not durable.
        // `reloadDisposition` refuses on `isSaving` for exactly that window, and
        // it is reachable independently of the other three.
        saved.mockReturnValue(
          new Promise((r) => {
            landSave = r as (rev: unknown) => void;
          }) as never,
        );
        persistence.markDirty("Target");
        void persistence.flushPage("Target");
        for (let i = 0; i < 40 && persistence.isSaving("Target"); i++) {
          await new Promise((r) => setTimeout(r, 0));
        }
      }

      const uuid = UUID;
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
      // ONCE, not permanently: the save path reads too, and hijacking every
      // read deadlocks the release itself rather than testing it.
      byPath.mockImplementationOnce(() => pending as never);
      byName.mockImplementationOnce(() => pending as never);

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
        if (c.epoch === "render-bumped") {
          // NOT a graph switch: `setTypographyMode`, the journal title format,
          // and several settings bump the render epoch to repaint open pages
          // while the graph stays exactly where it was. Work keyed off the epoch
          // is destroyed by a user toggling a display preference — which is what
          // round 12 reproduced.
          ui.setTypographyMode(ui.typographyMode() === "render" ? "off" : "render");
        }
      };

      if (c.timing === "before-release") mutate();

      // Free the incumbent → the retry fires and its read is now in flight.
      if (c.release === "lease") release?.();
      if (c.release === "forget") {
        store.forgetPage("Target");
        if (c.incumbent === "saving") {
          // Let the in-flight save finish. Leaving it hanging leaks a save-chain
          // entry into every later case, where `isSaving` then reports true for
          // a page that is not saving — which silently skipped the progress
          // assertion across the whole suite and cost this matrix its round-10
          // coverage without a single test going red.
          saved.mockResolvedValue("rev-2");
          landSave("rev-2");
          await new Promise((r) => setTimeout(r, 0));
        }
        // A forgotten page's editor is gone, so its component released too.
        // Leaving the lease held made the round-12 vacuity: nothing replaceable,
        // so nothing to retry.
        release?.();
        release = undefined;
      }
      if (c.release === "save") {
        // Not awaited: the retry fires from this save's own cleanup, and its
        // read is deliberately held open — awaiting the flush would wait on work
        // that is waiting on the test.
        void persistence.flushPage("Target");
        await new Promise((r) => setTimeout(r, 0));
      }
      if (c.release === "resolve-conflict") {
        // The real "Use disk version" route: adopt the disk bytes, which clears
        // the dirty mark, THEN drop the banner. Clearing the banner alone left
        // the page dirty and therefore still refusing.
        store.reloadPage(page("Target", INCUMBENT, "disk winner"));
        ui.clearConflict("Target");
        store.sweepReplaceable();
      }
      if (c.release === "save-lands") {
        saved.mockResolvedValue("rev-2");
        landSave("rev-2");
        await new Promise((r) => setTimeout(r, 0));
      }
      await new Promise((r) => setTimeout(r, 0));

      // Non-vacuity: every release above exists to make the page replaceable
      // again. If one does not, the case that follows proves nothing, and a
      // matrix that silently proves nothing is worse than no matrix — it reports
      // coverage it does not have.
      expect(
        store.mayReplaceInstance("Target"),
        `setup: "${c.release}" must actually leave the page replaceable (${label(c)})`,
      ).toBe(true);

      // ── the interleaving: mutate while that read is unresolved ──────────────
      if (c.timing === "mid-read") mutate();

      // ── and only now does it land ───────────────────────────────────────────
      land(targetDto);
      for (let i = 0; i < 4; i++) await new Promise((r) => setTimeout(r, 0));

      // The delete resolves: it failed, or the page came back. `deletePage`'s
      // failure path lifts the tombstone and announces, so mirror both.
      if (c.resolution === "lifted" && c.epoch !== "switched") {
        persistence.untombstone("Target");
        store.notifyPageBecameReplaceable("Target");
        for (let i = 0; i < 4; i++) await new Promise((r) => setTimeout(r, 0));
      }

      const installedTarget = store.doc.pages.find((p) => p.name === "Target")?.path === TARGET;
      // SATISFIED means the reference became durable — the block carries its
      // `id::` and that reached a save. "The page loaded" is not the outcome the
      // user needs; round 12 deleted `ensureStableBlockId` and every case still
      // passed, because installation was standing in for the stamp.
      const stamped = () =>
        installedTarget
        && !!store.resolveBlockRef({ uuid: UUID, page: "Target", pageKind: "page" })
        && saved.mock.calls.some((call) => JSON.stringify(call[0] ?? "").includes(UUID));
      // Whether the TARGET FILE is genuinely deleted — derived from what this
      // case did, never from the predicate under test. Asking
      // `isTombstonedFile` instead makes the oracle complicit: the round-8
      // defect IS that predicate answering "covered" for a file it should not,
      // so an oracle that trusts it excuses the very bug it exists to catch.
      // (It did: that replay went from 18 failures to zero.)
      //
      // Two things legitimately lift a tombstone before the assertions: an
      // explicit lift (failed delete / page recreated), and the reload inside
      // "Use disk version", whose `upsertPage` lifts it because a real page
      // exists again — but only when the tombstone predates that reload.
      const liftedByReload = c.release === "resolve-conflict" && c.timing === "before-release";
      const targetDeleted = c.tomb === "pathless" || c.tomb === "exact-target";
      const covered = targetDeleted && c.resolution === "stays" && !liftedByReload;

      // ── I1 ──────────────────────────────────────────────────────────────────
      if (covered && c.epoch !== "switched") {
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
      const satisfied = stamped();
      if (!satisfied && c.epoch !== "switched" && !covered) {
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
            stamped(),
            `I2: a replaceable page with nothing covering it must let the request COMPLETE — `
              + `the reference must end up durable, not merely loaded (${label(c)})`,
          ).toBe(true);
        }
      }

      release?.();
      landSave("rev-2"); // idempotent; nothing may outlive its own case
    });
  }
});
