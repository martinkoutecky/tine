/** GH #619 item 9 ("Pages and blocks"): the OTHER anchor's reading of one query's conditions.
 *
 *  One place computes it for both the block's own results (Macro.tsx) and the sheet's live preview
 *  (QueryLivePreview.tsx), so the two cannot disagree about what "both families" means. The same
 *  conditions are re-read under the other anchor by the ENGINE exactly as the sheet's anchor switch
 *  does (print, then parse): a leaf that does not apply there comes back as a diagnostic, never as a
 *  frontend guess (§7.4, D-14). The IR's anchor is never flipped directly.
 *
 *  Truncation is per family. The engine never truncates silently for size (an over-bound family
 *  rejects the whole run as `result-too-large`); what it does drop is rows a `sample` removed, and it
 *  reports the pre-sample count as `matched_total`. A family is "cut off" when that count exceeds the
 *  rows it returned, compared BEFORE the host block is taken out of the shown rows. */
import { backend } from "./backend";
import { readOwned, type Owner } from "./owned";
import { sharedQueryResult } from "./queryResultCache";
import { withoutHostBlock, resultTruncated, type BothFamilies } from "./components/queryMacroSupport";
import type { Anchor, Diagnostic, ExecutionContext, Query, QueryResult, ViewSettings } from "./editor/queryIr";
import type { RefGroup } from "./types";

const note = (diagnostics: Diagnostic[] | undefined): string | null => {
  const live = (diagnostics ?? []).filter((d) => !d.disabled);
  return live.length > 0 ? live.map((d) => d.message).join(" · ") : null;
};

/** The query's two families given its own run (`own`). `undefined` when the owner went stale. */
export async function bothFamilies(args: {
  owner: Owner;
  scope: string;
  /** Distinguishes this caller's cache entries; the caller's request key goes after it. */
  key: string;
  query: Query;
  view: ViewSettings;
  context?: ExecutionContext;
  own: QueryResult;
  hostBlockId?: string;
  hostProperties?: [string, string][];
}): Promise<{ both: BothFamilies; blockGroups: RefGroup[] } | undefined> {
  const { owner, scope, key, query, view, context, own } = args;
  const other: Anchor = query.anchor === "page" ? "block" : "page";
  const printed = await readOwned(owner, backend().printQuery({ ...query, anchor: other }, view, "tql"));
  if (printed.kind === "stale") return undefined;
  const reread = await readOwned(owner, sharedQueryResult(scope, `ir-twin-parse\0${key}`, () =>
    backend().parseQuery(printed.value, "tql", args.hostProperties)));
  if (reread.kind === "stale") return undefined;
  const twin = await readOwned(owner, sharedQueryResult(scope, `ir-twin\0${key}`, () =>
    backend().queryRun(reread.value.query, reread.value.view, context)));
  if (twin.kind === "stale") return undefined;
  const pagesResult = own.anchor === "page" ? own : twin.value;
  const blocksResult = own.anchor === "block" ? own : twin.value;
  const pages = pagesResult.anchor === "page" ? pagesResult.pages : [];
  const blockGroups = blocksResult.anchor === "block" ? withoutHostBlock(blocksResult.groups, args.hostBlockId) : [];
  return {
    blockGroups,
    both: {
      pages,
      pageTotal: pagesResult.anchor === "page" ? pagesResult.matched_total ?? pages.length : 0,
      pageNote: note(pagesResult.diagnostics),
      blockNote: note(blocksResult.diagnostics),
      pageMore: resultTruncated(pagesResult),
      blockMore: resultTruncated(blocksResult),
      ownAnchor: own.anchor,
    },
  };
}
