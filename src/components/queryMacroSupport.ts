// Pure support for the query macro (`Macro.tsx` re-exports what other modules import from there).
import type { Diagnostic, PageRow, QueryReport, QueryStatistics } from "../editor/queryIr";
import type { PageKind, QueryExecution, RefGroup } from "../types";
import { columnKey } from "./legacyQueryTable";

export interface Row {
  page: string;
  kind: PageKind;
  path?: string;
  text: string;
  props: Record<string, string>;
  /** `props` under OG's normalised column key (`columnKey`), first spelling wins. */
  byKey: Record<string, string>;
}

/** One cell of the legacy `:table? true` list view. */
export const cellText = (r: Row, c: string): string =>
  c === "block" ? r.text : c === "page" ? r.page : r.byKey[columnKey(c)] ?? r.props[c] ?? "";

/** A bounded excerpt of the ENGINE-PRINTED text a crossing save wrote (master
 *  `boundedFeature`, I-22). It is labelled as an excerpt, never as "the
 *  unsupported feature": nothing in the engine answers that question. */
export function boundedFeature(message: string): string | null {
  const single = message.replace(/\s+/g, " ").trim();
  if (!single) return null;
  return single.length > 120 ? `${single.slice(0, 119)}…` : single;
}

/** Remove the block a query is written in from that query's own results
 *  (GH #469; OG `query/result.cljs` "exclude the current one, otherwise it'll
 *  loop forever"). Only the block goes; its children are ordinary results. */
export function withoutHostBlock(groups: RefGroup[], hostBlockId: string | undefined): RefGroup[] {
  if (!hostBlockId) return groups;
  const hosts = (group: RefGroup) => group.blocks.some((block) => block.id === hostBlockId);
  if (!groups.some(hosts)) return groups;
  return groups
    .map((group) => (hosts(group) ? { ...group, blocks: group.blocks.filter((block) => block.id !== hostBlockId) } : group))
    .filter((group) => group.blocks.length > 0);
}

export interface QueryOperation {
  requestKey: string;
  groups: RefGroup[];
  pages: PageRow[] | null;
  diagnostics: Diagnostic[];
  report: QueryReport | null;
  statistics?: QueryStatistics;
  search: QueryExecution | null;
  matchedTotal: number | null;
  /** GH #619 item 9: "Pages and blocks" — both families of ONE query. The page
   *  family is the page-anchored reading and the block family the block-anchored
   *  one, whichever the macro's own text is. `null` for an ordinary query. */
  both: BothFamilies | null;
}
export interface BothFamilies {
  pages: PageRow[];
  pageTotal: number;
  /** Why the other anchor's reading of the same conditions did not apply. */
  pageNote: string | null;
  blockNote: string | null;
}
/** The host block property the "Pages and blocks" choice is stored in. The anchor
 *  itself lives only in the TQL text (`@page` / `@block`; an OG-form `{{query}}`
 *  infers it), so there is no OG key to reuse; OG ignores an unknown `tine.*`
 *  block property exactly as it ignores `tine.view`. */
export const RESULT_KINDS_PROPERTY = "tine.result-kinds";
export const PAGES_AND_BLOCKS = "pages-and-blocks";

