// Presentation only: groups never change a thread's persisted parent or selector.
import { anchorQuote, type QuoteSelector } from "./comments";
import { node, threadRootOf } from "./document";
import type { Format } from "./types";
import { orderCommentEntries } from "./margin";
import { threadResolved } from "./commentResolution";
import { commentChildOf, type CommentChild } from "./commentChildren";

export function passageKey(raw: string, selector: QuoteSelector, format: Format): string | null {
  const anchor = anchorQuote(raw, selector, format);
  return anchor.kind === "range" ? `${anchor.start}:${anchor.end}` : anchor.kind === "whole" ? "whole" : null;
}

/** Same-passage groups in passage order. `entries` are the comment roots to
 * present, in file order (resolved ones already filtered by the caller). */
export function groupCommentEntries(parentRaw: string, entries: readonly CommentChild[], format: Format): string[][] {
  const byId = new Map(entries.map(entry => [entry.id, entry]));
  const groups = new Map<string, string[]>();
  for (const id of orderCommentEntries(parentRaw, entries, format)) {
    // Ambiguous and stale selectors have no resolved range: keep them separate.
    const key = passageKey(parentRaw, byId.get(id)!.selector, format) ?? `unresolved:${id}`;
    const group = groups.get(key);
    if (group) group.push(id); else groups.set(key, [id]);
  }
  return [...groups.values()];
}

/** The one surface order (review finding 2, AR-1): presentation draws these
 * groups in this order and keyboard navigation walks the same flat list. */
export function threadOrder(groups: readonly (readonly string[])[]): string[] {
  return groups.flat();
}

/** Imperative form for actions: scans the children once. */
export function commentGroups(parentId: string, format: Format, showResolved = false): string[][] {
  const parent = node(parentId);
  if (!parent) return [];
  const entries = parent.children.flatMap(id => {
    const child = node(id);
    const entry = child && commentChildOf(id, child.raw, format);
    return entry && (showResolved || !entry.resolved) ? [entry] : [];
  });
  return groupCommentEntries(parent.raw, entries, format);
}

/** Hidden descendant body blocks only. A quoted reply remains one message in
 * its outermost thread and is never counted as another thread. */
export function descendantOpenThreads(parentId: string, format: Format): number {
  let count = 0;
  const visit = (id: string) => {
    const n = node(id);
    if (!n) return;
    const root = threadRootOf(id, format);
    if (root !== null) {
      if (root === id && !threadResolved(id)) count++;
      return;
    }
    for (const child of n.children) visit(child);
  };
  for (const id of node(parentId)?.children ?? []) if (threadRootOf(id, format) === null) visit(id);
  return count;
}
