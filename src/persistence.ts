// The debounced persistence engine — extracted from store.ts so the save
// invariant lives in ONE owner: the debounce, the per-page serial write queue,
// and the graph-token / baseRev / tombstone / conflict guards that keep edits
// from being lost, clobbered, or written into the wrong graph.
//
// store.ts owns the doc tree and calls markDirty(page) on every mutation; this
// module decides WHEN and HOW that reaches disk. Its whole dependency on store is
// the named import below, and every binding in it is read at call time rather
// than at module scope, so the store↔persistence import cycle resolves cleanly.
// `persistence.storeSurface.test.ts` pins that import list: it is the coupling
// this split exists to bound, so it may not grow unnoticed.

import {
  doc,
  bumpEditGeneration,
  editorActivationFor,
  peekPageInstanceGeneration,
  setProspectiveTarget,
  pageByName,
  pageInstanceGeneration,
  pageToDto,
  setEditorActivation,
  sweepReplaceable,
} from "./store";
import { ManagedActorRefusalError, backend } from "./backend";
import { favoritesPageChanged } from "./favoritesStore";
import { onGraphRebound } from "./modeHooks";
import {
  markConflict,
  clearConflict,
  isConflicted,
  conflicts,
  conflictObjectFor,
  bumpDataRev,
  bumpPageInventoryRev,
  pushToast,
  registerLiveSaveConflict,
  refreshLiveSaveConflictDraft,
} from "./ui";
import type { ClipboardSourcePage } from "./clipboard";
import { measureIssue248, measureIssue248Async } from "./issue248Probe";
import { failureShape } from "./failureShape";
import { recordClipboardAcceptedSaveForTest } from "./clipboardWorkProbe";

// ---------------------------------------------------------------------------
// Guard state (owned here; mutated only through the accessors below)
// ---------------------------------------------------------------------------

const dirty = new Set<string>();
// Per-page save baseline: the on-disk file rev the editor last loaded or saved.
// Sent on save so the backend conflicts against the version the editor actually
// has, not its own mutable cache (which the watcher can advance under us).
const baseRev = new Map<string, string | null>();
// Pages the user just deleted. A never-saved page can have a queued save with
// baseRev=null; without this, that save fires after the delete and the backend
// (missing file + null baseline = "new page") happily recreates it. While a name
// is tombstoned, saves for it are skipped; re-loading/creating the page clears it.
const deletedPages = new Set<string>();
// Bumped whenever the working set is reset (graph switch). A save abandons its
// baseline update if the graph changed under it; resetSaveState also clears
// `dirty` so a stray queued save becomes a no-op.
let graphToken = 0;
// Which GRAPH the app is bound to — a different question from `graphToken`, and
// deliberately a different counter.
//
// `graphToken` is the SAVE-INVALIDATION epoch: `doSave` captures it, and on
// failure restores the dirty mark only if it still matches. Moving it therefore
// does not merely invalidate a save, it makes a FAILED save stop tracking the
// edit as unsaved — neither written nor pending, and silently discarded at
// close. Reusing it as the binding identity turned an in-place graph reopen into
// exactly that data loss. (GH #254 increment 3, round 14.)
let graphBindingRev = 0;
// Per-page save queue: writes for one page run strictly one-after-another (never
// concurrently) and each runs against the LATEST store state.
const saveChain = new Map<string, Promise<boolean>>();
const transientFailures = new Map<string, number>();
const retryTimers = new Map<string, ReturnType<typeof setTimeout>>();
let saveTimer: ReturnType<typeof setTimeout> | null = null;
// An append outcome the managed runtime cannot prove is resolved only by
// reopening and replaying the physical journal. This latch belongs to this
// graph/process state, not to a page: continuing automatic saves could make a
// draft look retryable when the actor has already terminally fenced mutation.
let reopenRequired = false;
// When the current run of edits first went dirty, so the debounce below can be
// coalescing without being indefinitely postponable. Null whenever no save is
// armed — every drain path (`flushAll`, `resetSaveState`, the timer itself)
// clears `saveTimer`, so the next edit starts a fresh burst.
let burstStartedAt: number | null = null;
let dataRevTimer: ReturnType<typeof setTimeout> | null = null;
const assetWriteChain = new Set<Promise<boolean>>();
// Cross-page move barrier (audit C#1): while a moved subtree's DESTINATION write is not
// yet durable, the SOURCE pages must not save their post-removal state — otherwise an
// UNRELATED edit to a source during the dest-write window marks it dirty and writes the
// block out of existence (gone from the source on disk, not yet in the dest). `heldSources`
// = pages whose saves are blocked; `heldByDest` maps each dest to the sources waiting on
// it, released the moment that dest saves durably (immediately, or after a conflict is
// resolved). Until then the source keeps the block on disk, so it's never lost.
const heldSources = new Set<string>();
// Sources whose "keep mine" arrived DURING the dest-write window. The barrier
// below is absolute — including for a forced save — because the two rules guard
// different things: a conflict is about not clobbering someone else's bytes,
// while the barrier is about not writing a moved block out of existence. An
// override of the first must not override the second. Dropping the resolution
// would strand the page (a conflicted page is skipped by the ordinary save
// path), so remember it and re-issue it the moment the dest is durable.
// A "keep mine" the move barrier deferred, WITH the Direct epoch or managed
// revision the user clicked under. Re-issuing it later must present that exact
// observation and not whatever is current by then: authority observed after the
// click belongs to a winner the user never saw.
type ConflictObservation =
  | { kind: "direct"; epoch: number | null }
  | {
      kind: "managed";
      identity: number;
      observation: { path: string; revision: string } | null;
    };

export type ManagedConflictObservationSnapshot = {
  readonly identity: number;
  readonly observation:
    | { readonly kind: "observed"; readonly path: string; readonly revision: string }
    | { readonly kind: "unobserved" };
};

const heldForcedSaves = new Map<string, ConflictObservation | null>();
const heldByDest = new Map<string, string[]>();
// Managed cross-page moves are actor-owned semantic transactions. While one is
// unresolved, neither page may enter any ordinary/force/reobserve save lane:
// that would race the actor with a second per-page semantic intent.
const managedMoveHeldPages = new Set<string>();
const managedMoveDeferredIntents = new Map<string, SaveIntent[]>();
// Which conflict observation each banner is showing. "Keep mine" presents this
// back so the override answers the conflict the USER SAW. Without it, a second
// force request issued under one banner — a double click, the button is not
// disabled while its request is pending — consumes authority the first request
// minted for a NEWER external winner, and overwrites bytes nobody was shown.
// (GH #254 increment 2, adversarial implementation verification, finding 1.)
const conflictObservation = new Map<string, ConflictObservation>();
let managedConflictObservationClock = 0;

// ---------------------------------------------------------------------------
// Accessors — store.ts mutations call these instead of touching the guards.
// ---------------------------------------------------------------------------

/** Does a page have unsaved (debounced) edits pending? */
export function isDirty(name: string): boolean {
  return dirty.has(name);
}
/** Mark a page dirty and schedule a debounced save. */
export function markDirty(name: string, opts: { content?: boolean } = {}) {
  const page = pageByName(name);
  if (page?.readOnly || page?.guide) return;
  // Every content mutation moves the edit generation, not just `setRaw`: a paste,
  // an outline insert, a block move are all input the user made after clicking
  // "Use disk version", and a counter that only tracks typing lets them be
  // destroyed.
  //
  // But BOOKKEEPING must not move it. Re-arming an existing draft after clearing
  // a false-positive banner changes nothing the user wrote, and counting it as
  // post-click input cancels a discard the user never interrupted — reproduced
  // writing the local draft where the disk winner was requested.
  // (GH #254 increment 3.)
  if (opts.content !== false) bumpEditGeneration(name);
  dirty.add(name);
  scheduleSave();
}
/** Mark dirty WITHOUT scheduling — undo/redo restore batches several pages then
 *  schedules once. */
