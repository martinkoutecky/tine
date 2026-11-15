import type { Block as AstBlock, Format } from "./ast";
import { parseBody } from "./facets";
import { query_edn_json } from "./wasm/lsdoc_wasm.js";
import { queryListOptions, type QueryListOptions } from "../editor/queryListOptions";

// The BEGIN_QUERY recognizer, free of components so non-rendering callers (the find-in-page host
// inventory) can ask it without loading the renderer; components/BeginQuery.tsx renders its answer.
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
