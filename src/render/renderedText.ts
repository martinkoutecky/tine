// Rendered-text flattening of a block body — what the block LOOKS like as plain
// text (typographic glyphs, entity unicode, no markup markers), for the
// Copy/Export modal's "Rendered" mode. Driven by the ONE lsdoc parse (never a
// regex re-scan of raw): parse the block, walk the AST, emit visible text.
//
// Fidelity notes (pragmatic, documented): block refs emit their label or the
// bare uuid (resolving the referenced block needs live graph state); macros stay
// in `{{...}}` form (their expansion is graph-dependent); math emits the TeX
// source (the typeset glyphs aren't text); properties honor the built-in
// render-hidden set but not the user's `:block-hidden-properties` (graph state).

import { parseBlock } from "./parse";
import { typographic } from "./typography";
import { isRenderHiddenProp } from "./block";
import type { Block, Format, Inline, ListItem, TimestampInline, TimestampPoint, Url } from "./ast";

/** The `<…>`(active) / `[…]`(inactive) display text of a timestamp inline —
 *  shared with the renderer (render/inline.tsx) so there is one formatter. */
export function timestampText(s: TimestampInline): { text: string; active: boolean } {
  const pad2 = (n: number) => String(n).padStart(2, "0");
  const point = (p: TimestampPoint): string => {
    const d = p.date;
    let out = `${d.year}-${pad2(d.month)}-${pad2(d.day)}`;
    if (p.wday) out += ` ${p.wday}`;
    if (p.time) out += ` ${pad2(p.time.hour)}:${pad2(p.time.min)}`;
    return out;
  };
  const v = s.date as Record<string, unknown>;
  let active = true;
  let inner: string;
  if (s.ts === "Range" && v.start && v.stop) {
    const start = v.start as TimestampPoint;
    active = start.active ?? true;
    inner = `${point(start)}--${point(v.stop as TimestampPoint)}`;
  } else {
    const p = v as unknown as TimestampPoint;
    active = p.active ?? true;
    inner = point(p);
  }
  return { text: active ? `<${inner}>` : `[${inner}]`, active };
}

export interface RenderedTextOptions {
  /** Apply the `->`→`→` glyph substitution (match the app's typography mode). */
  typographicGlyphs: boolean;
  stripLinks: boolean; // [[Foo]] → Foo
  removeTags: boolean; // drop #tag nodes
  removeProperties: boolean; // drop property blocks
}

function urlDest(url: Url): string {
  switch (url.type) {
    case "page_ref":
    case "block_ref":
    case "search":
    case "file":
    case "embed_data":
      return url.v;
    case "complex":
      return url.protocol && url.link != null ? `${url.protocol}://${url.link}` : url.link ?? "";
  }
}

function inlineText(nodes: Inline[], o: RenderedTextOptions): string {
  let out = "";
  for (const s of nodes) {
    switch (s.k) {
      case "plain":
        out += o.typographicGlyphs ? typographic(s.text) : s.text;
        break;
      case "code":
      case "verbatim":
      case "target":
      case "inline_html":
        out += s.text as string;
        break;
      case "break":
      case "hardbreak":
        out += "\n";
        break;
      case "emphasis":
      case "subscript":
      case "superscript":
        out += inlineText(s.children, o);
        break;
      case "tag":
        if (!o.removeTags) out += `#${inlineText(s.children, o)}`;
        break;
      case "link": {
        const label = s.label && s.label.length ? inlineText(s.label, o) : "";
        if (s.url.type === "page_ref" || s.url.type === "block_ref") {
          const name = label || s.url.v;
          // The renderer shows `[[name]]` (dimmed brackets) for a bare page ref,
          // the alias text alone when labeled; block refs show label-or-uuid.
          out += s.url.type === "page_ref" && !label && !o.stripLinks ? `[[${name}]]` : name;
        } else {
          out += label || urlDest(s.url);
        }
        break;
      }
      case "nested_link":
        out += o.stripLinks ? s.content : `[[${s.content}]]`;
        break;
      case "macro":
        out += s.args.length ? `{{${s.name} ${s.args.join(", ")}}}` : `{{${s.name}}}`;
        break;
      case "latex":
        out += s.body;
        break;
      case "timestamp":
        out += timestampText(s).text;
        break;
      case "fnref":
        out += `[${s.name}]`;
        break;
      case "email":
        out += typeof s.text === "string" ? s.text : inlineText([], o) + emailText(s.text);
        break;
      case "entity":
        out += s.unicode;
        break;
      case "hiccup":
        out += s.v;
        break;
    }
  }
  return out;
}

