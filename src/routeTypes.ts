import type { PageKind } from "./types";

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
}

export type Route =
  | { kind: "journals" }
  | QueryRoute
  | (PageTarget & { kind: "page"; block?: string; path?: string });
