// "Changed since you last looked" (vision decision 9a, ADR 0073): each tracked
// page's seen baseline, a set of block hashes kept in app data, never in the
// graph. This module is the one owner of that record on the frontend: it loads
// a page's baseline once, writes it only on "Mark page seen", and drops it on
// "Forget seen state" or when Tine renames or deletes the page. Nothing here
// runs per edit, at page open beyond one load, or at close.
import { createSignal, untrack } from "solid-js";
import { backend, type SeenBaselineRequest } from "../backend";
import { graphMeta } from "../graphSession";
import { pageIdentityKey } from "../pageIdentity";
import { bindingOwner, graphOwner, readOwned, writeOwned } from "../owned";
import { loadedPage, node } from "../document";
import { pushToast } from "../toasts";
import { dbg } from "../debug";
import { pageBlockIds, seenBlockHash } from "./hash";

/** A page's baseline: its block hashes, or null for a page that is not tracked. */
export type SeenBaseline = ReadonlySet<string> | null;

// Keyed by graph root + page identity key, so a graph switch never shows
// another graph's answer and a spelling change of one page reads one record.
const [baselines, setBaselines] = createSignal<ReadonlyMap<string, SeenBaseline>>(new Map());
const inflight = new Set<string>();

const cacheKey = (root: string, pageName: string): string => `${root}\n${pageIdentityKey(pageName)}`;
const publish = (key: string, value: SeenBaseline) => setBaselines((prev) => new Map(prev).set(key, value));

function request(op: SeenBaselineRequest): Promise<string[] | null> {
  const io = backend().seenBaseline;
  if (!io) return Promise.reject(new Error("This build keeps no seen state."));
  return io.call(backend(), op);
}

/** Can this backend keep seen state? A published export cannot. */
export function seenSupported(): boolean {
  return typeof backend().seenBaseline === "function";
}

/** Reactive: the page's baseline, null when it is not tracked, undefined until
 *  its one load has answered. */
export function seenBaselineFor(pageName: string): SeenBaseline | undefined {
  const root = graphMeta()?.root;
  return root ? baselines().get(cacheKey(root, pageName)) : undefined;
}

/** Load the page's baseline once per graph session. A missing, unreadable or
 *  corrupt record, or a failed read, is "not tracked": never an error dialog
 *  and never a refusal to open the page (ADR 0073 failure behaviour). */
export function loadSeenBaseline(pageName: string): void {
  const root = graphMeta()?.root;
  if (!root || !seenSupported()) return;
  const key = cacheKey(root, pageName);
  if (untrack(baselines).has(key) || inflight.has(key)) return;
  inflight.add(key);
  const owner = graphOwner();
  void readOwned(owner, request({ op: "load", graph: root, page: pageIdentityKey(pageName) }))
    .then((result) => {
      if (result.kind !== "stale") publish(key, result.value ? new Set(result.value) : null);
    }, (error) => {
      dbg(`seen: reading the baseline of ${pageName} failed: ${String(error)}`);
      if (owner()) publish(key, null);
    })
    .finally(() => inflight.delete(key));
}

/** The hashes of every block of a loaded page as it is shown now. O(page). */
export function currentPageHashes(pageName: string): string[] {
  const page = loadedPage(pageName);
  if (!page) return [];
  return [...new Set(pageBlockIds(pageName).map((id) => seenBlockHash(node(id).raw, page.format)))];
}

/** Write the page's baseline: what it shows now becomes "seen". Starts tracking
 *  a page that had none. One app-data file, through the audited atomic writer;
 *  a failure is reported and changes nothing. */
export async function markPageSeen(pageName: string): Promise<boolean> {
  const root = graphMeta()?.root;
  const page = loadedPage(pageName);
  if (!root || !page || page.guide || !seenSupported()) return false;
  const hashes = currentPageHashes(pageName);
  try {
    // The record names the graph the hashes came from, so a graph switch while
    // it is in flight cannot file it under another graph.
    await writeOwned(bindingOwner(), request({ op: "mark", graph: root, page: pageIdentityKey(pageName), hashes }));
    publish(cacheKey(root, pageName), new Set(hashes));
    return true;
  } catch (error) {
    pushToast(`Couldn't mark “${pageName}” seen. (${String(error)})`, "error");
    return false;
  }
}

/** Stop tracking a page: its record is removed and the page renders as an
 *  untracked page again. `quiet` (rename / delete) only logs a failure. */
export async function forgetPageSeen(pageName: string, quiet = false): Promise<void> {
  const root = graphMeta()?.root;
  if (!root || !seenSupported()) return;
  try {
    await writeOwned(bindingOwner(), request({ op: "forget", graph: root, page: pageIdentityKey(pageName) }));
    publish(cacheKey(root, pageName), null);
  } catch (error) {
    if (quiet) dbg(`seen: forgetting ${pageName} failed: ${String(error)}`);
    else pushToast(`Couldn't forget the seen state of “${pageName}”. (${String(error)})`, "error");
  }
}

/** Test seam: drop every cached answer. */
export function resetSeenBaselinesForTests(): void {
  setBaselines(new Map());
  inflight.clear();
}