export function addDirty(name: string) {
  const page = pageByName(name);
  if (page?.readOnly || page?.guide) return;
  bumpEditGeneration(name);
  dirty.add(name);
}
/** Pages with pending edits (so the working-set cap can pin them). */
export function dirtyPages(): Iterable<string> {
  return dirty;
}
/** Is a save currently queued/in flight for this page? (a cross-page move must
 *  flush the source first so it isn't written after being emptied). */
export function isSaving(name: string): boolean {
  return saveChain.has(name);
}
/** Pages with a save queued or in flight. `doSave` removes a page from `dirty`
 *  before awaiting the backend, so these are invisible to `dirtyPages()` and the
 *  working-set cap would otherwise evict one mid-write. */
export function savingPages(): Iterable<string> {
  return saveChain.keys();
}
/** What the store currently holds for a page: whether it exists at all, and its
 *  revision (the content hash) if it does. */
export interface StoredPageState {
  exists: boolean;
  rev: string | null;
}

/** Has the stored page actually diverged from the baseline this editor loaded or
 *  last saved? This is the per-page proof that a change NOTIFICATION does not carry.
 *
 *  The managed runtime's `sparse-v2-changed` tick is a bare aggregate epoch — it
 *  names no page and carries no origin. (It no longer fires for an admission that
 *  committed nothing, which removed one source of spurious wake-ups, but a tick
 *  still cannot say WHICH page changed or whose write it was.)
 *  The legacy watcher event names a page but still cannot distinguish our
 *  own write echoing back from someone else's. So "a notification arrived while this
 *  page was dirty" is not evidence of a conflict, and treating it as one is a false
 *  positive the user pays for: `doSave` refuses a conflicted page, which makes the
 *  banner's own claim ("your unsaved changes weren't written") true only BECAUSE of
 *  the banner, and blocks the very save whose `base_rev` guard would have decided
 *  the question correctly.
 *
 *  All four quadrants matter:
 *  - stored revision equals the baseline → byte-for-byte what we already hold; the
 *    wake-up was our own echo (or another page's). NOT diverged.
 *  - stored revision differs → a genuine external change. Diverged.
 *  - gone from the store, but we had a baseline → the file we loaded was deleted
 *    under us. Diverged.
 *  - gone, and we never had a baseline → a brand-new page that was never written;
 *    there is nothing on disk to diverge from. NOT diverged. */
export function divergedFromBaseline(name: string, stored: StoredPageState): boolean {
  const baseline = baseRev.get(name) ?? null;
  if (!stored.exists) return baseline !== null;
  return stored.rev !== baseline;
}

/** Apply that verdict — in BOTH directions.
 *
 *  Raising a conflict was already gated on the proof above; clearing one was not
 *  gated on anything, because nothing but a user's click ever cleared a
 *  conflict. So a page could be parked behind the banner permanently by an event
 *  that never meant anything: a removal event carries no divergence proof at all
 *  (`handleGraphChange`), and an editor writing by temp+rename, or a
 *  mid-delivery sync pass, produces exactly that. The page then refuses every
 *  ordinary save, and both buttons on the banner destroy something — "Use disk
 *  version" the edit, "Keep mine" a file it never needed to overwrite.
 *
 *  When the disk provably holds this editor's own baseline again, the banner's
 *  claim ("the file changed under you") is false, so it goes, and the edit it
 *  was freezing is scheduled like any other. One owner for both directions means
 *  a later third observation site cannot reintroduce the asymmetry.
 *  (Direct Files data-safety audit, 2026-08-09, finding 17.)
 *
 *  The raising direction does not raise the banner itself — see
 *  `reconcileExternalChange` for why only the backend's own refusal may. */
export async function applyDivergenceVerdict(name: string, stored: StoredPageState): Promise<void> {
  if (divergedFromBaseline(name, stored)) {
    await reconcileExternalChange(name);
    return;
  }
  if (!isConflicted(name)) return;
  clearConflict(name);
  // Re-arm unconditionally rather than only for a page still in `dirty`. A
  // conflicted page's edit is by definition NOT on disk, and `doSave` removes a
  // page from `dirty` before awaiting the backend and does not put it back on a
  // banner-class conflict — so the page whose banner we just lifted is usually
  // unsaved AND unmarked. Leaving it that way strands a live edit that nothing
  // will ever write, and `flushAll` (which consults only `dirty`, in-flight
  // saves and `conflicts`) would then report the graph as safely landed and let
  // a close discard it.
  // Bookkeeping, not input: this re-arms an existing draft, it does not change it.
  markDirty(name, { content: false });
}
/** Hold `sources`' saves until `dest` is durably written (cross-page move barrier,
 *  audit C#1). `releaseSourcesFor(dest)` fires from doSave's success path. */
export function holdSourcesForDest(dest: string, sources: string[]) {
  const srcs = sources.filter((s) => s !== dest);
  if (srcs.length === 0) return;
  heldByDest.set(dest, srcs);
  for (const s of srcs) heldSources.add(s);
}

/** Hold every save intent for the exact affected pages until one managed move
 * episode settles. The returned release is affine and safe to call repeatedly. */
export function holdManagedMovePages(pages: readonly string[]): () => void {
  const held = [...new Set(pages)];
  for (const page of held) managedMoveHeldPages.add(page);
  let released = false;
  return () => {
    if (released) return;
    released = true;
    let scheduleOrdinary = false;
    for (const page of held) {
      managedMoveHeldPages.delete(page);
      const intents = managedMoveDeferredIntents.get(page) ?? [];
      managedMoveDeferredIntents.delete(page);
      for (const intent of intents) {
        if (intent.kind === "ordinary") scheduleOrdinary = true;
        else void enqueueSave(page, intent);
      }
    }
    if (scheduleOrdinary) scheduleSave();
  };
}

/** An unresolved actor append is durable but cannot safely coexist with new
 * per-page saves. This also makes clean close/graph switch visibly refuse. */
export function requireManagedRuntimeReopen(): void {
  latchReopenRequired();
}

/** Track an optimistic asset write so flushAll/app-close waits for the bytes to
 *  land before letting the process exit. The caller still owns success/failure
 *  handling for any UI/store rollback. */
export function trackAssetWrite<T>(write: Promise<T>): Promise<T> {
  let tracked: Promise<boolean>;
  tracked = write.then(
    () => true,
    () => false
  ).finally(() => {
    assetWriteChain.delete(tracked);
  });
  assetWriteChain.add(tracked);
  return write;
}
/** Dest saved durably → let its held sources persist their post-removal state now
 *  (the block is on disk in the dest, so removing it from the source loses nothing). */
