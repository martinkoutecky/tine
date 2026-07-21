import { type JSX } from "solid-js";
import type { Block as AstBlock, Format } from "../render/ast";
import { parseBody } from "../render/facets";
import { unquoteEdnString } from "../editor/edn";
import { QueryMacro } from "./Macro";

export type BeginQueryMatch =
  | { kind: "supported"; query: string; title?: string }
  | { kind: "unsupported"; reason: string };

const WHOLE_BEGIN_QUERY = /^[ \t]*#\+BEGIN_QUERY[ \t]*(?:\r\n|\n|\r)([\s\S]*)(?:\r\n|\n|\r)[ \t]*#\+END_QUERY[ \t]*$/i;

function skipTrivia(source: string, from: number): number {
  let i = from;
  while (i < source.length) {
    if (/\s|,/.test(source[i])) {
      i += 1;
      continue;
    }
    if (source[i] === ";") {
      while (i < source.length && source[i] !== "\n" && source[i] !== "\r") i += 1;
      continue;
    }
    break;
  }
  return i;
}

function stringEnd(source: string, from: number): number | null {
  let i = from + 1;
  while (i < source.length) {
    if (source[i] === "\\") {
      i += 2;
      continue;
    }
    if (source[i] === '"') return i + 1;
    i += 1;
  }
  return null;
}

function balancedEnd(source: string, from: number): number | null {
  const pairs: Record<string, string> = { "(": ")", "[": "]", "{": "}" };
  if (!(source[from] in pairs)) return null;
  const stack = [pairs[source[from]]];
  let i = from + 1;
  while (i < source.length) {
    const c = source[i];
    if (c === '"') {
      const end = stringEnd(source, i);
      if (end === null) return null;
      i = end;
      continue;
    }
    if (c === ";") {
      while (i < source.length && source[i] !== "\n" && source[i] !== "\r") i += 1;
      continue;
    }
    if (c in pairs) stack.push(pairs[c]);
    else if (c === ")" || c === "]" || c === "}") {
      if (stack.pop() !== c) return null;
      if (stack.length === 0) return i + 1;
    }
    i += 1;
  }
  return null;
}

function tokenEnd(source: string, from: number): number {
  let i = from;
  while (i < source.length && !/[\s,()[\]{}]/.test(source[i])) i += 1;
  return i;
}

function valueEnd(source: string, from: number): number | null {
  if (source[from] === '"') return stringEnd(source, from);
  if (source[from] === "(" || source[from] === "[" || source[from] === "{") {
    return balancedEnd(source, from);
  }
  const end = tokenEnd(source, from);
  return end > from ? end : null;
}

function queryMap(payload: string): BeginQueryMatch {
  const source = payload.trim();
  if (!source.startsWith("{")) return { kind: "unsupported", reason: "expected an EDN query map" };
  const mapEnd = balancedEnd(source, 0);
  if (mapEnd === null || skipTrivia(source, mapEnd) !== source.length) {
    return { kind: "unsupported", reason: "malformed EDN query map" };
  }

  let title: string | undefined;
  let query: string | undefined;
  let i = 1;
  while (i < mapEnd - 1) {
    i = skipTrivia(source, i);
    if (i >= mapEnd - 1) break;
    if (source[i] !== ":") return { kind: "unsupported", reason: "malformed EDN query map" };
    const keyEnd = tokenEnd(source, i);
    const key = source.slice(i, keyEnd);
    i = skipTrivia(source, keyEnd);
    const end = valueEnd(source, i);
    if (end === null || end > mapEnd - 1) {
      return { kind: "unsupported", reason: `missing value for ${key}` };
    }
    const value = source.slice(i, end);
    if (key === ":query") {
      if (query !== undefined) return { kind: "unsupported", reason: "duplicate :query entry" };
      query = value;
    } else if (key === ":title") {
      if (title !== undefined || !value.startsWith('"')) {
        return { kind: "unsupported", reason: "expected :title to be a string" };
      }
      title = unquoteEdnString(value.slice(1, -1));
    }
    i = end;
  }

  if (!query?.startsWith("[") || !/:find\b/.test(query) || !/:where\b/.test(query)) {
    return { kind: "unsupported", reason: "expected an advanced :query vector" };
  }
  return { kind: "supported", query, title };
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
