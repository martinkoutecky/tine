// The debounced persistence engine — extracted from store.ts so the save
// invariant lives in ONE owner: the debounce, the per-page serial write queue,
// and the graph-token / baseRev / tombstone / conflict guards that keep edits
// from being lost, clobbered, or written into the wrong graph.
//
// store.ts owns the doc tree and calls markDirty(page) on every mutation; this
// module decides WHEN and HOW that reaches disk. It depends on store only for a
// page snapshot (pageToDto) and the loaded flag (doc.loaded) — used at call time,
// so the store↔persistence import cycle resolves cleanly.

import { doc, forgetPage, loadSingle, pageByName, pageInstanceGeneration, pageToDto, reloadPage, setPageId } from "./store";
import { openPage } from "./router";
import { backend } from "./backend";
import { captureBinding, stillBound, type Binding } from "./binding";
import { errorFamily } from "./errorFamily";
import { markConflict, isConflicted, conflicts, bumpDataRev, bumpPageInventoryRev, pushToast } from "./ui";
import type { ClipboardSourcePage } from "./clipboard";

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
// Per-page save queue: writes for one page run strictly one-after-another (never
// concurrently) and each runs against the LATEST store state.
const saveChain = new Map<string, Promise<boolean>>();
const lastSaveFailure = new Map<string, string>();
let saveTimer: ReturnType<typeof setTimeout> | null = null;
let dataRevTimer: ReturnType<typeof setTimeout> | null = null;
const assetWriteChain = new Set<Promise<boolean>>();
// Cross-page move barrier (audit C#1): while a moved subtree's DESTINATION write is not
// yet durable, the SOURCE pages must not save their post-removal state — otherwise an
// UNRELATED edit to a source during the dest-write window marks it dirty and writes the
// block out of existence (gone from the source on disk, not yet in the dest). `heldSources`
// = pages whose saves are blocked; `heldByDest` maps each dest to the sources waiting on
// it, released the moment that dest saves durably (immediately, or after a conflict is
// resolved). Until then the source keeps the block on disk, so it's never lost.
const heldSources = new Map<string, Set<string>>();
const heldByDest = new Map<string, Set<string>>();

// ---------------------------------------------------------------------------
// Accessors — store.ts mutations call these instead of touching the guards.
// ---------------------------------------------------------------------------

/** Does a page have unsaved (debounced) edits pending? */
export function isDirty(name: string): boolean {
  return dirty.has(name);
}
/** Mark a page dirty and schedule a debounced save. */
export function markDirty(name: string) {
  const page = pageByName(name);
  if (page?.readOnly || page?.guide) return;
  dirty.add(name);
  scheduleSave();
}
/** Mark dirty WITHOUT scheduling — undo/redo restore batches several pages then
 *  schedules once. */