function releaseSourcesFor(dest: string) {
  const srcs = heldByDest.get(dest);
  if (!srcs) return;
  heldByDest.delete(dest);
  let any = false;
  for (const s of srcs) {
    if (!heldSources.delete(s)) continue;
    if (heldForcedSaves.has(s)) {
      // A "keep mine" the barrier deferred. Re-issue it now rather than letting
      // `scheduleSave` pick it up: the page is still marked conflicted, and the
      // ordinary path skips a conflicted page, so this resolution would
      // otherwise be silently lost. It carries the observation the user clicked
      // under, so if storage moved while the barrier held it, the backend
      // refuses and the refusal below re-raises a banner the user can decide on.
      const observation = heldForcedSaves.get(s) ?? null;
      heldForcedSaves.delete(s);
      dirty.add(s); // a force writes the store's state; keep it visible as unsaved
      void enqueueSave(s, { kind: "force", observation }).then((ok) => {
        if (ok) clearConflict(s);
        else pushToast(`Couldn't overwrite “${s}”.`, "error");
      });
      continue;
    }
    dirty.add(s); // its removal (and any held edit) can write now
    if (isConflicted(s)) {
      // A conflicted source cannot travel the ORDINARY route: `doSave` returns at
      // the conflicted-page guard before reaching the backend, so scheduling one
      // leaves the page dirty behind a banner whose authority may already be dead,
      // with nothing to clear it — the retry timer requires `!isConflicted` too.
      // Re-observe instead: that intent exists precisely to bypass the guard, and
      // it either lands or mints a fresh banner the user can answer.
      // (GH #254 increment 3, D5.)
      void enqueueSave(s, { kind: "reobserve" });
      continue;
    }
    any = true;
  }
  if (any) scheduleSave();
}
/** Record a page's load/save baseline rev (set on load and after each save). */
export function setBaseRev(name: string, rev: string | null) {
  baseRev.set(name, rev);
}
/** Has this page been deleted? A deleted page must never be recreated by anything
 *  that was already in flight when the deletion landed. */
export function isTombstoned(name: string): boolean {
  return deletedPages.has(name);
}

/**
 * Which GRAPH the app is bound to. Changes only when the binding actually
 * changes (`resetSaveState`, i.e. a graph switch or reload).
 *
 * Deliberately not `graphEpoch()`, which is a RENDER epoch: changing typography
 * mode, the journal title format, or a setting bumps it to repaint open pages,
 * without the graph moving at all. Work that must not cross a graph switch has
 * to key off the binding — keying off the render epoch means a user toggling a
 * display preference invalidates in-flight work that was perfectly valid.
 * (GH #254 increment 3, round 12.)
 */
export function graphBinding(): number {
  return graphBindingRev;
}

/**
 * Declare that the graph has been REBOUND without a full store reset.
 *
 * `changeJournalTitleFormat` is the case: the backend rewrites `config.edn`,
 * reopens the graph, and may migrate journal filenames — so work in flight
 * against the old binding is now aimed at paths that may no longer exist. The
 * frontend only bumped the render epoch there, which is why keying anything off
 * the epoch LOOKED equivalent: it accidentally covered this. Keying off the
 * binding is correct, but only if every real rebind says so.
 *
 * Deliberately not `resetSaveState()`: this is a rebind, not a graph switch, and
 * dropping the user's dirty state would lose edits the reopen did not touch.
 * (GH #254 increment 3, round 13.)
 */
export function bumpGraphBinding(): void {
  graphBindingRev++;
}

// A backend reopen is a rebind; the binding must move with it.
onGraphRebound(bumpGraphBinding);

/** Which FILE a tombstone was raised for, when the delete named one. */
const deletedPagePaths = new Map<string, string>();

/** Refuse only the exact deleted file when the tombstone named a path. */
export function isTombstonedFile(name: string, path?: string): boolean {
  if (!deletedPages.has(name)) return false;
  const deleted = deletedPagePaths.get(name);
  if (!deleted || !path) return true;
  return deleted === path;
}

/** Does a tombstone provably cover this file? Unknown path proceeds when the
 * tombstone names another exact file, allowing the read to decide. */
export function tombstoneCovers(name: string, path?: string): boolean {
  if (!deletedPages.has(name)) return false;
  const deleted = deletedPagePaths.get(name);
  if (!deleted) return true;
  return deleted === path;
}

/** Publish a tombstone directly for lifecycle tests and already-proven callers. */
export function tombstone(name: string, path?: string): void {
  deletedPages.add(name);
  if (path) deletedPagePaths.set(name, path);
  else deletedPagePaths.delete(name);
}

/** Atomically retire one quiescent page from persistence.
 *
 * Delete calls this synchronously after its awaited drain and exact-instance
 * check. A keystroke can land in that await-to-continuation handoff, so the
 * tombstone itself must re-check dirty and saving state in the same JavaScript
 * turn that publishes the marker. A caller may explicitly allow an already
 * captured conflict-resolution delete; ordinary deletes still reject a conflict
 * raised during their awaited drain. */
export function tombstoneIfQuiescent(
  name: string,
  allowConflicted = false,
  path?: string,
): boolean {
  if (
    (dirty.has(name) && !allowConflicted)
    || saveChain.has(name)
    || (!allowConflicted && isConflicted(name))
    || deletedPages.has(name)
  ) {
    return false;
  }
  deletedPages.add(name);
  // Always REPLACE the recorded file, never merge with an older tombstone's: a
  // pathless delete means "every file with this name", and inheriting a stale
  // path from a previous tombstone would silently narrow it to one file — the
  // wrong one, letting the others be resurrected. (GH #254 increment 3.)
  if (path) deletedPagePaths.set(name, path);
  else deletedPagePaths.delete(name);
  return true;
}
/** Lift a delete tombstone (page re-created, or the delete failed). */
export function untombstone(name: string) {
  deletedPages.delete(name);
  deletedPagePaths.delete(name);
}
/** Drop a page's dirty + baseline state — its content is leaving the working set. */
export function forgetSaveState(name: string) {
  dirty.delete(name);
  baseRev.delete(name);
  conflictObservation.delete(name);
  clearTransientRetry(name);
}
/** Cancel timers, invalidate in-flight saves (bump the graph token), and clear
 *  all guard state — on graph switch / reset, so nothing from the old graph can
 *  be written after a switch. */
export function resetSaveState() {
  if (saveTimer) {
    clearTimeout(saveTimer);
    saveTimer = null;
  }
  if (dataRevTimer) {
    clearTimeout(dataRevTimer);
    dataRevTimer = null;
  }
  graphToken++;
  reopenRequired = false;
  graphBindingRev++; // a switch is also a rebind
  dirty.clear();
  baseRev.clear();
  conflictObservation.clear();
  deletedPages.clear();
  deletedPagePaths.clear();
  heldSources.clear();
  heldByDest.clear();
  managedMoveHeldPages.clear();
  managedMoveDeferredIntents.clear();
  transientFailures.clear();
  for (const timer of retryTimers.values()) clearTimeout(timer);
  retryTimers.clear();
}

// ---------------------------------------------------------------------------
// The engine
// ---------------------------------------------------------------------------

// Debounced query-recompute trigger: bump dataRev only after edits go quiet, so
// sustained typing doesn't re-run every visible query every save batch.
function scheduleDataRev() {
  if (dataRevTimer) clearTimeout(dataRevTimer);
  dataRevTimer = setTimeout(() => {
    dataRevTimer = null;
    bumpDataRev();
  }, 700);
}

function clearTransientRetry(name: string) {
  transientFailures.delete(name);
  const timer = retryTimers.get(name);
  if (timer) clearTimeout(timer);
  retryTimers.delete(name);
}

function latchReopenRequired(): boolean {
  if (reopenRequired) return false;
  reopenRequired = true;
  if (saveTimer) {
    clearTimeout(saveTimer);
    saveTimer = null;
  }
  burstStartedAt = null;
  for (const timer of retryTimers.values()) clearTimeout(timer);
  retryTimers.clear();
  transientFailures.clear();
  return true;
}

/** Save failures the backend reports with a bounded code that a retry cannot
 *  change: the graph has two files whose names collide on case-insensitive
 *  filesystems, two paths pointing at one physical file, a symlink where a page
 *  was expected, or another page already holding this title. Retrying re-runs
 *  the whole pre-save check — on a large graph the expensive part — to arrive at
 *  the same answer, three times, per dirty page. Report it once instead. */
