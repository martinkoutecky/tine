import type { PageKind } from "./types";
import type { FriendlyPageMatchScope, ViewKind } from "./editor/queryIr";
import type { QueryDisplayDraft } from "./editor/queryDisplayDraft";

export interface PageTarget {
  name: string;
  pageKind: PageKind;
  path?: string;
}

export type QueryPresentation = ViewKind;

export interface QueryRoute {
  kind: "query";
  id: string;
  sourceKind: "search" | "dsl";
  source: string;
  presentation: QueryPresentation;
  /** Each family inherits `presentation` until its own choice is set. */
  pagePresentation?: QueryPresentation;
  blockPresentation?: QueryPresentation;
  /** A present draft, including `{}`, replaces that family's inherited display. */
  pageDisplay?: QueryDisplayDraft;
  blockDisplay?: QueryDisplayDraft;
  /** Omitted means the historical names/aliases-only page membership. */
  pageMatchScope?: FriendlyPageMatchScope;
}

/** One reader tab. viewId identifies the tab's view, while filename identifies
 * the asset; page and scale are saved per view in the graph session. */
export interface PdfRoute {
  kind: "pdf";
  viewId: string;
  filename: string;
  label: string;
  page?: number;
  scale?: number;
}

/** A bad saved PDF tab stays closable without discarding its containing pane. */
export interface InvalidRoute {
  kind: "invalid";
  title: string;
  message: string;
}

export type Route =
  | { kind: "journals" }
  /** The Concord overview: a view of the derived conflict queue, never a file. */
  | { kind: "conflicts" }
  | QueryRoute
  | PdfRoute
  | InvalidRoute
  | (PageTarget & {
    kind: "page";
    block?: string;
    path?: string;
    /** Sibling-index path to `block`, saved in a session for an ID-less block (navigation
     * never writes an `id::`); resolved by position on restore, then settled into `block`. */
    blockPos?: number[];
  });

/** A persisted sibling-index path (`blockPos`), or null when malformed. Bounded so
 * a hostile or corrupt session cannot make a restore walk an enormous path. */
export function parseBlockPos(raw: unknown): number[] | null {
  if (!Array.isArray(raw) || raw.length === 0 || raw.length > 256) return null;
  return raw.every((n) => Number.isSafeInteger(n) && n >= 0 && n <= 1_000_000) ? raw.slice() as number[] : null;
}
