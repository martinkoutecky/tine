// "Changed since you last looked" for a margin-comment thread (DESIGN §12, ADR 0079). The
// page's one seen read already marks any changed block, comment blocks included; a thread
// counts as changed when its root or any block under it is. No read of its own, no store.

/** Does the thread rooted at `root` hold a changed block? O(thread blocks); `changed`
 * and `childrenOf` are the reactive readers the caller passes, so a mark or an edit updates it. */
export function threadHasChange(
  root: string,
  changed: (id: string) => boolean,
  childrenOf: (id: string) => readonly string[] | undefined,
): boolean {
  const stack = [root];
  while (stack.length) {
    const id = stack.pop()!;
    if (changed(id)) return true;
    stack.push(...(childrenOf(id) ?? []));
  }
  return false;
}