export function isRetryableSaveFailure(error: unknown): boolean {
  const code = saveFailureCode(error);
  return ![
    "precheck.symlink",
    "precheck.portable_collision",
    "precheck.resource_alias",
    "precheck.not_portable",
    "precheck.nofollow",
    "precheck.limit",
    "identity.owned_elsewhere",
    "identity.name_taken",
    // Managed storage refused the save because the page moved underneath it.
    // Permanent until the page is reloaded — retrying just hides it.
    "managed.conflict",
    // The process cannot know whether this record was written. Reopen selects
    // the complete physical prefix; a same-process retry could duplicate it.
    "trusted_local.append_outcome_unknown",
  ].some((nonRetryable) => code === nonRetryable);
}

export type SaveFailureDisposition = "append_outcome_unknown" | "ordinary";

/** Classify bounded backend failures before any generic retry policy. */
export function saveFailureDisposition(error: unknown): SaveFailureDisposition {
  return saveFailureCode(error) === "trusted_local.append_outcome_unknown"
    ? "append_outcome_unknown"
    : "ordinary";
}

/** Extract a bounded save-failure code from the typed backend contracts.
 *
 *  W4-E4 retired the Direct save prefix parser that used to live here. The
 *  command funnel has already validated the tagged JSON payload, so a Direct
 *  failure arrives as an object with a closed `reasonCode`; no user-controlled
 *  path or page text participates in classification, and a page legitimately
 *  named `conflict_authority.notes` can no longer make an unrelated
 *  `precheck.symlink` failure look like a spent override. (GH #254 increment 2,
 *  fourth correction-delta re-verification.)
 *
 *  The `managed.conflict` prefix below is a Managed producer's own tagged
 *  string, not Direct save prose; E2/E2b move it onto the shared typed carrier.
 *  Returns `""` when nothing typed is present. */
function saveFailureCode(error: unknown): string {
  if (error instanceof ManagedActorRefusalError) return error.reasonCode;
  if (typeof error === "object" && error !== null && "kind" in error && "reasonCode" in error) {
    const typed = error as { kind: unknown; reasonCode: unknown };
    if (
      (typed.kind === "direct-save-failure" || typed.kind === "save-conflict")
      && typeof typed.reasonCode === "string"
    ) {
      return typed.reasonCode;
    }
  }
  // Managed storage still owns this pre-existing tagged string producer; E2/E2b
  // will move it through the shared typed carrier. It is not Direct save prose.
  if (typeof error === "string" && error.startsWith("managed.conflict: ")) {
    return "managed.conflict";
  }
  return "";
}

/** The observation epoch a banner-class conflict was raised at.
 *
 *  `conflict:<n>` is the whole contract for the keep-mine/use-disk banner; the
 *  bare `conflict` is the legacy shape and means the backend minted authority it
 *  could not name, so no override may be presented for it. Returns null when the
 *  error is not banner class at all, and -1 for the unnameable legacy shape. */
function conflictObservationEpoch(error: unknown): number | null {
  if (typeof error !== "object" || error === null || !("kind" in error) || error.kind !== "save-conflict") {
    return null;
  }
  if (!("epoch" in error) || error.epoch === null) return -1;
  return typeof error.epoch === "number" && Number.isSafeInteger(error.epoch) && error.epoch >= 0
    ? error.epoch
    : null;
}

/** Whether a save failed because the page revision changed underneath it.
 * Consumers outside the persistence loop use this bounded contract instead of
 * searching arbitrary backend prose for words such as "conflict" or "exists". */
export function isSaveConflictFailure(error: unknown): boolean {
  return conflictObservationEpoch(error) !== null || saveFailureCode(error) === "managed.conflict";
}

function scheduleTransientRetry(name: string, token: number, error: unknown) {
  if (reopenRequired) return;
  if (!isRetryableSaveFailure(error)) {
    transientFailures.delete(name);
    pushToast(`Couldn't save “${name}”. (${String(error)})`, "error");
    return;
  }
  const failures = (transientFailures.get(name) ?? 0) + 1;
  transientFailures.set(name, failures);
  if (failures >= 3) {
    // Automatic retries stop here. The caller has already put the page back in
    // `dirty`, so it still saves on the next edit or flush — but nothing is
    // scheduled, and the old copy ("will retry") implied a timer that does not
    // exist. Say what actually happens.
    transientFailures.delete(name);
    pushToast(
      `Couldn't save “${name}” after 3 tries — it will be retried when you next edit it. (${String(error)})`,
      "error"
    );
    return;
  }
  const prior = retryTimers.get(name);
  if (prior) clearTimeout(prior);
  const timer = setTimeout(() => {
    retryTimers.delete(name);
    if (token === graphToken && !reopenRequired && dirty.has(name) && !isConflicted(name)) {
      void enqueueSave(name);
    }
  }, failures === 1 ? 100 : 300);
  retryTimers.set(name, timer);
}

function cutSourceMatches(expected: ClipboardSourcePage): boolean {
  const page = pageByName(expected.name);
  return !!page
    && page.name === expected.name
    && page.kind === expected.kind
    && page.path === expected.path
    && pageInstanceGeneration(expected.name) === expected.generation;
}

function cutSourceUsable(expected: ClipboardSourcePage): boolean {
  return cutSourceMatches(expected)
    && !deletedPages.has(expected.name)
    && !isConflicted(expected.name);
}

/** Why a save is being run.
 *
 *  - `ordinary` — the user's pending edit. Skipped for a clean page and refused
 *    for a conflicted one.
 *  - `reobserve` — the disk changed under a page whose banner is already up, so
 *    the authority that banner stands on has just been revoked. Runs the SAME
 *    base-revision-guarded write, which cannot clobber; its refusal mints the
 *    replacement epoch, and its success means the banner was stale.
 *  - `force` — the user chose "Keep mine". Overwrites, presenting the epoch the
 *    banner showed. */
type SaveIntent =
  | { kind: "ordinary" }
  | { kind: "reobserve" }
  | { kind: "force"; observation: ConflictObservation | null };

const ORDINARY: SaveIntent = { kind: "ordinary" };

/** Capture the epoch the visible banner is showing, at the moment the user acts.
 *  Reading it later — when the queued request finally reaches the backend — is
 *  the bug: a re-observation running ahead of it in the queue replaces the entry
 *  with an epoch minted for a winner the user never saw, and the force would
 *  then be authorised to discard exactly that. */
function forceIntent(name: string): SaveIntent {
  return { kind: "force", observation: conflictObservation.get(name) ?? null };
}

/** Whether the visible conflict has a safe Keep mine path. A managed page that
 * disappeared, was renamed, or could not be identified keeps Use current live
 * while refusing to invent replacement authority. */
export function canForceSave(name: string): boolean {
  const observation = conflictObservation.get(name);
  if (!observation) return true;
  return observation.kind === "direct"
    ? observation.epoch !== null
    : observation.observation !== null;
}

/** The exact observation the banner for `name` is showing, captured AT THE CLICK.
 *
 *  Read at click time, never later: a re-observation running ahead of a queued
 *  request replaces the entry with an epoch minted for a winner the user never
 *  saw, and answering with that would discard exactly that winner. */
export function shownObservationFor(name: string): number | null {
  const observation = conflictObservation.get(name);
  return observation?.kind === "direct" ? observation.epoch : null;
}

/** Storage protocol represented by the conflict banner for `name`. */
export function conflictObservationKindFor(name: string): ConflictObservation["kind"] | null {
  return conflictObservation.get(name)?.kind ?? null;
}

/** Capture the exact managed observation represented by the current banner.
 *
 * The identity distinguishes two actor observations even when they report the
 * same path/revision (or are both explicitly unobserved). The returned value is
 * a detached typed snapshot, not the mutable map entry used by persistence.
 */
