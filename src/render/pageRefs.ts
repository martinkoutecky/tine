// The page names a block's text references, read off the ONE lsdoc parse (never a
// regex re-scan of raw). It answers the question the backend rename answers with
// `refs::rename_refs_multi` + `rename_tags_property_multi`: `[[page]]`, `#tag`,
// `#[[page]]`, Org `[[file:…/page.org]]` links, bare `tags::` values, and those
// same references inside property values and macro arguments. References inside
// code are literal, as in the backend. Names are returned as written; compare them
// with `pageIdentityKey`.
import { reference_target_name, nested_reference_names } from "./wasm/lsdoc_wasm.js";
import { parseBody, inlineText } from "./facets";
import { isQuotedPagePropertyValue, normalizeImplicitPageName, propertyKeyNorm, splitLinkableProperty } from "./block";
import type { Block, Format, Inline, ListItem } from "./ast";

/** Every page name `raw` references, in source order (may repeat). One lsdoc parse
 *  of `raw`, plus one per property value or macro argument that could hold a
 *  reference. */
export function pageRefsInText(raw: string, format: Format): string[] {
  const out: string[] = [];
  collectRaw(raw, format, out, true);
  return out;
}

function collectRaw(raw: string, format: Format, out: string[], properties: boolean): void {
  // Only '[' and '#' open an inline reference; bare tags need `::`.
  if (!raw.includes("[") && !raw.includes("#") && !raw.includes("::")) return;
  for (const block of parseBody(raw, format)) collectBlock(block, format, out, properties);
}

function collectBlock(block: Block, format: Format, out: string[], properties: boolean): void {
  switch (block.kind) {
    case "paragraph":
    case "heading":
    case "bullet":
    case "footnote_def":
      collectInlines(block.inline, format, out);
      break;
    case "quote":
    case "custom":
      block.children.forEach((child) => collectBlock(child, format, out, properties));
      break;
    case "list":
      block.items.forEach((item) => collectItem(item, format, out, properties));
      break;
    case "table":
      block.header?.forEach((cell) => collectInlines(cell, format, out));
      block.rows.forEach((row) => row.forEach((cell) => collectInlines(cell, format, out)));
      break;
    case "properties":
      if (!properties) break;
      for (const [key, value] of block.props) {
        // A value's own `[[…]]`/`#…` references (a nested parse never recurses
        // into properties again, so this is bounded).
        collectRaw(value, format, out, false);
        if (propertyKeyNorm(key) !== "tags" || isQuotedPagePropertyValue(value)) continue;
        for (const part of splitLinkableProperty(value)) {
          const bare = part.trim();
          if (bare && !bare.startsWith("[[") && !bare.startsWith("#")) out.push(normalizeImplicitPageName(bare));
        }
      }
      break;
  }
}

function collectItem(item: ListItem, format: Format, out: string[], properties: boolean): void {
  if (item.name) collectInlines(item.name, format, out);
  item.content.forEach((block) => collectBlock(block, format, out, properties));
  item.items.forEach((child) => collectItem(child, format, out, properties));
}

function collectInlines(inlines: readonly Inline[], format: Format, out: string[]): void {
  for (const inline of inlines) {
    switch (inline.k) {
      case "tag":
        out.push(inlineText(inline.children));
        break;
      case "emphasis":
      case "subscript":
      case "superscript":
        collectInlines(inline.children, format, out);
        break;
      case "link":
        // FilenameCandidates is the established rename-candidate policy:
        // Org file links select decoded stems, native evidence selects labels.
        const name = reference_target_name(inline.url.type, "v" in inline.url ? inline.url.v : "",
          inline.label ? inlineText(inline.label) : "", format === "org", true);
        if (name) out.push(name);
        if (inline.label) collectInlines(inline.label, format, out);
        break;
      case "nested_link":
        out.push(...nested_reference_names(inline.content));
        break;
      case "macro":
        for (const arg of inline.args) collectRaw(arg, format, out, false);
        break;
    }
  }
}
