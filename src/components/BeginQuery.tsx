import { type JSX } from "solid-js";
import type { Block as AstBlock, Format } from "../render/ast";
import { parseBody } from "../render/facets";
import { readEdn, ednSlice, unquoteEdnString } from "../editor/edn";
import { QueryMacro } from "./Macro";

export type BeginQueryMatch =
  | { kind: "supported"; query: string; title?: string }
  | { kind: "unsupported"; reason: string };

const WHOLE_BEGIN_QUERY = /^[ \t]*#\+BEGIN_QUERY[ \t]*(?:\r\n|\n|\r)([\s\S]*)(?:\r\n|\n|\r)[ \t]*#\+END_QUERY[ \t]*$/i;

function queryMap(payload: string): BeginQueryMatch {
  const source = payload.trim();
  const form = readEdn(source);
  if (!form || form.kind !== "map") return { kind: "unsupported", reason: "malformed EDN query map" };
  const bytes = new TextEncoder().encode(source);
  let title: string | undefined;
  let query: string | undefined;
  let inputs: string | undefined;
  for (let i = 0; i < form.children.length; i += 2) {
    const key = ednSlice(bytes, form.children[i]);
    const entry = form.children[i + 1];
    const value = ednSlice(bytes, entry);
    if (key === ":query") {
      if (query !== undefined) return { kind: "unsupported", reason: "duplicate :query entry" };
      query = value;
    } else if (key === ":inputs") {
      if (inputs !== undefined || !value.startsWith("[")) {
        return { kind: "unsupported", reason: "expected :inputs to be a vector" };
      }
      inputs = value;
    } else if (key === ":title") {
      if (title !== undefined || !value.startsWith('"')) {
        return { kind: "unsupported", reason: "expected :title to be a string" };
      }
      title = unquoteEdnString(value.slice(1, -1));
    }
  }

  if (!query?.startsWith("[") || !/:find\b/.test(query) || !/:where\b/.test(query)) {
    return { kind: "unsupported", reason: "expected an advanced :query vector" };
  }
  // The query vector plus its positional inputs is one execution source
  // (master 0eed673, #301): `:inputs [:current-page]` binds the host page.
  return { kind: "supported", query: `${query}${inputs ? ` :inputs ${inputs}` : ""}`, title };
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
  return queryMap(container[1]);
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
