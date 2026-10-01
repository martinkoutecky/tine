import { type JSX } from "solid-js";
import type { Block as AstBlock, Format } from "../render/ast";
import { parseBody } from "../render/facets";
import { query_edn_json } from "../render/wasm/lsdoc_wasm.js";
import { QueryMacro } from "./Macro";

export type BeginQueryMatch =
  | { kind: "supported"; query: string; title?: string }
  | { kind: "unsupported"; reason: string };

const WHOLE_BEGIN_QUERY = /^[ \t]*#\+BEGIN_QUERY[ \t]*(?:\r\n|\n|\r)([\s\S]*)(?:\r\n|\n|\r)[ \t]*#\+END_QUERY[ \t]*$/i;

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
  return JSON.parse(query_edn_json(container[1], "begin_query", "")) as BeginQueryMatch;
}

/** Read-only BEGIN_QUERY presentation. OG presents the authored title and query
 * result table in its normal custom-query shell (og/src/main/frontend/components/query.cljs:184-247).
 * Tine reuses QueryMacro so execution retains the existing native result bounds. */
export function BeginQuery(props: { match: BeginQueryMatch; currentPage?: string }): JSX.Element {
  if (props.match.kind === "unsupported") {
    return (
      <div class="query-unsupported begin-query-unsupported" role="alert">
        Unsupported BEGIN_QUERY: {props.match.reason}.
      </div>
    );
  }
  return (
    <QueryMacro
      body={`query ${props.match.query} {:table-view? true}`}
      title={props.match.title}
      currentPage={props.currentPage}
      strictAdvanced
      unsupportedLabel="Unsupported BEGIN_QUERY"
    />
  );
}
