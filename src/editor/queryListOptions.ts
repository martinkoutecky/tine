import { ednSlice, readEdn } from "./edn";

export interface QueryListOptions { breadcrumb?: boolean; groupByPage?: boolean; table?: boolean }

/** Presentation consumes the Rust EDN reader's direct map entries (including
 * discard-aware spans). It never scans query text for option-like strings. */
export function queryListOptions(source: string): QueryListOptions {
  const form = readEdn(source);
  const options: QueryListOptions = {};
  if (form?.kind !== "map") return options;
  const bytes = new TextEncoder().encode(source);
  for (let i = 0; i < form.children.length; i += 2) {
    const key = ednSlice(bytes, form.children[i]);
    const value = ednSlice(bytes, form.children[i + 1]);
    // OG treats nil as absent for breadcrumb-show?, but as false for grouping.
    if (key === ":breadcrumb-show?") options.breadcrumb = value !== "false";
    if (key === ":group-by-page?") options.groupByPage = value !== "false" && value !== "nil";
    if (key === ":table-view?") options.table = value !== "false" && value !== "nil";
  }
  return options;
}
