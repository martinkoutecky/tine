import { type Format } from "../../types";
import { splitProps, hideAll, joinProps, isBuiltinHidden } from "../../editor/properties";
import { type ClipboardBlock, type ClipboardPayloadSlot, consumeCutGrant } from "../../clipboard";
import { doc, Node, formatForPage, freshId, setDoc, docHasBlockIdentity } from "../model";
import { graphTransitioning } from "../../ui";
import { pageInstanceGeneration, markDirty, flushCutSourcePages, cutSourcePagesRetired } from "../save/engine";
import { graphEpoch, graphMeta } from "../../graphSession";
import { unwrap, produce } from "solid-js/store";
import { blockWritable } from "./properties";
import { existingBlockId, UUID_RE } from "./identity";
import { pushUndo } from "../history";
import { backend } from "../../backend";

type ClipboardProperty = { key: string; value: string };

function clipboardProperties(raw: string, format: Format): ClipboardProperty[] {
  const hidden = splitProps(raw, hideAll, format).hidden;
  if (!hidden) return [];
  const properties: ClipboardProperty[] = [];
  for (const line of hidden.split("\n")) {
    const match = format === "org"
      ? /^\s*:([A-Za-z0-9_@./-]+):\s*(.*)$/.exec(line)
      : /^\s*([A-Za-z0-9_./-]+)::\s*(.*)$/.exec(line);
    if (match) properties.push({ key: match[1], value: match[2] });
  }
  return properties;
}

function clipboardIdsForBlock(block: ClipboardBlock): string[] {
  return clipboardProperties(block.raw, block.sourceFormat)
    .filter((property) => property.key.toLowerCase() === "id")
    .map((property) => property.value.trim());
}

function clipboardRawForTarget(
  block: ClipboardBlock,
  targetFormat: Format,
  preserveIds: boolean,
): string {
  if (block.sourceFormat === targetFormat) {
    return preserveIds
      ? block.raw
      : splitProps(block.raw, (key) => key.toLowerCase() === "id", block.sourceFormat).visible;
  }

  // splitProps/joinProps classify metadata but deliberately do not translate
  // syntax. Map the ordered key/value stream explicitly so every property keeps
  // its relative order across Markdown `key:: value` and Org drawer forms.
  const visible = splitProps(block.raw, hideAll, block.sourceFormat).visible;
  const properties = clipboardProperties(block.raw, block.sourceFormat)
    .filter((property) => preserveIds || property.key.toLowerCase() !== "id");
  const translated = properties.map(({ key, value }) =>
    targetFormat === "org" ? `:${key}: ${value}` : `${key}:: ${value}`
  ).join("\n");
  return joinProps(visible, translated, targetFormat);
}

function clipboardCollapsed(block: ClipboardBlock): boolean {
  return clipboardProperties(block.raw, block.sourceFormat)
    .some(({ key, value }) => key.toLowerCase() === "collapsed" && value.trim().toLowerCase() === "true");
}

function liveDocReferences(id: string): boolean {
  const escaped = id.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
  const reference = new RegExp(`\\(\\(${escaped}\\)\\)`, "i");
  return Object.values(doc.byId).some((node) => reference.test(node.raw));
}

interface ClipboardPasteAuthority {
  epoch: number;
  root: string;
  targetId: string;
  targetNode: Node;
  targetPage: string;
  targetGeneration: number;
}

function captureClipboardPasteAuthority(targetId: string): ClipboardPasteAuthority | null {
  const target = doc.byId[targetId];
  if (!target || graphTransitioning()) return null;
  const targetGeneration = pageInstanceGeneration(target.page);
  if (targetGeneration === null) return null;
  return {
    epoch: graphEpoch(),
    root: graphMeta()?.root ?? "",
    targetId,
    targetNode: unwrap(target),
    targetPage: target.page,
    targetGeneration,
  };
}

function clipboardPasteAuthorityCurrent(authority: ClipboardPasteAuthority): boolean {
  const target = doc.byId[authority.targetId];
  return !graphTransitioning()
    && graphEpoch() === authority.epoch
    && (graphMeta()?.root ?? "") === authority.root
    && !!target
    && unwrap(target) === authority.targetNode
    && target.page === authority.targetPage
    && pageInstanceGeneration(authority.targetPage) === authority.targetGeneration;
}

