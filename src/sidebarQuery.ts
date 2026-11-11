// The "query" right-sidebar item kind (DESIGN §12), split out of ui.ts (size ratchet): its
// shape, identity key, equality and restore validation. Pure; no signals, no storage.

/** The open-comment-threads review: an unsaved query the sidebar runs without a block, scoped
 * to one page or the whole graph. It stores only the scope and the author filter; the query text
 * is derived (`commentReviewQuery.ts`), so nothing in the graph changes. */
export interface SidebarQuery {
  kind: "query";
  scope: "page" | "graph";
  /** The page of a page-scoped review; "" for the graph. */
  page: string;
  pageKind: "journal" | "page";
  /** Exact graph-relative owner of the page. Absent on the graph scope. */
  path?: string;
  /** Only threads written by this author (`tine.author`). Absent: every author. */
  author?: string;
  collapsed?: boolean;
}

export const sidebarQueryKey = (item: SidebarQuery): string =>
  item.scope === "graph" ? "query:graph" : `query:${item.pageKind}:${item.page}`;

export const sameSidebarQuery = (a: SidebarQuery, b: SidebarQuery): boolean =>
  a.scope === b.scope && a.page === b.page && a.pageKind === b.pageKind && a.path === b.path && a.author === b.author;

/** Restore check. A kind this build does not know (written by a newer one) never reaches here:
 * the caller drops it, so a restore never throws or renders a half-understood item. */
export function validSidebarQuery(o: Record<string, unknown>): boolean {
  return (o.scope === "graph" || o.scope === "page") && typeof o.page === "string"
    && (o.pageKind === "journal" || o.pageKind === "page")
    && (o.scope === "graph" || o.page !== "")
    && (o.author === undefined || (typeof o.author === "string" && o.author !== ""));
}