export function managedConflictObservationSnapshotFor(
  name: string,
): ManagedConflictObservationSnapshot | null {
  const current = conflictObservation.get(name);
  if (current?.kind !== "managed") return null;
  return {
    identity: current.identity,
    observation: current.observation
      ? {
          kind: "observed",
          path: current.observation.path,
          revision: current.observation.revision,
        }
      : { kind: "unobserved" },
  };
}

/** Is `snapshot` still the exact managed observation shown for `name`? */
export function managedConflictObservationMatches(
  name: string,
  snapshot: ManagedConflictObservationSnapshot,
): boolean {
  const current = conflictObservation.get(name);
  if (current?.kind !== "managed" || current.identity !== snapshot.identity) return false;
  if (snapshot.observation.kind === "unobserved") return current.observation === null;
  return current.observation?.path === snapshot.observation.path
    && current.observation.revision === snapshot.observation.revision;
}

/**
 * Make sure `name`'s editor has an activation, minting one if it has none.
 *
 * Idempotent, so ordinary re-saves never churn the token a live banner is bound
 * to. Failure is deliberately non-fatal: the ordinary path's base-revision guard
 * is unchanged and remains its own authority, so a user never loses a save
 * because an identity could not be minted — only the override path is
 * unavailable.
 */
async function ensureEditorActivation(name: string): Promise<void> {
  if (editorActivationFor(name) !== undefined) return;
  const page = pageByName(name);
  if (!page) return;
  // The BINDING, not the save epoch. These two checks ask "is this still the
  // same graph?", which is a different question from "is this save still valid?"
  // — and since the two counters were split, `graphToken` deliberately does not
  // move on an in-place reopen. Asking it here let an activation minted by the
  // OLD `Graph` be installed and sent to `savePage` after the backend replaced
  // it. (GH #254 increment 3, round 15.)
  const token = graphBindingRev;
  // The page's path cannot see a SAME-PATH content replacement — `reloadPage` and
  // the watcher-approved reload both install a new editor at the same path — so
  // the instance generation is required as well. Peeked, never read through the
  // lazily-creating accessor: that would mint a generation where none existed and
  // mutate the identity cut retirement compares.
  const pathAtStart = page.path ?? "";
  const instanceAtStart = peekPageInstanceGeneration(name);
  try {
    const handle = page.path
      // No local activation means there is nothing this exact instance may
      // legitimately reuse. Minting replaces any stale core record left by a
      // failed best-effort retirement instead of inheriting another editor's
      // conflict authority.
      // Snapshot-less by design. This is the save-time fallback for an already
      // mounted editor, not installation of a DTO. The ordinary save below still
      // compares its captured base revision, so divergence raises an answerable
      // conflict under the newly minted activation instead of blessing stale
      // bytes or leaving the banner without a live editor identity.
      ? await backend().activateEditor(page.path, "replace", null)
      : await backend().activateAbsentEditor(name, page.kind);
    // Re-check across the await. A graph switch or a re-install makes this a
    // DIFFERENT editor, and recording the handle then would hand one editor's
    // identity to another — reproduced writing a replacement graph's page.
    if (
      handle &&
      editorActivationFor(name) === undefined &&
      graphBindingRev === token &&
      (pageByName(name)?.path ?? "") === pathAtStart &&
      peekPageInstanceGeneration(name) === instanceAtStart
    ) {
      setEditorActivation(name, handle.activation);
      // An ABSENT editor must also carry the prospective target it is live for.
      // Recorded beside the activation and read at the DTO boundary — NOT written
      // onto the store page, which was tried and reverted: mutating the page while
      // a save builds its snapshot disturbs cut retirement, which is
      // authority-bound to the exact loaded instance. (GH #254 increment 3.)
      if (handle.prospective && handle.target) setProspectiveTarget(name, handle.target);
    } else if (handle) {
      // The core minted this identity, but the exact editor that requested it
      // disappeared while IPC was in flight. Leaving it live lets a later Reuse
      // hand an unowned activation to another editor of the same path.
      await backend()
        .retireEditorActivation(handle.target, handle.activation)
        .catch(() => {});
    }
  } catch {
    // Non-fatal, as above.
  }
}

/** The load baseline the editor's conflict episode was minted under.
 *
 *  The episode is `{ loaded_revision, activation }`, so presenting an observation
 *  has to name the same revision the refused save did or the episode equality
 *  refuses the very editor whose banner it is. */
export function saveBaselineFor(name: string): string | null {
  return baseRev.get(name) ?? null;
}

/** Forget a spent or dead observation without touching the banner. */
export function dropObservation(name: string): void {
  conflictObservation.delete(name);
}

/** Re-observe `name`: the guarded save that bypasses the conflicted-page early
 *  return, so a page whose authority died still reaches the backend and either
 *  lands or raises a fresh banner. */
export function reobserve(name: string): Promise<boolean> {
  return enqueueSave(name, { kind: "reobserve" });
}

function enqueueSave(
  name: string,
  intent: SaveIntent = ORDINARY,
  expectedCutSource?: ClipboardSourcePage,
): Promise<boolean> {
  const prev = saveChain.get(name) ?? Promise.resolve(true);
  const next = prev.then(
    () => doSave(name, intent, expectedCutSource),
    () => doSave(name, intent, expectedCutSource),
  );
  saveChain.set(name, next);
  void next.finally(() => {
    if (saveChain.get(name) === next) saveChain.delete(name);
    // Announce only once this save is genuinely out of the queue. Announcing from
    // the success path instead — even deferred a microtask — still ran while the
    // entry was present, so the re-verification correctly dropped the event and
    // the waiting request was never re-driven. (GH #254 increment 3.)
    // Sweep rather than announce one name: a save frees the page it wrote AND can
    // release others (the cross-page move barrier), and the sweep re-verifies each
    // watched page anyway, so it cannot announce something that is not ready.
    sweepReplaceable();
  });
  return next;
}

const LIVE_CONFLICT_CAPSULE_DEBOUNCE_MS = 500;
interface PendingLiveConflictCapsule {
  timer: ReturnType<typeof setTimeout>;
  page: NonNullable<ReturnType<typeof pageToDto>>;
  token: number;
}
const liveConflictCapsulePending = new Map<string, PendingLiveConflictCapsule>();

/** Capsule durability has its own debounce: a conflicted page remains dirty,
 * so ordinary save attempts may recur even though there is no file write to
 * perform. Coalesce those attempts around the latest draft without coupling
 * the crash-recovery envelope rewrite to the main save timer. */
export function scheduleLiveSaveConflictDraftRefresh(
  page: NonNullable<ReturnType<typeof pageToDto>>,
): void {
  const previous = liveConflictCapsulePending.get(page.name);
  if (previous) clearTimeout(previous.timer);
  const token = graphToken;
  const timer = setTimeout(() => {
    if (liveConflictCapsulePending.get(page.name)?.timer !== timer) return;
    liveConflictCapsulePending.delete(page.name);
    // A timer belongs to the graph whose draft scheduled it. Never let an old
    // graph's delayed refresh attach to a same-named page after a switch.
    if (graphToken !== token) return;
    void refreshLiveSaveConflictDraft(page);
  }, LIVE_CONFLICT_CAPSULE_DEBOUNCE_MS);
  liveConflictCapsulePending.set(page.name, { timer, page, token });
}

/** Persist every debounced capsule draft NOW and wait for it. The close /
 * switch / restore barrier (`flushAll`) drains file saves, but a conflicted
 * page's latest draft lives only in its capsule — leaving the debounce timer
 * pending lets the app exit with a crash-recovery envelope that trails the
 * in-memory draft by up to the debounce window. Stale-graph entries are
 * dropped rather than written, exactly as their timers would have done. */
