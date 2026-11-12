// The "N below" count of a folded block (vision §3.7; GH #891 review rounds 2
// and 3, PC-1/PC-3/PC-4/PC-5): a reading of the covering comment index
// (`commentIndex.ts`), which already follows structure and comment membership.
// It costs O(roots × depth) when a root or the structure changes, nothing on a
// comment-free page, and nothing while typing in an ordinary block.

import { createMemo, type Accessor } from "solid-js";
import { createCommentIndexFor } from "./commentIndex";
import { useSeenThread } from "./seen/SeenHeader";

/** Open thread roots hidden below `id`'s direct children, counted once per
 * thread (never its replies); 0 when `active` is false or `id` is itself in a
 * thread. Same answer as the imperative `descendantOpenThreads`. */
export function createFoldedThreadCount(id: Accessor<string>, active: Accessor<boolean>): Accessor<number> {
  const index = createCommentIndexFor(() => (active() ? id() : null));
  return createMemo(() => index()?.openBelow(id()) ?? 0);
}

/** How many of those threads hold a message changed since the user last looked (ADR 0079): the
 * same roots as the count, read against the routed page's existing seen marks. */
export function createFoldedChangedThreadCount(id: Accessor<string>, active: Accessor<boolean>): Accessor<number> {
  const index = createCommentIndexFor(() => (active() ? id() : null));
  const changedThread = useSeenThread();
  return createMemo(() => (index()?.openRootsBelow(id()) ?? []).filter(changedThread).length);
}