function emailText(v: unknown): string {
  if (v && typeof v === "object") {
    const r = v as { local_part?: string; domain?: string };
    if (r.local_part && r.domain) return `${r.local_part}@${r.domain}`;
  }
  return "";
}

function headerPrefix(b: Block): string {
  let out = "";
  if ((b.kind === "bullet" || b.kind === "heading") && b.marker) out += `${b.marker} `;
  if ((b.kind === "bullet" || b.kind === "heading") && b.priority) out += `[#${b.priority}] `;
  return out;
}

function listItemLines(item: ListItem, depth: number, o: RenderedTextOptions, index: number): string[] {
  const pad = "  ".repeat(depth);
  const bullet = item.ordered ? `${item.number ?? index + 1}. ` : "- ";
  const head = item.name && item.name.length ? `${inlineText(item.name, o)}: ` : "";
  const body = item.content.flatMap((b) => blockLines(b, o)).join("\n");
  const bodyLines = (head + body).split("\n");
  const lines = [`${pad}${bullet}${bodyLines[0] ?? ""}`];
  for (const l of bodyLines.slice(1)) lines.push(`${pad}  ${l}`);
  item.items.forEach((child, i) => lines.push(...listItemLines(child, depth + 1, o, i)));
  return lines;
}

function blockLines(b: Block, o: RenderedTextOptions): string[] {
  switch (b.kind) {
    case "paragraph":
    case "heading":
    case "bullet":
      return (headerPrefix(b) + inlineText(b.inline, o)).split("\n");
    case "src":
    case "example":
      return b.code.replace(/\n$/, "").split("\n");
    case "quote":
    case "custom":
      return b.children.flatMap((c) => blockLines(c, o));
    case "list":
      return b.items.flatMap((item, i) => listItemLines(item, 0, o, i));
    case "table": {
      const row = (cells: Inline[][]) => cells.map((c) => inlineText(c, o)).join(" | ");
      const lines: string[] = [];
      if (b.header) lines.push(row(b.header));
      for (const r of b.rows) lines.push(row(r));
      return lines;
    }
    case "properties":
      if (o.removeProperties) return [];
      return b.props.filter(([k]) => !isRenderHiddenProp(k)).map(([k, v]) => `${k} ${v}`);
    case "hr":
      return ["---"];
    case "displayed_math":
    case "raw_html":
      return b.text.split("\n");
    case "latex_env":
      return b.content.split("\n");
    case "footnote_def":
      return [`[${b.name}] ${inlineText(b.inline, o)}`];
    case "hiccup":
      return [b.v];
    case "drawer":
    case "directive":
    case "comment":
      return []; // not rendered as body text
  }
}

const INLINE_FLOW = new Set(["paragraph", "heading", "bullet"]);

/** The visible plain text of one block body (multi-line). Mirrors the renderer:
 *  consecutive inline-flow blocks are newline-joined; structural blocks stack. */
export function renderedBlockText(raw: string, format: Format, o: RenderedTextOptions): string {
  const blocks = parseBlock(raw, format === "org");
  const lines: string[] = [];
  for (const b of blocks) {
    const ls = blockLines(b, o);
    if (ls.length === 0) continue;
    // Mirror the renderer's isEmptyInlineFlow: a whitespace-only header/paragraph
    // (e.g. the empty bullet a re-bulleted `|table|` body produces) renders as
    // nothing, not a blank line.
    if (INLINE_FLOW.has(b.kind) && ls.every((l) => l.trim() === "")) continue;
    lines.push(...ls);
  }
  while (lines.length > 1 && lines[lines.length - 1].trim() === "") lines.pop();
  return lines.join("\n");
}
