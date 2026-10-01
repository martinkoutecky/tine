import { doc, formatForBlock, pageByName } from "../model";
import { splitProps } from "../../editor/properties";
import { type ClipboardPayloadData, type ClipboardSourcePage, type ClipboardBlock, CLIPBOARD_PAYLOAD_MAX_BLOCKS, CLIPBOARD_PAYLOAD_MAX_RAW_BYTES } from "../../clipboard";
import { pageInstanceGeneration } from "../save/engine";
import { type ExportNode } from "../../editor/exportText";
import { type BlockDto } from "../../types";

/** Serialize a block (and, normally, its subtree) to Logseq markdown.
 *  - `stripId`: drop the internal `id::` property line (fence-aware) — OG does this
 *    when copying to the clipboard (`copy-to-clipboard-without-id-property!`) so a
 *    referenced block doesn't leak `id:: <uuid>` into pasted text. (Quick-capture
 *    writing to a journal FILE passes false to keep `id::`.)
 *  - `stripCollapsed`: also drop `collapsed::` (OG keeps it; opt-in cleaner copy).
 *  - `onlySelected`: when a Set is passed, recurse only into children that are in it
 *    (used by the "copy only the selected blocks, not the whole sub-tree" mode). */
export function blockSubtreeMarkdown(
  id: string,
  level = 0,
  stripId = false,
  stripCollapsed = false,
  onlySelected?: Set<string>
): string {
  const n = doc.byId[id];
  if (!n) return "";
  const format = formatForBlock(id);
  const strip = stripId || stripCollapsed;
  const raw = strip
    ? splitProps(
        n.raw,
        (k) => (stripId && k === "id") || (stripCollapsed && k === "collapsed"),
        format,
      ).visible
    : n.raw;
  const out = markdownBlockLines(raw, level);
  for (const c of n.children) {
    if (onlySelected && !onlySelected.has(c)) continue;
    out.push(blockSubtreeMarkdown(c, level + 1, stripId, stripCollapsed, onlySelected));
  }
  return out.join("\n");
}

/**
 * Build the private clipboard forest from selection roots. Unlike the public
 * text flavor this always includes the complete subtree and exact raw strings,
 * including id::/collapsed:: and hidden properties. Returns null (without
 * affecting the public copy) when the bounded in-memory payload is too large.
 */
export function buildClipboardPayload(ids: string[]): ClipboardPayloadData | null {
  const selected = new Set(ids.filter((id) => !!doc.byId[id]));
  const roots = [...selected].filter((id) => !hasSelectedAncestor(id, selected));
  if (roots.length === 0) return null;

  let blockCount = 0;
  let rawBytes = 0;
  const encoder = new TextEncoder();
  const pages = new Map<string, ClipboardSourcePage>();

  const build = (id: string): ClipboardBlock | null => {
    const node = doc.byId[id];
    if (!node) return null;
    blockCount++;
    rawBytes += encoder.encode(node.raw).byteLength;
    if (blockCount > CLIPBOARD_PAYLOAD_MAX_BLOCKS || rawBytes > CLIPBOARD_PAYLOAD_MAX_RAW_BYTES) return null;

    const page = pageByName(node.page);
    const generation = pageInstanceGeneration(node.page);
    if (!page || generation === null) return null;
    if (!pages.has(page.name)) {
      pages.set(page.name, {
        name: page.name,
        kind: page.kind,
        ...(page.id ? { path: page.id } : {}),
        generation,
      });
    }

    const children: ClipboardBlock[] = [];
    for (const child of node.children) {
      const built = build(child);
      if (!built) return null;
      children.push(built);
    }
    return { key: id, raw: node.raw, children, sourceFormat: page.format };
  };

  const blocks: ClipboardBlock[] = [];
  for (const id of roots) {
    const built = build(id);
    if (!built) return null;
    blocks.push(built);
  }
  return { blocks, sourcePages: [...pages.values()] };
}

/** Build an ExportNode forest (raw + children) for the given block ids and their
 *  subtrees — input to the configurable text exporter (Copy / Export modal). */
export function exportNodesFor(ids: string[]): ExportNode[] {
  const set = new Set(ids);
  // A multi-selection (selectedIds) is a flat slice of visible order, so it can
  // contain BOTH a parent and its descendants. Export only the selection's roots
  // — a kept node's subtree already carries its children, so emitting a selected
  // child again as a top-level node would duplicate it (the "1 2 3 1 2 3" bug).
  const toNode = (id: string): ExportNode | null => {
    const n = doc.byId[id];
    if (!n) return null;
    return {
      raw: n.raw,
      format: pageByName(n.page)?.format ?? "md",
      children: n.children.map(toNode).filter((x): x is ExportNode => x != null),
    };
  };
  return ids
    .filter((id) => !hasSelectedAncestor(id, set))
    .map(toNode)
    .filter((x): x is ExportNode => x != null);
}

/** Serialize a fetched BlockDto subtree to Logseq markdown (for pages not in the
 *  working set, e.g. copy-page-as-markdown). */
export function dtoSubtreeMarkdown(b: BlockDto, level = 0): string {
  const out = markdownBlockLines(b.raw, level);
  for (const c of b.children) out.push(dtoSubtreeMarkdown(c, level + 1));
  return out.join("\n");
}

/** Clipboard outline formatting, shared by live and fetched inputs; O(raw).
 * OG copies Org blocks as Markdown; metadata stripping above uses the SOURCE
 * format so Org drawers are removed before this portable outline is emitted. */
function markdownBlockLines(raw: string, level: number): string[] {
  const tabs = "\t".repeat(level);
  const lines = raw.split("\n");
  const out = [`${tabs}- ${lines[0] ?? ""}`.replace(/\s+$/, "")];
  for (const line of lines.slice(1)) out.push(line === "" ? "" : `${tabs}  ${line}`);
  return out;
}

function hasSelectedAncestor(id: string, selected: Set<string>): boolean {
  let parent = doc.byId[id]?.parent ?? null;
  while (parent !== null) {
    if (selected.has(parent)) return true;
    parent = doc.byId[parent]?.parent ?? null;
  }
  return false;
}
