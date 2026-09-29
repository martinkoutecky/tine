import type { PageKind } from "./types";
import type { FriendlyPageMatchScope } from "./editor/queryIr";
import type { QueryDisplayDraft } from "./editor/queryDisplayDraft";

export interface PageTarget {
  name: string;
  pageKind: PageKind;
  path?: string;
}

export type QueryPresentation = "search" | "list" | "table" | "board";

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

export type Route =
  | { kind: "journals" }
  | QueryRoute
  | (PageTarget & { kind: "page"; block?: string; path?: string });