export async function flushLiveSaveConflictDrafts(): Promise<void> {
  const pending = [...liveConflictCapsulePending.values()];
  for (const entry of pending) clearTimeout(entry.timer);
  liveConflictCapsulePending.clear();
  await Promise.all(
    pending
      .filter((entry) => entry.token === graphToken)
      .map((entry) => refreshLiveSaveConflictDraft(entry.page)),
  );
}

/** Write the page's CURRENT state once. No-op success if it isn't dirty and not
 *  forced. Sends `baseRev` (the version the editor loaded) so the backend
 *  conflicts against external changes; updates the baseline on success. On a
 *  conflict marks it (no clobber); on a transient error keeps it dirty + toasts. */
async function doSave(
  name: string,
  intent: SaveIntent,
  expectedCutSource?: ClipboardSourcePage,
): Promise<boolean> {
  const force = intent.kind === "force";
  // A cut-retirement save is authority-bound to the exact loaded page instance.
  // Check when this queued operation actually reaches its snapshot boundary, not
  // only when the caller enqueues it: another save may have been ahead of it.
  if (expectedCutSource && !cutSourceUsable(expectedCutSource)) return false;
  if (deletedPages.has(name)) return true; // tombstoned — never recreate a deleted page
  if (managedMoveHeldPages.has(name)) {
    const deferred = managedMoveDeferredIntents.get(name) ?? [];
    // Flush loops may submit the same ordinary intent repeatedly while the
    // actor owns the page. One retained ordinary intent is enough, while
    // reobserve/force intents keep their order and captured authority.
    if (intent.kind !== "ordinary" || !deferred.some((candidate) => candidate.kind === "ordinary")) {
      deferred.push(intent);
      managedMoveDeferredIntents.set(name, deferred);
    }
    return false;
  }
  // A re-observation must reach the backend even though the page is clean or
  // already conflicted: those are exactly the states a banner leaves behind, and
  // only a fresh refusal can mint the authority the visible banner needs.
  if (intent.kind === "ordinary" && !dirty.has(name)) return true; // saved by a prior link
  if (intent.kind === "ordinary" && isConflicted(name)) {
    const draft = pageToDto(name);
    if (draft) scheduleLiveSaveConflictDraftRefresh(draft);
    return false;
  }
  // A cross-page move source: hold its save until the destination is durable (C#1).
  // Stays dirty, so it writes the moment `releaseSourcesFor(dest)` frees it.
  if (heldSources.has(name)) {
    if (intent.kind === "force") heldForcedSaves.set(name, intent.observation);
    return false;
  }
  const token = graphToken;
  // The BINDING for the activation window, separately from the save epoch above:
  // a reopen replaces the `Graph` (and its activation registry) without
  // invalidating this save, so the two questions need their own counters here
  // too. (GH #254 increment 3, round 15.)
  const bindingAtStart = graphBindingRev;
  const activationInstanceAtStart = peekPageInstanceGeneration(name);
  const activationPathAtStart = pageByName(name)?.path ?? "";
  // Acquire this editor's identity before the DTO is built, so `pageToDto` can
  // stamp it. Keyed through the STORE's registry, never by path alone: a copied
  // DTO does not travel this path, never acquires an identity, and is refused on
  // the override path for presenting none. (GH #254 increment 3.)
  if (editorActivationFor(name) === undefined) await ensureEditorActivation(name);
  // Acquisition is an awaited IPC, so the world can move under it. Refusing to
  // RECORD a stale identity is not enough — the save must abandon too, or it
  // serializes the replacement graph's bytes and writes them anyway (reproduced,
  // with `activation: undefined`, which is exactly why an identity check alone
  // could not catch it). (GH #254 increment 3.)
  if (
    graphToken !== token
    || graphBindingRev !== bindingAtStart
    || peekPageInstanceGeneration(name) !== activationInstanceAtStart
    || (pageByName(name)?.path ?? "") !== activationPathAtStart
  ) return false;
  const dto = measureIssue248("frontend.pageToDtoMs", () => pageToDto(name));
  if (!dto) {
    // Two very different reasons, and they must not share an outcome (audit
    // finding 6). If the page is not in the store at all there is nothing to
    // write and never will be, so leaving the name in `dirty` wedges
    // `flushAll()` permanently: every later graph switch aborts with "Some pages
    // couldn't be saved" and every window close offers to discard edits that are
    // already on disk. Drop the phantom. If the page IS present, the DTO was
    // refused by the page-header firewall (`pageToDto` has already toasted) —
    // that is a real unsaved edit, so it stays dirty and keeps blocking.
    if (!pageByName(name)) dirty.delete(name);
    return false;
  }
  if (dto.guide) {
    console.warn("Refusing to persist ephemeral bundled Guide pages", { count: 1 });
    dirty.delete(name);
    return true;
  }
  if (dto.read_only) {
    console.error("Refusing to persist read-only pages", { count: 1 });
    dirty.delete(name);
    return false;
  }
  dirty.delete(name);
  const baseline = baseRev.get(name) ?? null;
  const issuingInstance = peekPageInstanceGeneration(name);
  const issuingActivation = dto.activation;
  try {
    const observation = intent.kind === "force" ? intent.observation : null;
    const saved = await measureIssue248Async("frontend.savePageAwaitMs", () =>
      backend().savePage(
        dto,
        baseline,
        force,
        observation?.kind === "direct" ? observation.epoch : null,
        observation?.kind === "managed" ? observation.observation : null,
      )
    );
    const rev = saved.revision;
    // A reload/rename/delete/rebind while savePage was in flight invalidates the
    // retirement proof even if those bytes landed. Never let that stale success
    // authorize identity reuse or update the replacement instance's baseline.
    const cutStillUsable = !expectedCutSource || cutSourceUsable(expectedCutSource);
    const exactIssuerStillLive = cutStillUsable
      && graphBindingRev === bindingAtStart
      && peekPageInstanceGeneration(name) === issuingInstance
      && editorActivationFor(name) === issuingActivation;
    if (saved.activation) {
      if (exactIssuerStillLive) {
        // A first-create response may resolve/retarget the activation. It belongs
        // only to the exact page instance and graph binding that issued this save;
        // a late success must never bless a replacement editor.
        setEditorActivation(name, saved.activation.activation);
        setProspectiveTarget(name, saved.activation.target);
      } else {
        // The save landed, but its editor disappeared while it was in flight.
        // Retire the returned exact handle so the successful response cannot
        // leave unowned override authority behind.
        await backend()
          .retireEditorActivation(saved.activation.target, saved.activation.activation)
          .catch(() => {});
      }
    }
    // The durable save may have completed, but its caller no longer owns the
    // current frontend instance. Do not update that replacement's baseline,
    // clear its conflict, or release move barriers on its behalf.
    if (!exactIssuerStillLive) return false;
    if (token === graphToken) {
      baseRev.set(name, rev);
      if (baseline === null) bumpPageInventoryRev();
    }
    // The favorites arrangement lives in an ordinary page, so editing it in
    // Tine's own editor is how a keyboard user reorders favorites. The sidebar
    // has to follow that edit, and this is the one place every in-app page save
    // passes through.
    void favoritesPageChanged([name]);
    clearTransientRetry(name);
    conflictObservation.delete(name);
    // The bytes landed, so any banner still up is answered. Only a re-observation
    // can arrive here with one raised (the ordinary path refuses a conflicted
    // page and a force is followed by its own clear), and leaving it would park
    // the page behind a warning about a change that is now written.
    if (isConflicted(name)) clearConflict(name);
    releaseSourcesFor(name); // if this was a cross-page dest, its sources can save now
    if (import.meta.env.MODE === "test") {
      recordClipboardAcceptedSaveForTest(expectedCutSource ? "source" : "target", name);
    }
    return true;
  } catch (e) {
    if (saveFailureDisposition(e) === "append_outcome_unknown") {
      if (token === graphToken) {
        dirty.add(name); // the draft remains local until reopen resolves the frame
        if (latchReopenRequired()) {
          pushToast(
            "Managed storage could not establish the append outcome. Reopen Tine before saving again.",
            "error"
          );
        }
      }
      return false;
    }
    // The backend says "conflict" and nothing else for a real base-revision
    // conflict. Match it exactly: a substring test used to catch every other
    // backend `AlreadyExists` too -- a portable-filename collision, a
    // physical-resource alias, "another document owns this page identity" --
    // and mark the page conflicted, which puts up a prompt whose only two
    // options cannot resolve any of them AND stops the page saving from then on.
    // Those now arrive with their own bounded code and fall through to the
    // retry/toast path below.
    const observed = conflictObservationEpoch(e);
    if (observed !== null) {
      clearTransientRetry(name);
      if (observed >= 0) {
        conflictObservation.set(name, { kind: "direct", epoch: observed });
        try {
          const capture = await backend().captureLiveSaveConflict(dto, baseline, observed);
          if (!capture) throw new Error("Direct Files capture returned no authority payload");
          await registerLiveSaveConflict(dto, baseline, observed, capture);
        } catch (captureError) {
          // The draft remains live and close protection stays armed. Surface the
          // missing restart capsule instead of pretending crash recovery exists.
          await registerLiveSaveConflict(dto, baseline, observed);
          pushToast(
            `Couldn't preserve “${name}” for restart recovery. Keep Tine open while resolving it. (${String(captureError)})`,
            "error",
            { sticky: true },
          );
        }
      } else {
        conflictObservation.set(name, { kind: "direct", epoch: null });
      }
      markConflict(name);
    } else if (saveFailureCode(e) === "managed.conflict") {
      // The refusal itself carries no overwrite authority. Observe the exact
      // pinned managed page through the actor, but do not load it into the
      // store: the retained draft stays intact until the user explicitly
      // chooses Use current. Keep mine returns this revision to one serialized
      // actor turn, which re-proves it before authoring anything.
      clearTransientRetry(name);
      let managedObservation: { path: string; revision: string } | null = null;
      try {
        const current = dto.path
          ? await backend().getPageByPath(dto.path)
          : await backend().getPage(name, dto.kind);
        if (token !== graphToken) return false;
        if (current?.path && current.rev && (!dto.path || current.path === dto.path)) {
          managedObservation = { path: current.path, revision: current.rev };
        }
      } catch {
        if (token !== graphToken) return false;
        // Missing, renamed, ambiguous, or temporarily unobservable owners do
        // not get replacement authority. The non-destructive draft remains
        // parked and Use current remains available.
      }
      conflictObservation.set(name, {
        kind: "managed",
        identity: ++managedConflictObservationClock,
        observation: managedObservation,
      });
      // Use the same semantic capture boundary as Direct Files. Managed returns
      // no durable replacement authority: only the retained page and base below
      // enter the app-private capsule.
      try {
        await backend().captureLiveSaveConflict(dto, baseline, 0);
      } catch (error) {
        // The capture only enriches the review; the retained draft below is
        // the recovery material and must still reach the banner and capsule.
        console.error("[tine] managed conflict capture failed", failureShape(error));
      }
      // Re-notify an already visible banner so its Keep mine enabled state
      // reflects this newly observed (or now unobservable) managed owner. Clear
      // before replacing the capsule so the newly persisted draft survives.
      if (isConflicted(name)) clearConflict(name);
      // Persist only the retained draft and its load baseline while raising the
      // ordinary banner. The actor's replacement observation above remains
      // session-scoped and must be minted again after restart.
      await markConflict(name, { page: dto, baseRev: baseline, storage: "managed" });
    } else if (saveFailureCode(e).startsWith("conflict_authority.")) {
      // The force named an observation the disk has since moved past — a later
      // external write, or a read, revoked it before the click reached the
      // backend. The refusal is right: that authority was for a state the user
      // is no longer looking at. But the banner it answers is now dead, and
      // leaving it up is the unresolvable tail again: every further "Keep mine"
      // presents the same spent epoch, the ordinary retry is forbidden while the
      // page is conflicted, and only the destructive button still works.
      //
      // So drop the dead epoch and observe again. A still-divergent disk raises
      // a FRESH banner with live authority, describing the state that is
      // actually there now; a disk the guard accepts simply takes the edit. The
      // user re-decides against what they can see, which is the whole contract.
      // (GH #254 increment 2, third correction-delta re-verification.)
      clearTransientRetry(name);
      conflictObservation.delete(name);
      if (token === graphToken) {
        dirty.add(name); // the edit is still unwritten
        void reconcileExternalChange(name);
      }
    } else if (saveFailureCode(e).startsWith("conflict_retry.")) {
      // The backend could not coherently observe the winner, so it minted no
      // authority — and a force that reached here has already CONSUMED the
      // token its banner was standing on. Leaving that banner up is the
      // unresolvable state C5 exists to prevent: "Keep mine" is permanently
      // dead (its authority is spent), the retry below refuses to run while the
      // page is conflicted, and the only live button, "Use disk version",
      // discards the user's edits.
      //
      // So retract the spent banner and let the ordinary retry decide. The edit
      // stays dirty and unwritten; if the disk really has diverged, the retry's
      // own base_rev guard raises a FRESH conflict with a fresh token, which is
      // a banner the user can actually act on. (GH #254 increment 2,
      // adversarial implementation verification, finding 3.)
      clearConflict(name);
      conflictObservation.delete(name);
      if (token === graphToken) {
        dirty.add(name);
        scheduleTransientRetry(name, token, e);
      }
    } else if (token === graphToken) {
      dirty.add(name); // keep pending — retried on next edit / flush
      scheduleTransientRetry(name, token, e);
    }
    return false;
  }
}

