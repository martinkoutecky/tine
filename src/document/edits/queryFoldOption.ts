import { untrack } from "solid-js";
import { graphScopedSignal } from "../../binding";
import { blockRegions, parserReady } from "../../render/parse";
import { doc, formatForBlock } from "../model";

// GH #619 Q8: a query block has ONE fold state, the bullet's `collapsed::`, and
// two controls on it (the bullet and the query's own arrow). The one exception
// is the query's authored `:collapsed? true` default: it folds a query whose
// block carries no `collapsed::` property of its own. The query macro, which
// alone parses its options (I-12), reports that default here so the bullet, the
// arrow and the document edits read one answer.
// Graph-scoped (I-20): the reports end with the graph, as the macros do.
const [reports, setReports] = graphScopedSignal<Readonly<Record<string, number>>>();

/** Report that the query hosted by block `id` is authored `:collapsed? true`.
 *  Counted, because one block can be rendered by several surfaces. Returns the release. */
export function reportQueryFoldOption(id: string): () => void {
  const bump = (by: number) => {
    const { [id]: current = 0, ...rest } = untrack(reports) ?? {};
    const next = current + by;
    setReports(next > 0 ? { ...rest, [id]: next } : rest);
  };
  bump(1);
  return () => bump(-1);
}

/** Whether the block's query is authored `:collapsed? true` (reported by its macro). */
export function queryOptionFolds(id: string): boolean {
  return (reports()?.[id] ?? 0) > 0;
}

/** Whether the block's own source carries a `collapsed::` property, true or false. */
export function hasExplicitCollapsed(id: string): boolean {
  const n = doc.byId[id];
  if (!n || !parserReady() || !n.raw.toLowerCase().includes("collapsed")) return false;
  return blockRegions(n.raw, formatForBlock(id)).properties.some((p) => p.primary && p.key.toLowerCase() === "collapsed");
}

/** The block's stored fold: `collapsed::`, else the query's authored default. */
export function storedFold(id: string): boolean {
  const n = doc.byId[id];
  // The text test keeps an ordinary block from subscribing to the reports.
  return !!n && (n.collapsed || (n.raw.includes("collapsed") && queryOptionFolds(id) && !hasExplicitCollapsed(id)));
}
