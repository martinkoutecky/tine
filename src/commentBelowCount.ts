// The "N below" count of a folded block (vision §3.7; GH #891 review round 2,
// PC-1/PC-3/PC-4/PC-5), maintained like `createCommentChildren`: structure (the
// block's ancestors and the hidden subtree it walks) is tracked; text is not. A
// structural change re-walks the subtree once with untracked reads. A raw edit
// of one walked block re-derives only that block: an O(1) `tine.quote` screen,
// plus a facet lookup only for comment candidates. It re-walks only when that
// block's comment membership changed, and adjusts the count when a counted
// thread was resolved or reopened. Typing in the folded block costs one screen.

import { createComputed, createSignal, onCleanup, untrack, type Accessor } from "solid-js";
import { isCommentId, node, onBlockRawChanged } from "./document";
import { threadResolved } from "./commentResolution";
import type { Format } from "./types";

/** Open thread roots hidden below `id`'s direct children, counted once per
 * thread (never its replies); 0 when `active` is false or `id` is itself in a
 * thread. Same answer as the imperative `descendantOpenThreads`. */
export function createFoldedThreadCount(id: Accessor<string>, format: Accessor<Format>, active: Accessor<boolean>): Accessor<number> {
  const [membership, remeasure] = createSignal(undefined, { equals: false });
  const [open, setOpen] = createSignal(0);
  createComputed(() => {
    membership();
    if (!active()) { setOpen(0); return; }
    const fmt = format();
    // Each walked block's comment status, and each counted thread's openness.
    const walked = new Map<string, boolean>();
    const counted = new Map<string, boolean>();
    const comment = (at: string) => { const is = untrack(() => isCommentId(at, fmt)); walked.set(at, is); return is; };
    let inThread = false;
    for (let at: string | null = id(); at !== null; at = node(at)?.parent ?? null) if (comment(at)) inThread = true;
    const visit = (at: string) => {
      if (comment(at)) { counted.set(at, untrack(() => !threadResolved(at))); return; }
      for (const child of node(at)?.children ?? []) visit(child);
    };
    // A direct child's thread belongs to this block's own entry, not "below".
    if (!inThread) for (const child of node(id())?.children ?? []) if (!comment(child)) for (const below of node(child)?.children ?? []) visit(below);
    let count = 0;
    for (const isOpen of counted.values()) if (isOpen) count++;
    setOpen(count);
    onCleanup(onBlockRawChanged(changed => {
      if (isCommentId(changed, fmt) !== walked.get(changed)) { remeasure(); return; }
      const was = counted.get(changed);
      if (was === undefined) return;
      const now = !threadResolved(changed);
      if (now !== was) { counted.set(changed, now); setOpen(value => value + (now ? 1 : -1)); }
    }, walked.keys()));
  });
  return open;
}