function insertClipboardBlocksSync(
  targetId: string,
  blocks: readonly ClipboardBlock[],
  preserveIds: boolean,
  preservedIds: readonly string[],
): string | null {
  const target = doc.byId[targetId];
  if (!blocks.length || !target || !blockWritable(targetId)) return null;
  const targetFormat = formatForPage(target.page);
  const prepared = blocks.map(function prepare(block): {
    id: string;
    raw: string;
    collapsed: boolean;
    children: ReturnType<typeof prepare>[];
  } {
    const sourceIds = clipboardIdsForBlock(block);
    return {
      id: preserveIds && sourceIds.length === 1 ? sourceIds[0].toLowerCase() : freshId(),
      raw: clipboardRawForTarget(block, targetFormat, preserveIds),
      collapsed: clipboardCollapsed(block),
      children: block.children.map(prepare),
    };
  });
  const visible = splitProps(target.raw, isBuiltinHidden, targetFormat).visible;
  const replaceHost = target.children.length === 0
    && visible.trim() === ""
    && existingBlockId(target.raw, targetFormat) === null
    && !liveDocReferences(targetId);
  const parent = target.parent;
  const pageName = target.page;
  let lastId: string | null = null;

  pushUndo("clipboard-paste", [pageName], preserveIds ? preservedIds : []);
  setDoc(produce((state) => {
    const create = (block: typeof prepared[number], blockParent: string | null): string => {
      const children = block.children.map((child) => create(child, block.id));
      state.byId[block.id] = {
        id: block.id,
        raw: block.raw,
        collapsed: block.collapsed,
        parent: blockParent,
        page: pageName,
        children,
      };
      return block.id;
    };
    const created = prepared.map((block) => create(block, parent));
    const siblings = parent === null
      ? state.pages[state.pages.findIndex((page) => page.name === pageName)].roots
      : state.byId[parent].children;
    const at = siblings.indexOf(targetId);
    if (replaceHost) {
      siblings.splice(at, 1, ...created);
      delete state.byId[targetId];
    } else {
      siblings.splice(at + 1, 0, ...created);
    }
    lastId = created[created.length - 1] ?? null;
  }));
  markDirty(pageName);
  return lastId;
}

/** Associate an already-captured private clipboard slot with one target. The
 * wrapper is intentionally non-async: a cut grant is consumed synchronously,
 * before the returned continuation can reach retirement or any other await. */
export function pasteClipboardPayload(
  targetId: string,
  slot: ClipboardPayloadSlot,
): Promise<string | null> {
  const authority = captureClipboardPasteAuthority(targetId);
  const grant = slot.op === "cut" ? consumeCutGrant(slot.generation) : null;
  if (!authority) return Promise.resolve(null);

  const idLists: string[][] = [];
  const visit = (block: ClipboardBlock) => {
    idLists.push(clipboardIdsForBlock(block));
    block.children.forEach(visit);
  };
  slot.blocks.forEach(visit);
  const ids = idLists.flat();
  const normalizedIds = ids.map((id) => id.toLowerCase());
  const idsValid = idLists.every((blockIds) => blockIds.length <= 1)
    && ids.every((id) => UUID_RE.test(id))
    && new Set(normalizedIds).size === normalizedIds.length;

  return (async () => {
    let preserveIds = !!grant
      && ids.length > 0
      && idsValid
      && slot.graph === authority.root;

    if (preserveIds) {
      preserveIds = await flushCutSourcePages(grant!.sourcePages);
      if (preserveIds && !clipboardPasteAuthorityCurrent(authority)) return null;
    }
    if (preserveIds) {
      try {
        const resolved = await backend().resolveBlocks(normalizedIds);
        preserveIds = resolved.length === normalizedIds.length && resolved.every((block) => block === null);
      } catch {
        preserveIds = false;
      }
    }

    // Final JS-single-thread section: every authority and retirement check is
    // synchronous and insertion follows immediately with no await boundary.
    if (!clipboardPasteAuthorityCurrent(authority)) return null;
    if (preserveIds) {
      preserveIds = cutSourcePagesRetired(grant!.sourcePages)
        && normalizedIds.every((id) => !docHasBlockIdentity(id));
    }
    return insertClipboardBlocksSync(targetId, slot.blocks, preserveIds, preserveIds ? normalizedIds : []);
  })();
}

