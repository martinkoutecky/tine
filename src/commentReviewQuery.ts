// Open-threads review (DESIGN §12, D-20261107-MC3): "which margin comments still need an
// answer?" is a QUERY in the right sidebar, not a comments panel. This file owns the three
// things that are specific to it: the query text, how a review item opens, and its author
// filter. The sidebar item (`SidebarQuery`, ui.ts) stores only scope + author; the text is
// derived here, runs through the existing query engine (the indexed property filters), and
// its results are the ordinary live, editable result blocks. No comment index of its own,
// no graph write, no persisted record beyond the sidebar item.

import { rightSidebar, setRightSidebar, setRightSidebarOpen, sidebarItemKey, setRightSidebarFocusRequest, type SidebarQuery } from "./ui";
import { pageTargetFromRoute, route } from "./router";
import type { PageKind } from "./types";

const quote = (text: string): string => `"${text.replace(/\\/g, "\\\\").replace(/"/g, "\\\"")}"`;

/** The simple-query text of the review. A thread root carries `tine.quote`; resolving it adds
 * `tine.resolved:: true` (the roots-only rule the "N below" count uses). Page scope adds
 * `(page …)`; an author adds `(property tine.author …)`. All three are indexed property/page
 * filters of the existing engine, so the cost is the query's, not a page walk. */
export function openThreadsQuery(item: Pick<SidebarQuery, "scope" | "page" | "author">): string {
  const clauses = ["(property tine.quote)", "(not (property tine.resolved true))"];
  if (item.scope === "page") clauses.push(`(page ${quote(item.page)})`);
  if (item.author) clauses.push(`(property tine.author ${quote(item.author)})`);
  return `(and ${clauses.join(" ")})`;
}

/** The macro body the sidebar hands to the query renderer. */
export const openThreadsMacroBody = (item: Pick<SidebarQuery, "scope" | "page" | "author">): string =>
  `query ${openThreadsQuery(item)}`;

/** Open (or bring to the top, expanded) the review for `scope`. A scope has one slot, so
 * reopening keeps the author filter the user set. `page` is the current page for page scope;
 * with none (journals feed, a query route) the review falls back to the graph. */
export function openCommentReviewInSidebar(
  scope: "page" | "graph",
  page?: { name: string; pageKind: PageKind; path?: string },
): void {
  const target = scope === "page" && page ? page : undefined;
  const item: SidebarQuery = target
    ? { kind: "query", scope: "page", page: target.name, pageKind: target.pageKind, ...(target.path ? { path: target.path } : {}) }
    : { kind: "query", scope: "graph", page: "", pageKind: "page" };
  setRightSidebarOpen(true);
  const key = sidebarItemKey(item);
  const existing = rightSidebar().find((i): i is SidebarQuery => i.kind === "query" && sidebarItemKey(i) === key);
  const opened: SidebarQuery = existing ? { ...existing, collapsed: false } : item;
  setRightSidebar([opened, ...rightSidebar().filter((i) => i !== existing)]);
  setRightSidebarFocusRequest({ key });
}

/** The focused pane's page, if it shows one. */
export function openCommentReviewForFocusedPage(): void {
  const target = pageTargetFromRoute(route());
  openCommentReviewInSidebar(target ? "page" : "graph", target ?? undefined);
}

/** Set or clear (empty / whitespace) a review item's author filter, in place. */
export function setCommentReviewAuthor(item: SidebarQuery, author: string): void {
  const trimmed = author.trim();
  if ((item.author ?? "") === trimmed) return;
  const { author: _drop, ...rest } = item;
  const next: SidebarQuery = trimmed ? { ...rest, author: trimmed } : rest;
  setRightSidebar(rightSidebar().map((i) => (i === item ? next : i)));
}
