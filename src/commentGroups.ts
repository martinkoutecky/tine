// Presentation only: groups never change a thread's persisted parent or selector.
import { anchorQuote, quoteSelectorOf, type QuoteSelector } from "./comments";
import { node, threadRootOf } from "./document";
import type { Format } from "./types";
import { facetsOf } from "./render/facets";
import { orderedCommentIds } from "./margin";
import { threadResolved } from "./commentResolution";

export function passageKey(raw: string, selector: QuoteSelector, format: Format): string | null {
  const anchor = anchorQuote(raw, selector, format);
  return anchor.kind === "range" ? `${anchor.start}:${anchor.end}` : anchor.kind === "whole" ? "whole" : null;
}

export function commentGroups(parentId: string, format: Format, showResolved = false): string[][] {
  const parent = node(parentId);
  if (!parent) return [];
  const groups = new Map<string, string[]>();
  for (const id of orderedCommentIds(parentId, format)) {
    if (!showResolved && threadResolved(id)) continue;
    const selector = quoteSelectorOf(facetsOf(node(id)!.raw, format).properties)!;
    // Ambiguous and stale selectors have no resolved range: keep them separate.
    const key = passageKey(parent.raw, selector, format) ?? `unresolved:${id}`;
    const group = groups.get(key);
    if (group) group.push(id); else groups.set(key, [id]);
  }
  return [...groups.values()];
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
