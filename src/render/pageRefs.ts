// Link targets and block references, read off the ONE lsdoc parse (never a regex
// re-scan of raw). Which PAGES a block references is not answered here: that is
// the reference evidence's walk (`reference_evidence/walk.rs`), which the window
// asks through wasm (`block_reference_names`) and a rename asks natively
// (`rename_would_change`), conceptual model step 2 batch 6.
import { reference_target_name } from "./wasm/lsdoc_wasm.js";
import { parseBody } from "./facets";
import type { Block, Format, Inline, ListItem } from "./ast";

/** The graph's `:file/name-format` is legacy (the default). Set only by
 *  `graph.ts applyConfigDerivedState` from GraphMeta `file_name_format`. */
let legacyFileNames = true;

export function setFileNameFormat(format: string | null | undefined): void {
  legacyFileNames = format !== "triple-lowbar";
}

/** The page an accepted link target names, by the native policy
 *  (`block_regions::reference_target_name`, filename candidates): an Org
 *  `[[file:…]]` link names the linked file's page, decoded with the graph's
 *  file-name format as the store names that file. O(target). */
export function linkTargetName(kind: string, value: string, label: string, org: boolean): string | undefined {
  return reference_target_name(kind, value, label, org, true, legacyFileNames);
}

/** Every block uuid `raw` references with `((uuid))`, in source order (may repeat), off
 *  one lsdoc parse (plus one per property value or macro argument that could hold one):
 *  a lookalike inside code is literal and is not a reference. */
export function blockRefsInText(raw: string, format: Format): string[] {
  const out: string[] = [];
  collectRaw(raw, format, out, true);
  return out;
}

function collectRaw(raw: string, format: Format, out: string[], properties: boolean): void {
  // Only '[' and '(' open an inline block reference.
  if (!raw.includes("[") && !raw.includes("(")) return;
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
      // A value's own references (a nested parse never recurses into
      // properties again, so this is bounded).
      if (properties) for (const [, value] of block.props) collectRaw(value, format, out, false);
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
      case "emphasis":
      case "subscript":
      case "superscript":
        collectInlines(inline.children, format, out);
        break;
      case "link":
        if (inline.url.type === "block_ref") out.push(inline.url.v);
        if (inline.label) collectInlines(inline.label, format, out);
        break;
      case "macro":
        for (const arg of inline.args) collectRaw(arg, format, out, false);
        break;
    }
  }
}