/** How long an edit waits for the typing to settle before it is written. */
const SAVE_DEBOUNCE_MS = 400;
/** …and the longest a run of edits can postpone that write by continuing.
 *
 *  Without a ceiling the debounce re-arms on every keystroke, so a fluent
 *  typist never pauses long enough to trigger it and nothing reaches disk until
 *  they stop — a crash mid-paragraph loses the paragraph. Measured from the
 *  FIRST edit of the burst, not the last, so the bound is on how stale the file
 *  can be rather than on how fast the user types. (Direct Files data-safety
 *  audit, finding 9.) */
const MAX_SAVE_DELAY_MS = 3_000;

export function scheduleSave() {
  if (!doc.loaded || reopenRequired) return;
  if (saveTimer) clearTimeout(saveTimer);
  else burstStartedAt = Date.now();
  const postponedBy = Date.now() - (burstStartedAt ?? Date.now());
  const delay = Math.max(0, Math.min(SAVE_DEBOUNCE_MS, MAX_SAVE_DELAY_MS - postponedBy));
  saveTimer = setTimeout(() => {
    saveTimer = null;
    burstStartedAt = null;
    const names = [...dirty];
    void (async () => {
      const results = await Promise.all(names.map((n) => enqueueSave(n)));
      // The backend cache now reflects these edits → let queries recompute, but
      // coalesce: re-running every on-screen query is a whole-graph scan, so wait
      // for a lull instead of firing on every 400ms save batch.
      if (results.some(Boolean)) scheduleDataRev();
    })();
  }, delay);
}

