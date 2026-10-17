// The exports page and export definitions (share dialog, docs/contracts/export-definitions.md).
// An ordinary page marked `tine/exports:: true` (Org `#+tine/exports: true`) holds the definitions the
// share dialog writes: query blocks carrying `export:: <folder>`. Located by its marker through the query
// engine (the `tine/journal-sections` pattern), so renaming it keeps it working and no config key is
// written. Created on first use as "Exports" (a free name: "Exports 2", ...) with `public:: false` too, so
// OG Logseq's own publishing skips it, plus one overview block listing every definition. Definitions are
// appended through the document layer's single capture writer; nothing here writes a file.
import type { Format } from "./types";
import type { Query } from "./editor/queryIr";
import { backend } from "./backend";
import { bindingCurrent, captureBinding } from "./binding";
import { createPage, exportsPage } from "./document";
import { errorFamily } from "./errorFamily";
import { graphMeta } from "./graphSession";
import { graphOwner, readOwned } from "./owned";
import { QUERY_MACRO_NAMES } from "./editor/queryMacroName";

export const EXPORTS_PAGE_PROPERTY = "tine/exports";
export const DEFAULT_EXPORTS_PAGE = "Exports";
/** The destination property of a folder export (contract: `DESTINATION_KEYS`). */
export const EXPORT_PROPERTY = "export";
/** The overview query, the contract's `definitions_query()` in a query macro. */
export const EXPORTS_OVERVIEW = `{{${QUERY_MACRO_NAMES[0]} (or (property export) (property tine.share))}}`;

/** The marker pre-block in the page's own format: Org reads a Markdown `key:: value` line as body text. */
export function exportsMarker(format: Format): string {
  return format === "org"
    ? `#+${EXPORTS_PAGE_PROPERTY}: true\n#+public: false`
    : `${EXPORTS_PAGE_PROPERTY}:: true\npublic:: false`;
}

/** Names of the pages carrying the marker, from the engine's page-property index. */
export async function locateExportsPages(): Promise<string[]> {
  const owner = graphOwner();
  const parsed = await readOwned(owner, backend().parseQuery(`(page-property ${EXPORTS_PAGE_PROPERTY} true)`, "og"));
  if (parsed.kind === "stale") return [];
  const result = await readOwned(owner, backend().queryRun(parsed.value.query, parsed.value.view));
  return result.kind === "current" && result.value.anchor === "page" ? result.value.pages.map((page) => page.name) : [];
}

/** The exports page's name: the one the marker locates (first by name when several carry it), else a new
 *  page "Exports" (or "Exports 2", ...: a page of that name the user already has is never overwritten, because
 *  createPage refuses an existing file without a base revision). Throws with a reason a toast can show. */
export async function ensureExportsPage(): Promise<string> {
  const binding = captureBinding();
  const owner = graphOwner();
  const located = await locateExportsPages();
  if (!bindingCurrent(binding)) throw new Error("graph-changed");
  if (located.length) return [...located].sort((a, b) => a.localeCompare(b))[0];
  // The graph's preferred format spells the marker, as Today's sections do (createPage writes by it).
  const format: Format = graphMeta()?.preferred_format ?? "md";
  for (let n = 1; n < 100; n += 1) {
    const name = n === 1 ? DEFAULT_EXPORTS_PAGE : `${DEFAULT_EXPORTS_PAGE} ${n}`;
    const existing = await readOwned(owner, backend().getPage(name, "page"));
    if (existing.kind === "stale") throw new Error("graph-changed");
    if (existing.value) continue;
    try {
      await createPage(name, exportsPage(name, exportsMarker(format), EXPORTS_OVERVIEW, format), { bindingGeneration: binding.backendGeneration });
      return name;
    } catch (error) {
      if (bindingCurrent(binding) && errorFamily(error) === "conflict") continue;
      throw error;
    }
  }
  throw new Error("no free name for the exports page");
}

/** The "this page" selector as a query: the page row whose own name is `name` (contract: `@page and name = '…'`). */
export function pageSelectorQuery(name: string): Query {
  return {
    anchor: "page",
    filter: { kind: "leaf", leaf: { kind: "attr", attr: "name", op: "eq", value: { kind: "text", text: name } } },
    diagnostics: [],
    source: { kind: "builder" },
  };
}

/** The name a query selects when it is exactly the "this page" selector, else null. */
export function selectorPageName(query: Query): string | null {
  const filter = query.filter;
  if (query.anchor !== "page" || filter.kind !== "leaf" || filter.leaf.kind !== "attr") return null;
  const { attr, op, value } = filter.leaf;
  return attr === "name" && op === "eq" && value.kind === "text" ? value.text : null;
}

/** The selector macro for `name`, spelled by the engine's printer so it stays byte-identical when the builder
 *  reopens and saves it: `{{tine-query @page and name = 'Project'}}`. Null when the graph changed meanwhile. */
export async function pageSelectorMacro(name: string): Promise<string | null> {
  const printed = await readOwned(graphOwner(), backend().printQuery(pageSelectorQuery(name), {}, "tql_macro"));
  return printed.kind === "current" ? `{{${QUERY_MACRO_NAMES[1]} ${printed.value}}}` : null;
}
