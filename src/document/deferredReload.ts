/** Deferred replay of external reloads declined mid-edit (GH #337; master
 * 9869c1cfe). `reloadDisposition` answers "skip" while a block on the page is
 * being edited, a block move is in flight, or a component draft pinned the page.
 * Those refusals are right, but the watcher event they decline used to be
 * dropped: the backend cache was fresh while the visible page stayed stale until
 * some unrelated event touched it again.
 *
 * `deferExternalReload(page, change)` records the declined change per page,
 * latest observation wins, bound to the graph it was seen in. When the page's
 * gate opens (editing ends or moves to another page, the move settles, the draft
 * pin is released) the change is handed back to the ORIGINAL handler, which
 * re-evaluates the disposition at replay time: a page that turned dirty meanwhile
 * takes the ordinary conflict path, never a clobbering reload; a page that is
 * still held re-defers. A graph switch discards every record.
 *
 * `whenPageReplaceable(page, key, run)` is the same wait for work other than a
 * watcher change: a refresh or load the working set declined because the page
 * holding that name had uncommitted input (the PDF-notes refresh, a sidebar
 * item's load; GH #254 family, master 7bd793bd0). `run` is called once, when
 * the page is replaceable; the newest `run` per (page, key) wins; the returned
 * function cancels. A settled save (the data revision moving) is a transition
 * too, so a wait on a page with unsaved edits ends once they are on disk.
 *
 * Cost: O(deferred pages + waiters) per editing/move/pin/save transition;
 * nothing at all until the first deferral. Callers need not know which trigger
 * fired. */
import { createEffect, createRoot, on } from "solid-js";
import type { GraphChange } from "../backend";
import { captureBinding, clearOnBindingInvalidated, bindingCurrent, type Binding } from "../binding";
import { editingId } from "../editorController";
import { dataRev } from "../graphSession";
import { isBlockMoving } from "./edits/moves";

interface Deferred { change: GraphChange; binding: Binding }
interface Replay { ready(page: string): boolean; run(change: GraphChange): void }

const deferred = new Map<string, Deferred>();
const waiters = new Map<string, Map<string, { run: () => void; binding: Binding }>>();
let replay: Replay | null = null;

/** The handler that owns the decision, and the gate that says a page may now be
 * replaced. Installed once by the module that applies watcher changes. */
export function installDeferredReloadReplay(next: Replay): void { replay = next; }

/** Record `change` for replay once loaded page `page` is replaceable. */
export function deferExternalReload(page: string, change: GraphChange): void {
  deferred.set(page, { change, binding: captureBinding() });
  watchTransitions();
}

/** Call `run` once page `page` is replaceable; replaces an earlier wait with
 * the same `key`. Returns the cancel function. O(1). */
export function whenPageReplaceable(page: string, key: string, run: () => void): () => void {
  const entry = { run, binding: captureBinding() };
  const forPage = waiters.get(page) ?? new Map();
  forPage.set(key, entry);
  waiters.set(page, forPage);
  watchTransitions();
  return () => {
    if (forPage.get(key) === entry) forPage.delete(key);
    if (!forPage.size && waiters.get(page) === forPage) waiters.delete(page);
  };
}

/** Replay every deferred change, and run every waiter, whose page is
 * replaceable now. */
export function replayDeferredExternalReloads(): void {
  if (!replay) return;
  for (const [page, forPage] of [...waiters]) {
    if (!replay.ready(page)) continue;
    waiters.delete(page);
    for (const entry of forPage.values()) if (bindingCurrent(entry.binding)) entry.run();
  }
  if (deferred.size === 0) return;
  for (const [name, entry] of [...deferred]) {
    if (!bindingCurrent(entry.binding)) { deferred.delete(name); continue; }
    if (!replay.ready(name)) continue;
    deferred.delete(name);
    // Fire and forget like a live watcher event; a decline re-defers.
    replay.run(entry.change);
  }
}

clearOnBindingInvalidated(() => { deferred.clear(); waiters.clear(); });

// Editing ending (or moving to another block), a move settling and a save
// settling (the data revision) each change a signal read here; the pin release calls `replayDeferredExternalReloads` itself.
// Created on the first deferral, not at module load: the moves module is still
// initialising while this one is imported, and nothing is watched until then.
let watching = false;
function watchTransitions(): void {
  if (watching) return;
  watching = true;
  createRoot(() => createEffect(on([editingId, () => isBlockMoving(), dataRev], replayDeferredExternalReloads, { defer: true })));
}
