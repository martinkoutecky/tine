// The ONE frontend answer to "is this a query macro, and where does it sit in the raw source" (SPEC §4.3.1, …

import { QUERY_MACRO_NAMES } from "./queryMacroName";
export { QUERY_MACRO_SCAFFOLD } from "./queryMacroName";

export { QUERY_MACRO_NAMES };

/** Which grammar's literals protect a delimiter while scanning FORM text. */
export type FormFamily = "edn" | "tql";

/** The family a macro NAME implies: `query` carries OG or advanced text, `tine-query` carries TQL (§7.1). */
export function formFamilyForMacroName(name: string): FormFamily {
  return name.toLowerCase() === "tine-query" ? "tql" : "edn";
}

/** Whether `name` is one of the query macro names, case-insensitively and as a
*  WHOLE token — `{{query-foo}}` is not a query (§7.9). */
export function isQueryMacroName(name: string): boolean {
  const lower = name.toLowerCase();
  return QUERY_MACRO_NAMES.some((candidate) => candidate === lower);
}

/** One brace the scan found outside every literal, comment and page ref. */
interface Brace {
  at: number;
  open: boolean;
  /** Nesting depth AFTER this brace, counting from `formDepth`. */
  depth: number;
}

// Index just past an EDN double-quoted string opening at `at`; end of input if unterminated.
function ednStringEnd(text: string, at: number): number {
  let j = at + 1;
  while (j < text.length) {
    if (text[j] === "\\") j += 2;
    else if (text[j] === '"') return j + 1;
    else j += 1;
  }
  return text.length;
}

// Index just past a TQL single-quoted string opening at `at`; end of input if unterminated.
function tqlStringEnd(text: string, at: number): number {
  let j = at + 1;
  while (j < text.length) {
    if (text[j] === "'") {
      if (text[j + 1] === "'") {
        j += 2;
        continue;
      }
      return j + 1;
    }
    j += 1;
  }
  return text.length;
}

// Index just past a `[[page ref]]` opening at `at`; end of input if unterminated.
function pageRefEnd(text: string, at: number): number {
  const close = text.indexOf("]]", at + 2);
  return close === -1 ? text.length : close + 2;
}

/** **The one scan.** Walk `text` once and report every `{` / `}` that is not
*  inside a protected region, with the depth it produces.
*
*  `formDepth` is the depth at which the form text sits: 0 when scanning a macro
*  ARGUMENT (the splitter), 2 when scanning from inside `{{` (the extent
*  reader). While the depth is at `formDepth` the `family` decides which literals
*  protect a brace; deeper than that we are inside an options map and EDN rules
*  apply. An unterminated literal consumes to end of input rather than
*  resynchronising — that is what makes an unbalanced `}` inside a literal
*  invisible to the split. Transcribes `macro_text::scan_braces`. */
function scanBraces(text: string, family: FormFamily, formDepth: number): Brace[] {
  const out: Brace[] = [];
  let depth = formDepth;
  let i = 0;
  while (i < text.length) {
    // Inside a map the text is EDN whatever the form was: an EDN symbol's apostrophe (`'foo`, `#'x`) is never a …
    const edn = depth > formDepth || family === "edn";
    const c = text[i];
    if (c === '"' && edn) {
      i = ednStringEnd(text, i);
      continue;
    }
    if (c === "'" && !edn) {
      i = tqlStringEnd(text, i);
      continue;
    }
    if (c === ";" && edn) {
      while (i < text.length && text[i] !== "\n") i += 1;
      continue;
    }
    if (c === "[" && text.startsWith("[[", i)) {
      i = pageRefEnd(text, i);
      continue;
    }
    if (c === "{") {
      depth += 1;
      out.push({ at: i, open: true, depth });
    } else if (c === "}") {
      depth -= 1;
      out.push({ at: i, open: false, depth });
    }
    i += 1;
  }
  return out;
}

/** One query macro as it sits in the ORIGINAL raw source. */
export interface MacroExtent {
  /** Index of the opening `{{`. */
  start: number;
  /** Index just past the closing `}}`. */
  end: number;
  name: string;
  argument: string;
}

/** Read one macro whose `{{` is at `start`, if its name is a query macro name. */
function macroAt(raw: string, start: number): MacroExtent | null {
  const rest = raw.slice(start + 2);
  let name: string | null = null;
  for (const candidate of QUERY_MACRO_NAMES) {
    if (rest.length < candidate.length) continue;
    if (rest.slice(0, candidate.length).toLowerCase() !== candidate) continue;
    const after = rest[candidate.length];
    if (after !== undefined && after !== " " && after !== "\t" && after !== "}") continue;
    if (name === null || candidate.length > name.length) name = candidate;
  }
  if (name === null) return null;
  const argumentStart = start + 2 + name.length;
  const family = formFamilyForMacroName(name);
  // Depth 2 is what the two opening braces already contributed, so form text sits at depth 2 and a `{` of the …
  const braces = scanBraces(raw.slice(argumentStart), family, 2);
  const close = braces.find((brace) => !brace.open && brace.depth === 0);
  if (!close) return null; // unterminated
  const end = argumentStart + close.at + 1;
  // Everything between the name and the LAST closing brace is the argument; one leading space is the macro's …
  const argument = raw.slice(argumentStart, end - 2);
  return {
    start,
    end,
    name,
    argument: argument.startsWith(" ") ? argument.slice(1) : argument,
  };
}

/** The first query macro in `raw`, or null. */
export function queryMacroExtent(raw: string): MacroExtent | null {
  return queryMacroExtentFrom(raw, 0);
}

/** Every query macro in `raw`, in source order. */
export function queryMacroExtents(raw: string): MacroExtent[] {
  const out: MacroExtent[] = [];
  let from = 0;
  while (from < raw.length) {
    const found = queryMacroExtentFrom(raw, from);
    if (!found) break;
    from = found.end;
    out.push(found);
  }
  return out;
}

/** Recover the sole authored macro from an entire-block render. A property
 *  line or leading whitespace can move its raw offset; refuse ambiguity. */
export function singleQueryMacroExtent(raw: string, displayed: MacroExtent): MacroExtent | undefined {
  const extents = queryMacroExtents(raw);
  const extent = extents.length === 1 ? extents[0] : undefined;
  return extent?.name === displayed.name && extent.argument === displayed.argument ? extent : undefined;
}

function queryMacroExtentFrom(raw: string, from: number): MacroExtent | null {
  let search = from;
  for (;;) {
    const at = raw.indexOf("{{", search);
    if (at === -1) return null;
    const found = macroAt(raw, at);
    if (found) return found;
    search = at + 2;
  }
}

const UTF8_ENCODER = new TextEncoder();

/** The extent a parsed macro node's SPAN points at, or null. */
export function queryMacroExtentAtSpan(
  raw: string,
  span: readonly [number, number] | undefined,
): MacroExtent | null {
  if (span === undefined || span[0] < 2) return null;
  const trimmed = raw.trimStart();
  const leadBytes = UTF8_ENCODER.encode(raw.slice(0, raw.length - trimmed.length)).length;
  const wanted = span[0] - 2 + leadBytes;
  for (const extent of queryMacroExtents(raw)) {
    if (UTF8_ENCODER.encode(raw.slice(0, extent.start)).length === wanted) return extent;
  }
  return null;
}
