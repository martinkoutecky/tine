import { createSignal } from "solid-js";
import { backend } from "./backend";
import { graphEpoch } from "./ui";

// Inline page-icon lookups, made cheap for icon-heavy pages. Every `icon::`
// requested in the same microtask tick is coalesced into ONE page_icons IPC, each
// name is fetched at most ONCE per open graph, and — crucially — a batch that
// resolves NO icons does NOT touch the signal, so the common case (few/no icon::
// pages, e.g. a 2000-block page of plain refs) costs one IPC and zero re-renders.
// A plain reactive read (not createResource-per-ref) keeps the per-reference cost
// to a signal read + object lookup. Icons refresh on graph switch / reopen, like
// the block-ref batch — not on every edit.
let cacheRev = -1;
const [iconMap, setIconMap] = createSignal<Record<string, string>>({});
const requested = new Set<string>();
let pending: string[] = [];
let scheduled = false;

function ensureRev() {
  const epoch = graphEpoch();
  if (epoch !== cacheRev) {
    cacheRev = epoch;
    setIconMap({}); // new graph — drop the previous graph's icons
    requested.clear();
    pending = [];
  }
  return epoch;
}

function flush() {
  scheduled = false;
  if (!pending.length) return;
  const batch = pending;
  pending = [];
  const batchRev = cacheRev;
  void backend()
    .pageIcons(batch)
    .then((map) => {
      if (graphEpoch() !== batchRev) return;
      // Only re-render if the batch actually found an icon — otherwise the signal
      // (and every reference subscribed to it) stays untouched.
      if (batch.some((n) => map[n])) setIconMap((prev) => ({ ...prev, ...map }));
    })
    .catch(() => {});
}

/** Reactive: the referenced page's `icon::` (emoji/character) or "". Reading it
 *  lazily enqueues a batched fetch; the value fills in once the batch resolves. */
export function pageIcon(name: string): string {
  ensureRev();
  if (!requested.has(name)) {
    requested.add(name);
    pending.push(name);
    if (!scheduled) {
      scheduled = true;
      queueMicrotask(flush);
    }
  }
  return iconMap()[name] ?? "";
}
