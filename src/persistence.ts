// The debounced persistence engine — extracted from store.ts so the save
// invariant lives in ONE owner: the debounce, the per-page serial write queue,
// and the graph-token / baseRev / tombstone / conflict guards that keep edits
// from being lost, clobbered, or written into the wrong graph.
//
// store.ts owns the doc tree and calls markDirty(page) on every mutation; this
// module decides WHEN and HOW that reaches disk. It depends on store only for a
// page snapshot (pageToDto) and the loaded flag (doc.loaded) — used at call time,
// so the store↔persistence import cycle resolves cleanly.

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
import { backend } from "./backend";
import { onGraphRebound } from "./modeHooks";
import { markConflict, clearConflict, isConflicted, conflicts, bumpDataRev, bumpPageInventoryRev, pushToast } from "./ui";
import type { ClipboardSourcePage } from "./clipboard";
import { measureIssue248, measureIssue248Async } from "./issue248Probe";

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
// A "keep mine" the move barrier deferred, WITH the observation epoch the user
// clicked under. Re-issuing it later must present that epoch and not whatever is
// current by then: an epoch minted after the click belongs to a winner the user
// never saw. (GH #254 increment 2, third correction-delta re-verification.)
const heldForcedSaves = new Map<string, number | null>();
const heldByDest = new Map<string, string[]>();
// Which conflict observation each banner is showing. "Keep mine" presents this
// back so the override answers the conflict the USER SAW. Without it, a second
// force request issued under one banner — a double click, the button is not
// disabled while its request is pending — consumes authority the first request
// minted for a NEWER external winner, and overwrites bytes nobody was shown.
// (GH #254 increment 2, adversarial implementation verification, finding 1.)
const conflictObservation = new Map<string, number>();

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
      // otherwise be silently lost. It carries the epoch the user clicked under,
      // so if the disk moved while the barrier held it, the backend refuses and
      // the refusal below re-raises a banner the user can decide on again.
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
/** Tombstone a page so any pending/in-flight save can't recreate its file. */
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

/**
 * Was `path` the file this tombstone was raised for?
 *
 * A tombstone is keyed by page name, but two files legitimately share one name
 * (the duplicate-day stray of #21, same-titled pages in different folders). A
 * name-level test therefore refuses the SURVIVING file as well as the deleted
 * one — which loses work rather than protecting it. When the delete named a
 * path, only that exact file is refused. (GH #254 increment 3.)
 */
export function isTombstonedFile(name: string, path?: string): boolean {
  if (!deletedPages.has(name)) return false;
  const deleted = deletedPagePaths.get(name);
  // No path recorded (a never-saved page): nothing distinguishes the files, so
  // the name is all there is to go on.
  if (!deleted || !path) return true;
  return deleted === path;
}

/**
 * Does a tombstone PROVABLY cover this file? The same question as
 * `isTombstonedFile`, with the opposite answer when the file is unknown.
 *
 * The two exist because "unknown path" means opposite things at the two
 * boundaries. Deciding whether to INSTALL bytes, not knowing which file they
 * came from must refuse — installing the deleted one is the harm. Deciding
 * whether to WAIT, not knowing must proceed: refusing means never reading, so a
 * request that cannot name its file waits forever behind a tombstone raised for
 * some other file of that name. Block autocomplete supplies no path, so that is
 * the common case, not the exotic one. (GH #254 increment 3.)
 */
export function tombstoneCovers(name: string, path?: string): boolean {
  if (!deletedPages.has(name)) return false;
  const deleted = deletedPagePaths.get(name);
  if (!deleted) return true; // pathless delete: every file of that name
  return deleted === path; // unknown path proves nothing — let the read decide
}

