import { t } from "../i18n";
import { type JSX } from "solid-js";
import type { Block as AstBlock, Format } from "../render/ast";
import { parseBody } from "../render/facets";
import { query_edn_json } from "../render/wasm/lsdoc_wasm.js";
import { QueryMacro } from "./Macro";
import { queryListOptions, type QueryListOptions } from "../editor/queryListOptions";

export type BeginQueryMatch =
  | { kind: "supported"; query: string; title?: string; keeps_block_children?: boolean; listOptions?: QueryListOptions }
  | { kind: "unsupported"; reason: string };

// Mirrored by `whole_begin_query_payload` (tine-graph-features render.rs); both read
// tests/fixtures/i12-begin-query-container-golden.json. The payload is lazy so a
// CRLF closing delimiter does not leave a stray CR in it, and one final line
// ending after the closing delimiter is tolerated, as on the native side.
const WHOLE_BEGIN_QUERY = /^[ \t]*#\+BEGIN_QUERY[ \t]*(?:\r\n|\n|\r)([\s\S]*?)(?:\r\n|\n|\r)[ \t]*#\+END_QUERY[ \t]*(?:\r\n|\n|\r)?$/i;

/** The EDN payload when `raw` is exactly one terminated BEGIN_QUERY container
 *  (the same regexp `inspectBeginQuery` runs; exported for the shared golden). */
export function wholeBeginQueryPayload(raw: string): string | null {
  return WHOLE_BEGIN_QUERY.exec(raw)?.[1] ?? null;
}

/** Match only a parser-confirmed, terminated custom/query that owns the whole block.
 * OG dispatches this exact markup node to its custom-query component rather than
 * recursively painting the payload (og/src/main/frontend/components/block.cljs:3278-3284).
 * The payload is sliced from authored raw text; AST text is never used to rebuild EDN. */
export function inspectBeginQuery(
  raw: string,
  format: Format,
  parsed?: AstBlock[],
): BeginQueryMatch | null {
  const container = WHOLE_BEGIN_QUERY.exec(raw);
  if (!container) return null;
  const blocks = parsed ?? parseBody(raw, format);
  const body = blocks.filter((block, index) => {
    if (index === 0 && (block.kind === "bullet" || block.kind === "heading")) return false;
    return true;
  });
  if (body.length !== 1 || body[0].kind !== "custom" || body[0].name.toLowerCase() !== "query") {
    return { kind: "unsupported", reason: "container was not recognized as a query" };
  }
  const match = JSON.parse(query_edn_json(container[1], "begin_query", "")) as BeginQueryMatch;
  const options = queryListOptions(container[1]);
  return match.kind === "supported" && Object.keys(options).length ? { ...match, listOptions: options } : match;
}

/** Forward authored presentation alongside the engine's child-row option.
 * Tine intentionally retains its existing table default; explicit false allows
 * OG's list options to apply (GH #881; query.cljs/custom-query-inner). */
function beginQueryOptions(match: { keeps_block_children?: boolean; listOptions?: QueryListOptions }): string {
  const options = match.listOptions;
  return `{${[
    `:table-view? ${options?.table !== false}`,
    ...(match.keeps_block_children ? [":remove-block-children? false"] : []),
    ...(options?.breadcrumb !== undefined ? [`:breadcrumb-show? ${options.breadcrumb}`] : []),
    ...(options?.groupByPage !== undefined ? [`:group-by-page? ${options.groupByPage}`] : []),
  ].join(" ")}}`;
}

/** Read-only BEGIN_QUERY presentation. OG presents the authored title and query
 * result table in its normal custom-query shell (og/src/main/frontend/components/query.cljs:184-247).
 * Tine reuses QueryMacro so execution retains the existing native result bounds. */
export function BeginQuery(props: { match: BeginQueryMatch; currentPage?: string; hideWhenEmpty?: boolean; findHost?: string }): JSX.Element {
  if (props.match.kind === "unsupported") {
    return (
      <div class="query-unsupported begin-query-unsupported" role="alert">{t("begin-query-unsupported", { reason: props.match.reason })}
      </div>
    );
  }
  return (
    <QueryMacro
      body={`query ${props.match.query} ${beginQueryOptions(props.match)}`}
      title={props.match.title}
      currentPage={props.currentPage}
      findHost={props.findHost}
      hideWhenEmpty={props.hideWhenEmpty}
      strictAdvanced
      unsupportedLabel="Unsupported BEGIN_QUERY"
    />
  );
}
