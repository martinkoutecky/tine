import { exportNodesFor, node, pageByName, resolveBlockRef } from "../document";
import { noticeChangedResults, samePreBlockResult, resolveResultRow, referenceExportSource } from "./queryResultIdentity";
import { blockDtoExternalId } from "../blockIdentity";
import { resultTicket, resultsTicketActionable } from "../queryResultsEpoch";
import { backend } from "../backend";
import { graphOwner, readOwned, type Owner } from "../owned";
import { pushToast } from "../toasts";
import { msg, t } from "../i18n";
import { openExportNodesModal } from "../ui";
import { blockDtosToExportNodes } from "./ExportModal";
import type { ExportNode } from "../editor/exportText";
import type { BlockDto, PageEntry, RefGroup } from "../types";
import { projectExportNodes } from "../editor/exportProjection";

export function uniqueResultRoots(blocks: BlockDto[]): BlockDto[] {
  const descendants = new Set<string>();
  const visited = new Set<BlockDto>();
  const visit = (block: BlockDto) => {
    if (visited.has(block)) return;
    visited.add(block);
    for (const child of block.children) {
      descendants.add(child.id);
      visit(child);
    }
  };
  for (const block of blocks) visit(block);
  const seen = new Set<string>();
  return blocks.filter(block => !descendants.has(block.id) && !seen.has(block.id) && !!seen.add(block.id));
}

function loadedSourcePage(group: RefGroup) {
  const page = pageByName(group.page);
  return page?.kind === group.kind && (!group.path || page.id === group.path) ? page : undefined;
}

async function readSourcePage(group: RefGroup, owner: Owner) {
  const result = await readOwned(owner, group.path ? backend().getPageByPath(group.path) : backend().getPage(group.page, group.kind));
  if (result.kind === "stale") return null;
  const page = result.value;
  if (!page || page.name !== group.page || page.kind !== group.kind || (group.path && page.id !== group.path)) {
    throw new Error("source unavailable");
  }
  return page;
}

/** Open the page outline exporter for selected reference trees. Loaded roots
 * use the ordinary page selection. Read each cold source page at most once
 * without mounting it, then convert only the selected subtrees. Returns false
 * if a source or the chooser has gone away. */
export async function openReferenceResultsCopy(groups: RefGroup[], current: Owner): Promise<boolean> {
  const owner = graphOwner(current);
  const forest: ExportNode[] = [];
  let count = 0;
  try {
    for (const group of groups) {
      if (!owner()) return false;
      const loaded = loadedSourcePage(group);
      const outline = group.blocks.filter(block => !block.page_property && !block.page_preamble);
      const headers = group.blocks.filter(block => block.page_property || block.page_preamble);
      const sourcePage = loaded ? null : await readSourcePage(group, owner);
      if (!loaded && !sourcePage) return false;
      const format = loaded?.format ?? sourcePage?.format ?? "md";
      const children = blockDtosToExportNodes(headers, format);
      if (loaded) {
        const ids = outline.map(block => {
          // The row's own runtime key still naming the same text is the block:
          // no page-wide authored-ID index for an ordinary export (GH #793).
          const at = { page: group.page, pageKind: group.kind, ...(group.path ? { path: group.path } : {}) };
          const keyed = resolveBlockRef({ uuid: block.id, ...at }, { locator: true });
          if (keyed !== null && node(keyed)?.raw === block.raw) return keyed;
          const id = resolveBlockRef({
            uuid: blockDtoExternalId(block), page: group.page, pageKind: group.kind,
            ...(group.path ? { path: group.path } : {}),
          }, { navigation: true });
          if (id === null) throw new Error("reference moved");
          return id;
        });
        children.push(...exportNodesFor([...new Set(ids)], true));
      } else if (sourcePage) {
        const roots = outline.map(block => {
          const source = referenceExportSource(block, sourcePage);
          if (!source) throw new Error("reference moved");
          return source;
        });
        const references = new Map(roots.map((root, index) => [root.id, outline[index]]));
        for (const root of uniqueResultRoots(roots)) {
          const reference = references.get(root.id)!;
          children.push(...blockDtosToExportNodes([root], format, !!reference.breadcrumb?.length, references));
        }
      }
      count += group.blocks.length;
      forest.push({ raw: group.page, format, children });
    }
    if (!owner()) return false;
    openExportNodesModal(forest, count, true);
    return true;
  } catch {
    if (owner()) pushToast(msg("references-export-source-changed"), "error");
    return false;
  }
}