export function addDirty(name: string) {
  const page = pageByName(name);
  if (page?.readOnly || page?.guide) return;
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
/** Hold `sources`' saves until `dest` is durably written (cross-page move barrier,
 *  audit C#1). `releaseSourcesFor(dest)` fires from doSave's success path. */
export function holdSourcesForDest(dest: string, sources: string[]) {
  const srcs = sources.filter((s) => s !== dest);
  if (srcs.length === 0) return;
  let held = heldByDest.get(dest);
  if (!held) {
    held = new Set();
    heldByDest.set(dest, held);
  }
  for (const s of srcs) {
    held.add(s);
    let dependencies = heldSources.get(s);
    if (!dependencies) {
      dependencies = new Set();
      heldSources.set(s, dependencies);
    }
    dependencies.add(dest);
  }
}

/** Undo of an as-yet-unpublished move returns content to its still-durable
 * original page. Retire that old dependency before installing the reverse one,
 * otherwise each page can end up waiting for the other to save. */
export function cancelSourceHoldForDest(source: string, dest: string): void {
  const sources = heldByDest.get(dest);
  if (!sources?.delete(source)) return;
  if (sources.size === 0) heldByDest.delete(dest);
  const dependencies = heldSources.get(source);
  dependencies?.delete(dest);
  if (dependencies?.size === 0) heldSources.delete(source);
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
    const dependencies = heldSources.get(s);
    dependencies?.delete(dest);
    if (dependencies && dependencies.size === 0) {
      heldSources.delete(s);
      dirty.add(s); // its removal (and any held edit) can write now
      any = true;
    }
  }
  if (any) scheduleSave();
}
/** Record a page's load/save baseline rev (set on load and after each save). */
export function setBaseRev(name: string, rev: string | null) {
  baseRev.set(name, rev);
}
/** Tombstone a page so any pending/in-flight save can't recreate its file. */
export function tombstone(name: string) {
  deletedPages.add(name);
}
/** Lift a delete tombstone (page re-created, or the delete failed). */
export function untombstone(name: string) {
  deletedPages.delete(name);
}
/** Drop a page's dirty + baseline state — its content is leaving the working set. */
export function forgetSaveState(name: string) {
  dirty.delete(name);
  baseRev.delete(name);
  lastSaveFailure.delete(name);
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
  dirty.clear();
  baseRev.clear();
  deletedPages.clear();
  heldSources.clear();
  heldByDest.clear();
  lastSaveFailure.clear();
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

function cutSourceMatches(expected: ClipboardSourcePage): boolean {
  const page = pageByName(expected.name);
  return !!page
    && page.name === expected.name
    && page.kind === expected.kind
    // A grant taken before the page's first save has no file id; that save
    // then records the id it created (`setPageId`) on the same instance, so
    // the generation check alone pins it.
    && (expected.path === undefined || page.id === expected.path)
    && pageInstanceGeneration(expected.name) === expected.generation;
}

function cutSourceUsable(expected: ClipboardSourcePage): boolean {
  return cutSourceMatches(expected)
    && !deletedPages.has(expected.name)
    && !isConflicted(expected.name);
}

function enqueueSave(
  name: string,
  force = false,
  expectedCutSource?: ClipboardSourcePage,
): Promise<boolean> {
  const binding = captureBinding();
  const token = graphToken;
  const generation = pageInstanceGeneration(name);
  const prev = saveChain.get(name) ?? Promise.resolve(true);
  const next = prev.then(
    () => doSave(name, force, binding, token, generation, expectedCutSource),
    () => doSave(name, force, binding, token, generation, expectedCutSource),
  );
  saveChain.set(name, next);
  void next.finally(() => {
    if (saveChain.get(name) === next) saveChain.delete(name);
  });
  return next;
}

/** Write the page's CURRENT state once. No-op success if it isn't dirty and not
 *  forced. Sends `baseRev` (the version the editor loaded) so the backend
 *  conflicts against external changes; updates the baseline on success. On a
 *  conflict marks it (no clobber); on a transient error keeps it dirty + toasts. */
async function doSave(
  name: string,
  force: boolean,
  binding: Binding,
  token: number,
  generation: number | null,
  expectedCutSource?: ClipboardSourcePage,
): Promise<boolean> {
  if (!stillBound(binding) || token !== graphToken || pageInstanceGeneration(name) !== generation) return false;
  // A cut-retirement save is authority-bound to the exact loaded page instance.
  // Check when this queued operation actually reaches its snapshot boundary, not
  // only when the caller enqueues it: another save may have been ahead of it.
  if (expectedCutSource && !cutSourceUsable(expectedCutSource)) return false;
  if (deletedPages.has(name)) return true; // tombstoned — never recreate a deleted page
  if (!force && !dirty.has(name)) return true; // already saved by a prior link
  if (isConflicted(name) && !force) return false;
  // A cross-page move source: hold its save until the destination is durable (C#1).
  // Stays dirty, so it writes the moment `releaseSourcesFor(dest)` frees it.
  if (heldSources.has(name) && !force) return false;
  const dto = pageToDto(name);
  if (!dto) return false;
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
    let id = pageByName(name)?.id;
    if (!id) {
      const resolved = await backend().resolvePage(dto.name, dto.kind);
      if (!stillBound(binding) || token !== graphToken || pageInstanceGeneration(name) !== generation) return false;
      if (resolved.kind === "alias") {
        // A pathless draft may acquire an alias while it is open. Its blocks
        // belong to the alias owner, but the owner's existing bytes must win
        // the front of the page. Read its current revision and use an ordinary
        // guarded save; forceSave must not clobber an externally edited owner.
        const owner = resolved.owners[0] && await backend().getPageByPath(resolved.owners[0]);
        if (!stillBound(binding) || token !== graphToken || pageInstanceGeneration(name) !== generation) return false;
        if (!owner || owner.read_only || owner.guide || isDirty(owner.name) || isSaving(owner.name) || isConflicted(owner.name)) {
          throw new Error("conflict");
        }
        const ownerGeneration = pageInstanceGeneration(owner.name);
        // A draft's property-only first root is folded into pre_block by
        // pageToDto. Keep those bytes too: on the owner they are ordinary
        // appended content, never a replacement for the owner's preamble.
        const draftBlocks = dto.pre_block
          ? [{ id: "", raw: dto.pre_block, collapsed: false, children: [] }, ...dto.blocks]
          : dto.blocks;
        const appended = { ...owner, blocks: [...owner.blocks, ...draftBlocks] };
        const ownerRev = await backend().savePage(owner.id, appended, owner.rev ?? null, false, binding.backendGeneration);
        if (!stillBound(binding) || token !== graphToken || pageInstanceGeneration(name) !== generation) return false;
        if (dirty.has(name) || isDirty(owner.name) || isSaving(owner.name)
            || isConflicted(owner.name) || pageInstanceGeneration(owner.name) !== ownerGeneration) {
          // The saved snapshot is durable, but a later edit must remain visible
          // in the draft rather than being discarded by the route change.
          throw new Error("conflict");
        }
        const landed = { ...appended, rev: ownerRev };
        forgetPage(name);
        reloadPage(landed);
        loadSingle(landed);
        openPage(owner.name, owner.kind);
        pushToast(`Moved “${name}” into its alias owner “${owner.name}”.`, "info");
        bumpPageInventoryRev();
        releaseSourcesFor(name);
        lastSaveFailure.delete(name);
        return true;
      }
      id = resolved.id;
    }
    const rev = await backend().savePage(id, dto, baseline, force, binding.backendGeneration);
    // A reload/rename/delete/rebind while savePage was in flight invalidates the
    // retirement proof even if those bytes landed. Never let that stale success
    // authorize identity reuse or update the replacement instance's baseline.
    if (expectedCutSource && !cutSourceUsable(expectedCutSource)) return false;
    if (token === graphToken && stillBound(binding) && pageInstanceGeneration(name) === generation) {
      // Record the file this save wrote, but only on the instance that asked:
      // a reload/rebind meanwhile carries its own id.
      setPageId(name, id);
      baseRev.set(name, rev);
      if (baseline === null) bumpPageInventoryRev();
      releaseSourcesFor(name); // only this binding may release its held sources
      lastSaveFailure.delete(name);
      return true;
    }
    return false;
  } catch (e) {
    if (token === graphToken && stillBound(binding) && pageInstanceGeneration(name) === generation) {
      const family = errorFamily(e);
      if (family === "conflict" || family === "deleted" || family === "twin"
          || family === "read-only" || family === "invalid-target") {
        markConflict(name);
      } else {
        dirty.add(name); // keep pending — retried on next edit / flush
      }
      if (family !== "conflict" && lastSaveFailure.get(name) !== family) {
        pushToast(`Couldn't save “${name}” — ${family === "deleted" ? "the file was deleted on disk; your edits remain in the editor" : String(e)}`, "error");
        lastSaveFailure.set(name, family);
      }
    }
    return false;
  }
}

export function scheduleSave() {
  if (!doc.loaded) return;
  if (saveTimer) clearTimeout(saveTimer);
  saveTimer = setTimeout(() => {
    saveTimer = null;
    const names = [...dirty];
    void (async () => {
      const results = await Promise.all(names.map((n) => enqueueSave(n)));
      // The backend cache now reflects these edits → let queries recompute, but
      // coalesce: re-running every on-screen query is a whole-graph scan, so wait
      // for a lull instead of firing on every 400ms save batch.
      if (results.some(Boolean)) scheduleDataRev();
    })();
  }, 400);
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
    sources.map((source) => enqueueSave(source.name, false, source)),
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
  return dirty.size === 0 && saveChain.size === 0 && assetWriteChain.size === 0 && conflicts().length === 0;
}

/** Resolve a save conflict by overwriting the on-disk file with the in-memory
 *  version ("keep mine"). Returns whether the overwrite succeeded — the caller
 *  must not clear the conflict unless it did. */
export async function forceSave(name: string): Promise<boolean> {
  dirty.add(name); // ensure doSave writes even though it's parked as conflicted
  const ok = await enqueueSave(name, true);
  if (!ok) pushToast(`Couldn't overwrite “${name}”.`, "error");
  return ok;
}