export function tombstone(name: string, path?: string) {
  deletedPages.add(name);
  // Always REPLACE the recorded file, never merge with an older tombstone's: a
  // pathless delete means "every file with this name", and inheriting a stale
  // path from a previous tombstone would silently narrow it to one file — the
  // wrong one, letting the others be resurrected. (GH #254 increment 3.)
  if (path) deletedPagePaths.set(name, path);
  else deletedPagePaths.delete(name);
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
  graphBindingRev++; // a switch is also a rebind
  dirty.clear();
  baseRev.clear();
  conflictObservation.clear();
  deletedPages.clear();
  deletedPagePaths.clear();
  heldSources.clear();
  heldByDest.clear();
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

/** Save failures the backend reports with a bounded code that a retry cannot
 *  change: the graph has two files whose names collide on case-insensitive
 *  filesystems, two paths pointing at one physical file, a symlink where a page
 *  was expected, or another page already holding this title. Retrying re-runs
 *  the whole pre-save check — on a large graph the expensive part — to arrive at
 *  the same answer, three times, per dirty page. Report it once instead. */
export function isRetryableSaveFailure(error: unknown): boolean {
  const message = String(error);
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
  ].some((code) => message.includes(code));
}

/** The bounded failure code the Direct backend prefixes to a save error.
 *
 *  `direct_save_error_message` emits `"{code}: {raw error}"`, and many raw errors
 *  carry a graph-relative PATH. So a family test has to read the code, not search
 *  the whole string: a page legitimately named `conflict_authority.notes` would
 *  otherwise make an unrelated `precheck.symlink` failure look like a spent
 *  override, and its handler would keep re-observing a save that can never
 *  succeed. (GH #254 increment 2, fourth correction-delta re-verification.)
 *
 *  Returns `""` when there is no code separator or the prefix is not code-shaped
 *  — including the banner-class `conflict` / `conflict:<n>` shape, which is
 *  matched by `conflictObservationEpoch` before this is consulted. */
function directSaveFailureCode(error: unknown): string {
  const message = String(error).replace(/^Error: /, "");
  const separator = message.indexOf(": ");
  if (separator <= 0) return ""; // no code at all — never a family
  const code = message.slice(0, separator);
  // A bounded code is dot-separated, lower-case and underscored — usually
  // `family.condition`, but `unknown` is a genuine single-segment one. Requiring
  // the SHAPE means an error that merely opens with code-like prose cannot be
  // read as one: without it, `Error("conflict_authority.spent while reporting
  // …")` was accepted whole and routed into the authority handler.
  return /^[a-z][a-z_]*(\.[a-z][a-z_]*)*$/.test(code) ? code : "";
}

/** The observation epoch a banner-class conflict was raised at.
 *
 *  `conflict:<n>` is the whole contract for the keep-mine/use-disk banner; the
 *  bare `conflict` is the legacy shape and means the backend minted authority it
 *  could not name, so no override may be presented for it. Returns null when the
 *  error is not banner class at all, and -1 for the unnameable legacy shape. */
function conflictObservationEpoch(error: unknown): number | null {
  const message = String(error).replace(/^Error: /, "");
  if (message === "conflict") return -1;
  const match = /^conflict:(\d+)$/.exec(message);
  return match ? Number(match[1]) : null;
}