/** An external write landed on a page that still has unsaved edits. Do NOT mark
 *  it conflicted here.
 *
 *  A conflict banner is only usable if "Keep mine" can present the observation
 *  epoch it was shown, and the backend mints one exactly once: for the disk
 *  state it just refused to overwrite. A banner raised from the frontend has no
 *  epoch, so `save_page` refuses the force, the banner stays up, its retry is
 *  forbidden while the page is conflicted, and the only live action left is
 *  "Use disk version" — which discards the edit. That is the unresolvable
 *  dirty-editor tail this whole mechanism exists to prevent. (GH #254
 *  increment 2, correction-delta re-verification, HIGH blocker.)
 *
 *  So run the ordinary, base-revision-guarded save instead. It cannot overwrite
 *  a diverged file; if the disk really moved, its refusal raises the banner WITH
 *  authority through the single conflict path in `doSave`, and if it did not,
 *  the pending edit simply lands. Reading the file to prove divergence first is
 *  not enough on its own — that read revokes any authority for the path. */
export async function reconcileExternalChange(name: string): Promise<void> {
  await enqueueSave(name, { kind: "reobserve" });
}

/** Save one page immediately, bypassing the debounce — for actions that must
 *  durably persist before the user might quit (e.g. parking a block in the
 *  sidebar writes an id:: that has to survive a restart). Returns success. */
export async function flushPage(name: string): Promise<boolean> {
  if (!doc.loaded) return false;
  const ok = await enqueueSave(name);
  if (ok) scheduleDataRev();
  return ok;
}

/** Drain one page's save chain until it is clean and no write remains in flight.
 *
 * Delete uses this rather than one flush: an edit may land while its first save
 * awaits the backend, which makes the page dirty again after that save resolves.
 * The next turn must durably accept that newer snapshot too, or deletion must
 * refuse while retaining the draft.  This is deliberately page-local and
 * bounded; graph-wide flushAll semantics (assets and unrelated drafts) are not
 * part of deleting one page. */
export async function flushPageToQuiescence(name: string): Promise<boolean> {
  if (isConflicted(name) || deletedPages.has(name)) return false;
  // A clean satellite page needs no write to be quiescent. A dirty or saving
  // page still requires an armed main store so its draft can actually drain.
  if (!doc.loaded && (dirty.has(name) || saveChain.has(name))) return false;
  for (let attempt = 0; attempt < 4; attempt++) {
    if (isConflicted(name) || deletedPages.has(name)) return false;
    const inFlight = saveChain.get(name);
    if (inFlight) {
      if (!(await inFlight)) return false;
      continue;
    }
    if (!dirty.has(name)) return true;
    if (!(await enqueueSave(name))) return false;
  }
  return !dirty.has(name) && !saveChain.has(name) && !isConflicted(name) && !deletedPages.has(name);
}

/** Retire every page touched by a cut against the exact instances recorded in
 * the clipboard grant. Preflight the whole set before starting any writes, then
 * bind the same identity+generation check into each queued save at snapshot and
 * completion. A clean page still queues a checked no-op so an earlier save is
 * drained before it counts as retired. */
export async function flushCutSourcePages(sources: readonly ClipboardSourcePage[]): Promise<boolean> {
  if (
    !doc.loaded
    || sources.length === 0
    || new Set(sources.map((source) => source.name)).size !== sources.length
    || !sources.every(cutSourceUsable)
  ) return false;
  const results = await Promise.all(
    sources.map((source) => enqueueSave(source.name, ORDINARY, source)),
  );
  if (results.some(Boolean)) scheduleDataRev();
  return results.every(Boolean);
}

/** Final synchronous retirement guard used immediately before identity insert. */
export function cutSourcePagesRetired(sources: readonly ClipboardSourcePage[]): boolean {
  return sources.length > 0 && sources.every((source) =>
    cutSourceMatches(source)
    && !dirty.has(source.name)
    && !saveChain.has(source.name)
    && !deletedPages.has(source.name)
    && !heldSources.has(source.name)
    && !isConflicted(source.name)
  );
}

/** Persist every dirty page now and wait for them (incl. anything mid-write) —
 *  for graph switch / restore / app close. Returns true only if everything
 *  landed (no conflicts or errors), so the caller can abort a destructive
 *  transition rather than discard the un-saved edit. */
export async function flushAll(): Promise<boolean> {
  if (reopenRequired) return false;
  if (saveTimer) {
    clearTimeout(saveTimer);
    saveTimer = null;
  }
  let landed = false;
  // Drain repeatedly: an edit made WHILE a save is in flight re-dirties the page,
  // and a queued save may still be running, so one pass can miss work. Keep
  // flushing until nothing is pending (bounded against a persistently-failing
  // save).
  for (let i = 0; i < 4; i++) {
    const names = new Set<string>([...dirty, ...saveChain.keys()]);
    const assetWrites = [...assetWriteChain];
    if (names.size === 0 && assetWrites.length === 0) break;
    const [results] = await Promise.all([
      Promise.all([...names].map((n) => enqueueSave(n))),
      Promise.all(assetWrites),
    ]);
    if (results.some(Boolean)) landed = true;
  }
  // A conflicted page's draft is not on disk; its capsule is the only durable
  // copy. Land the debounced refresh before the caller decides to close.
  await flushLiveSaveConflictDrafts();
  if (landed) bumpDataRev();
  // Success only if nothing is still pending AND there are no unresolved
  // conflicts (a conflicted page's edit is NOT on disk) — so a destructive
  // transition (graph switch / restore / close) can abort instead of discarding it.
  return !reopenRequired
    && dirty.size === 0
    && assetWriteChain.size === 0
    && conflicts().length === 0;
}

/** Explain a refused rename flush using the state that actually blocked it.
 * The guard remains graph-wide because rename reloads every mounted page. */
export function renameFlushFailureMessage(): string {
  if (reopenRequired) {
    return "Couldn't rename: storage needs to reopen before pending edits can save. Your pending edits are still here.";
  }
  const quoted = (names: readonly string[]) => {
    const shown = names.slice(0, 3).map((name) => `“${name}”`).join(", ");
    return names.length > 3 ? `${shown}, and ${names.length - 3} more` : shown;
  };
  const conflicted = conflicts();
  if (conflicted.length) {
    const missingReview = conflicted.filter((name) => !conflictObjectFor(pageByName(name)?.path, name));
    if (missingReview.length) {
      return `Couldn't rename: saves are blocked for ${quoted(missingReview)}, but no conflict review is available. Check the save error or debug log before retrying. Your pending edits are still here.`;
    }
    return `Couldn't rename: ${quoted(conflicted)} ${conflicted.length === 1 ? "has an unresolved save conflict" : "have unresolved save conflicts"}. Open ${conflicted.length === 1 ? "that page" : "those pages"} and resolve ${conflicted.length === 1 ? "it" : "them"} before renaming. Your pending edits are still here.`;
  }
  const pending = [...new Set([...dirty, ...saveChain.keys()])];
  if (pending.length) {
    return `Couldn't save pending edits in ${quoted(pending)} before renaming. Check the save error for ${pending.length === 1 ? "that page" : "those pages"}, then try again. Your pending edits are still here.`;
  }
  return "Couldn't finish pending saves before renaming. Check the save error, then try again. Your pending edits are still here.";
}

/** Resolve a save conflict by overwriting the on-disk file with the in-memory
 *  version ("keep mine"). Returns whether the overwrite succeeded — the caller
 *  must not clear the conflict unless it did. */
export async function forceSave(name: string): Promise<boolean> {
  if (!canForceSave(name)) return false;
  dirty.add(name); // ensure doSave writes even though it's parked as conflicted
  const ok = await enqueueSave(name, forceIntent(name));
  if (!ok) pushToast(`Couldn't overwrite “${name}”.`, "error");
  return ok;
}