/** Read selected source pages once each, sequentially. Selection is frozen at
 * invocation and graph ownership covers every await and modal opening. The
 * manager's GH #387 format: page/date titles wrap the result trees, which go
 * through the same Copy / export as dialog as outline selections. */
export async function openQueryResultsCopy(groups: RefGroup[], pages: Pick<PageEntry, "name">[] = []): Promise<void> {
  const owner = graphOwner();
  const forest: ExportNode[] = pages.map(page => ({ raw: page.name, children: [] }));
  let changed = 0;
  let count = pages.length;
  const validators: (() => boolean)[] = [];
  try {
    for (const group of groups) {
      if (!owner()) return;
      const loaded = loadedSourcePage(group);
      let format = loaded?.format ?? "md";
      let children: ExportNode[];
      if (loaded) {
        const ids: string[] = [];
        const propertyRoots: BlockDto[] = [];
        for (const block of group.blocks) {
          validators.push(() => block.page_property || block.page_preamble ? resultsTicketActionable(resultTicket(block)) && samePreBlockResult(block, loaded!.preBlock, format)
            : !!resolveResultRow({ id: block.id, dto: block, page: group.page, kind: group.kind, path: group.path }));
          if (block.page_property || block.page_preamble) {
            if (resultsTicketActionable(resultTicket(block)) && samePreBlockResult(block, loaded!.preBlock, format)) propertyRoots.push(block);
            else changed++;
            continue;
          }
          const source = resolveResultRow({ id: block.id, dto: block, page: group.page, kind: group.kind, path: group.path });
          if (!source) changed++;
          else ids.push(source.id);
        }
        count += ids.length + propertyRoots.length;
        children = [...blockDtosToExportNodes(propertyRoots, format), ...exportNodesFor(ids)];
      } else {
        const dto = await readSourcePage(group, owner);
        if (!dto) return;
        const sourceFormat = dto.format ?? "md";
        format = sourceFormat;
        const roots: BlockDto[] = [];
        for (const block of group.blocks) {
          validators.push(() => block.page_property || block.page_preamble ? resultsTicketActionable(resultTicket(block)) && samePreBlockResult(block, dto.pre_block, sourceFormat)
            : !!resolveResultRow({ id: block.id, dto: block, page: group.page, kind: group.kind, path: group.path }, "action", dto));
          if (block.page_property || block.page_preamble) {
            if (resultsTicketActionable(resultTicket(block)) && samePreBlockResult(block, dto.pre_block, sourceFormat)) roots.push(block);
            else changed++;
            continue;
          }
          const source = resolveResultRow({ id: block.id, dto: block, page: group.page, kind: group.kind, path: group.path }, "action", dto);
          if (!source) changed++;
          else roots.push(source);
        }
        count += roots.length;
        const projected = projectExportNodes(blockDtosToExportNodes(dto.blocks, sourceFormat));
        const byId = new Map<string, ExportNode>();
        const index = (nodes: ExportNode[]) => { for (const node of nodes) { if (node.id) byId.set(node.id, node); index(node.children); } };
        index(projected);
        children = uniqueResultRoots(roots).map(block => byId.get(block.id) ?? blockDtosToExportNodes([block], sourceFormat)[0]);
      }
      if (children.length) forest.push({ raw: group.page, format, children });
    }
    if (owner()) {
      changed = Math.max(changed, validators.filter(validate => !validate()).length);
      noticeChangedResults(changed);
      if (!changed && forest.length) openExportNodesModal(forest, count);
    }
  } catch {
    if (owner()) pushToast(t("query-calm-copy-failed"), "error");
  }
}