function scheduleTransientRetry(name: string, token: number, error: unknown) {
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
    if (token === graphToken && dirty.has(name) && !isConflicted(name)) {
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
  | { kind: "force"; observation: number | null };

const ORDINARY: SaveIntent = { kind: "ordinary" };

/** Capture the epoch the visible banner is showing, at the moment the user acts.
 *  Reading it later — when the queued request finally reaches the backend — is
 *  the bug: a re-observation running ahead of it in the queue replaces the entry
 *  with an epoch minted for a winner the user never saw, and the force would
 *  then be authorised to discard exactly that. */
function forceIntent(name: string): SaveIntent {
  return { kind: "force", observation: conflictObservation.get(name) ?? null };
}

/** The exact observation the banner for `name` is showing, captured AT THE CLICK.
 *
 *  Read at click time, never later: a re-observation running ahead of a queued
 *  request replaces the entry with an epoch minted for a winner the user never
 *  saw, and answering with that would discard exactly that winner. */
export function shownObservationFor(name: string): number | null {
  return conflictObservation.get(name) ?? null;
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
  const token = graphToken;
  // The page's path cannot see a SAME-PATH content replacement — `reloadPage` and
  // the watcher-approved reload both install a new editor at the same path — so
  // the instance generation is required as well. Peeked, never read through the
  // lazily-creating accessor: that would mint a generation where none existed and
  // mutate the identity cut retirement compares.
  const pathAtStart = page.path ?? "";
  const instanceAtStart = peekPageInstanceGeneration(name);
  try {
    const handle = page.path
      ? await backend().activateEditor(page.path, "reuse")
      : await backend().activateAbsentEditor(name, page.kind);
    // Re-check across the await. A graph switch or a re-install makes this a
    // DIFFERENT editor, and recording the handle then would hand one editor's
    // identity to another — reproduced writing a replacement graph's page.
    if (
      editorActivationFor(name) === undefined &&
      graphToken === token &&
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
  // A re-observation must reach the backend even though the page is clean or
  // already conflicted: those are exactly the states a banner leaves behind, and
  // only a fresh refusal can mint the authority the visible banner needs.
  if (intent.kind === "ordinary" && !dirty.has(name)) return true; // saved by a prior link
  if (intent.kind === "ordinary" && isConflicted(name)) return false;
  // A cross-page move source: hold its save until the destination is durable (C#1).
  // Stays dirty, so it writes the moment `releaseSourcesFor(dest)` frees it.
  if (heldSources.has(name)) {
    if (intent.kind === "force") heldForcedSaves.set(name, intent.observation);
    return false;
  }
  const token = graphToken;
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
  if (graphToken !== token) return false;
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
    console.warn("Refusing to persist ephemeral bundled Guide page", name);
    dirty.delete(name);
    return true;
  }
  if (dto.read_only) {
    console.error("Refusing to persist read-only page", name);
    dirty.delete(name);
    return false;
  }
  dirty.delete(name);
  try {
    const baseline = baseRev.get(name) ?? null;
    const rev = await measureIssue248Async("frontend.savePageAwaitMs", () =>
      backend().savePage(dto, baseline, force, intent.kind === "force" ? intent.observation : null)
    );
    // A reload/rename/delete/rebind while savePage was in flight invalidates the
    // retirement proof even if those bytes landed. Never let that stale success
    // authorize identity reuse or update the replacement instance's baseline.
    if (expectedCutSource && !cutSourceUsable(expectedCutSource)) return false;
    if (token === graphToken) {
      baseRev.set(name, rev);
      if (baseline === null) bumpPageInventoryRev();
    }
    clearTransientRetry(name);
    conflictObservation.delete(name);
    // The bytes landed, so any banner still up is answered. Only a re-observation
    // can arrive here with one raised (the ordinary path refuses a conflicted
    // page and a force is followed by its own clear), and leaving it would park
    // the page behind a warning about a change that is now written.
    if (isConflicted(name)) clearConflict(name);
    releaseSourcesFor(name); // if this was a cross-page dest, its sources can save now
    return true;
  } catch (e) {
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
      if (observed >= 0) conflictObservation.set(name, observed);
      else conflictObservation.delete(name);
      markConflict(name);
    } else if (directSaveFailureCode(e).startsWith("conflict_authority.")) {
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
    } else if (directSaveFailureCode(e).startsWith("conflict_retry.")) {
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
  if (!doc.loaded) return;
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
  if (landed) bumpDataRev();
  // Success only if nothing is still pending AND there are no unresolved
  // conflicts (a conflicted page's edit is NOT on disk) — so a destructive
  // transition (graph switch / restore / close) can abort instead of discarding it.
  return dirty.size === 0 && assetWriteChain.size === 0 && conflicts().length === 0;
}

/** Resolve a save conflict by overwriting the on-disk file with the in-memory
 *  version ("keep mine"). Returns whether the overwrite succeeded — the caller
 *  must not clear the conflict unless it did. */
export async function forceSave(name: string): Promise<boolean> {
  dirty.add(name); // ensure doSave writes even though it's parked as conflicted
  const ok = await enqueueSave(name, forceIntent(name));
  if (!ok) pushToast(`Couldn't overwrite “${name}”.`, "error");
  return ok;
}
