// "Changed since you last looked" (vision decision 9a, ADR 0073): each tracked
// page's seen baseline, a set of block hashes kept in app data, never in the
// graph. This module is the one owner of that record on the frontend: it loads
// a page's baseline once, writes it only on "Mark page seen", and drops it on
// "Forget seen state" or when Tine renames or deletes the page. Nothing here
// runs per edit, at page open beyond one load, or at close.
import { createSignal } from "solid-js";
import { backend } from "../backend";
import { clearOnBindingInvalidated } from "../binding";
import { graphMeta } from "../graphSession";
import { pageIdentityKey } from "../pageIdentity";
import { bindingOwner, graphOwner, readOwned, writeOwned } from "../owned";
import { loadedPage, node } from "../document";
import { pushToast } from "../toasts";
import { dbg } from "../debug";
import { pageBlockIds, seenBlockHash } from "./hash";

/** A page's baseline: its block hashes, or null for a page that is not tracked. */
export type SeenBaseline = ReadonlySet<string> | null;

// Keyed by graph root + page identity key, so a late answer never lands under
// another graph and a spelling change of one page reads one record. Both end
// with the graph binding (I-21), so the cache holds at most the pages this
// graph session opened.
const baselines = new Map<string, SeenBaseline>();
const inflight = new Set<string>();
const [version, setVersion] = createSignal(0);
clearOnBindingInvalidated(() => {
  baselines.clear();
  inflight.clear();
  setVersion((n) => n + 1);
});

const cacheKey = (root: string, pageName: string): string => `${root}\n${pageIdentityKey(pageName)}`;
const publish = (key: string, value: SeenBaseline) => {
  baselines.set(key, value);
  setVersion((n) => n + 1);
};

/** Can this backend keep seen state? A published export cannot. */
export function seenSupported(): boolean {
  const io = backend();
  return typeof io.readSeenBaseline === "function" && typeof io.writeSeenBaseline === "function";
}

/** Reactive: the page's baseline, null when it is not tracked, undefined until
 *  its one load has answered. */
export function seenBaselineFor(pageName: string): SeenBaseline | undefined {
  const root = graphMeta()?.root;
  version();
  return root ? baselines.get(cacheKey(root, pageName)) : undefined;
}

/** Load the page's baseline once per graph session. A missing, unreadable or
 *  corrupt record, or a failed read, is "not tracked": never an error dialog
 *  and never a refusal to open the page (ADR 0073 failure behaviour). */
export function loadSeenBaseline(pageName: string): void {
  const root = graphMeta()?.root;
  if (!root || !seenSupported()) return;
  const key = cacheKey(root, pageName);
  if (baselines.has(key) || inflight.has(key)) return;
  inflight.add(key);
  const owner = graphOwner();
  void readOwned(owner, backend().readSeenBaseline!(root, pageIdentityKey(pageName)))
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
    await writeOwned(bindingOwner(), backend().writeSeenBaseline!({ op: "mark", graph: root, page: pageIdentityKey(pageName), hashes }));
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
    await writeOwned(bindingOwner(), backend().writeSeenBaseline!({ op: "forget", graph: root, page: pageIdentityKey(pageName) }));
    publish(cacheKey(root, pageName), null);
  } catch (error) {
    if (quiet) dbg(`seen: forgetting ${pageName} failed: ${String(error)}`);
    else pushToast(`Couldn't forget the seen state of “${pageName}”. (${String(error)})`, "error");
  }
}

/** Test seam: drop every cached answer. */
export function resetSeenBaselinesForTests(): void {
  baselines.clear();
  inflight.clear();
  setVersion((n) => n + 1);
}
