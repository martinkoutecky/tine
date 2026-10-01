// "The link at the caret", for GH #274 (Ctrl+O to open a page without the mouse).
//
// Transcribed from OG rather than invented, because this is a keyboard contract
// users already have muscle memory for:
// `frontend/util/thingatpt.cljs` `extract-nearest-link-from-text`, driven by
// `frontend/handler/editor.cljs` `get-nearest-page-or-url` / `get-nearest-page`
// and bound at `frontend/modules/shortcut/config.cljs:214` (`:editor/follow-link`,
// `mod+o`) and `:217` (`:editor/open-link-in-sidebar`, `mod+shift+o`).
//
// Note what OG's rule actually is, because it is not "the link under the caret":
// it collects EVERY candidate in the block and picks the one NEAREST the caret,
// so a caret anywhere in a block with a single link still follows it. That is
// friendlier than it sounds and it is the behaviour people are used to.

export type NearestLinkKind = "page" | "tag" | "block" | "url";

export interface NearestLink {
  kind: NearestLinkKind;
  /** The dereferenced target: page name, tag name, block uuid, or the URL. */
  value: string;
  /** Offsets of the whole match in the source text. */
  start: number;
  end: number;
}

import { parseBlock, blockRegions } from "../render/parse";
import { reference_target_name, nested_reference_names } from "../render/wasm/lsdoc_wasm.js";
import { inlineText } from "../render/facets";
import { utf8ToUtf16Cursor } from "../render/utf16Cursor";
import type { Block, Inline, ListItem, Format } from "../render/ast";

const rank: Record<NearestLinkKind, number> = { page: 0, block: 1, tag: 2, url: 3 };
const candidatesByAst = new WeakMap<Block[], NearestLink[]>();

/** Nearest accepted link, using OG's whole-block distance and kind tie policy.
 * No incomplete-token fallback: unaccepted syntax has no navigation target.
 * Cost O(block bytes × links) cold when nested spans revisit source prefixes;
 * warm selection is O(accepted links) after the parser-cache lookup, allocating
 * no candidates. Parser failures throw. Candidates share the bounded AST cache. */
export function nearestLink(text: string, caret: number, options: { includeUrls?: boolean; format?: Format } = {}): NearestLink | null {
  const blocks = parseBlock(text, options.format === "org");
  let candidates = candidatesByAst.get(blocks);
  if (!candidates) {
    candidates = collectCandidates(text, blocks, options.format ?? "md");
    candidatesByAst.set(blocks, candidates);
  }
  let best: NearestLink | null = null, bestScore = -Infinity;
  for (const candidate of candidates) {
    if (candidate.kind === "url" && !options.includeUrls) continue;
    const { start, end, kind } = candidate;
    const score = caret < start ? caret - start : caret > end ? end - caret : 0;
    if (score > bestScore || (score === bestScore && best && rank[kind] < rank[best.kind])) {
      best = candidate; bestScore = score;
    }
  }
  return best;
}

function collectCandidates(text: string, blocks: Block[], format: Format): NearestLink[] {
  const candidates: NearestLink[] = [];
  const lead = text.length - text.trimStart().length;
  const cursor = utf8ToUtf16Cursor(text.trimStart());
  const consider = (inline: Inline, kind: NearestLinkKind, value: string) => {
    const span = inline.span;
    if (!span || span[0] < 2 || !value.trim()) return;
    const start = lead + cursor(span[0] - 2);
    const end = lead + cursor(span[1] - 2);
    candidates.push({ kind, value: value.trim(), start, end });
  };
  const inlines = (nodes: readonly Inline[]) => {
    for (const inline of nodes) {
      if (inline.k === "tag") consider(inline, "tag", inlineText(inline.children));
      else if (inline.k === "link" && !inline.image) {
        const url = inline.url;
        if (url.type === "page_ref" || url.type === "search" || url.type === "file") {
          const name = reference_target_name(url.type, url.v, inline.label ? inlineText(inline.label) : "", format === "org", true);
          if (name) consider(inline, "page", name);
        }
        else if (url.type === "block_ref") consider(inline, "block", url.v);
        else if (url.type === "complex") {
          consider(inline, "url", url.protocol ? `${url.protocol}://${url.link ?? ""}` : url.link ?? inline.full);
        }
        if (inline.label) inlines(inline.label);
      } else if (inline.k === "nested_link") {
        for (const name of nested_reference_names(inline.content)) consider(inline, "page", name);
      } else if (inline.k === "emphasis" || inline.k === "subscript" || inline.k === "superscript") inlines(inline.children);
    }
  };
  const item = (node: ListItem) => {
    if (node.name) inlines(node.name);
    walk(node.content); node.items.forEach(item);
  };
  const walk = (nodes: readonly Block[]) => {
    for (const block of nodes) {
      if ("inline" in block) inlines(block.inline);
      else if (block.kind === "quote" || block.kind === "custom") walk(block.children);
      else if (block.kind === "list") block.items.forEach(item);
      else if (block.kind === "table") {
        block.header?.forEach(inlines); block.rows.forEach((row) => row.forEach(inlines));
      }
    }
  };
  walk(blocks);
  // Accepted properties own their value spans; the same reader handles links
  // in those values. Restrict the reparse to values with possible candidates.
  const valueCursor = utf8ToUtf16Cursor(text);
  for (const property of blockRegions(text, format).properties) {
    if (!property.primary || (!property.value.includes("[") && !property.value.includes("#"))) continue;
    const offset = valueCursor(property.value_range[0]);
    const end = valueCursor(property.value_range[1]);
    const value = text.slice(offset, end);
    const valueBlocks = parseBlock(value, format === "org");
    for (const candidate of collectCandidates(value, valueBlocks, format)) {
      candidates.push({ ...candidate, start: offset + candidate.start, end: offset + candidate.end });
    }
  }
  candidates.sort((a, b) => a.start - b.start);
  return candidates;
}
