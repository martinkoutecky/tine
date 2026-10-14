import { exportNodesFor, formatForPage, node, pageByName, resolveBlockRef } from "../document";
import { existingBlockId } from "../blockIdentity";
import { noticeChangedResults, samePagePropertyResult, sameQueryResult } from "./queryResultIdentity";
import { backend } from "../backend";
import { graphOwner, readOwned } from "../owned";
import { pushToast } from "../toasts";
import { t } from "../i18n";
import { openExportNodesModal } from "../ui";
import { blockDtosToExportNodes } from "./ExportModal";
import type { ExportNode } from "../editor/exportText";
import type { BlockDto, PageEntry, RefGroup } from "../types";

export function uniqueResultRoots(blocks: BlockDto[]): BlockDto[] {
  const descendants = new Set<string>();
  const visit = (children: BlockDto[]) => { for (const child of children) { descendants.add(child.id); visit(child.children); } };
  for (const block of blocks) visit(block.children);
  const seen = new Set<string>();
  return blocks.filter(block => !descendants.has(block.id) && !seen.has(block.id) && !!seen.add(block.id));
}

/** The manager's GH #387 format: page/date titles wrap the result trees,
 * which go through the same Copy / export as dialog as outline selections. */
export function queryResultsCopyNodes(groups: RefGroup[], pages: Pick<PageEntry, "name">[] = []): ExportNode[] {
  return [
    ...pages.map((page) => ({ raw: page.name, format: formatForPage(page.name), children: [] })),
    ...groups.map((group) => {
      const format = formatForPage(group.page);
      return { raw: group.page, format, children: blockDtosToExportNodes(uniqueResultRoots(group.blocks), format) };
    }),
  ];
}

/** Read selected source pages once each, sequentially. Selection is frozen at
 * invocation and graph ownership covers every await and modal opening. */
export async function openQueryResultsCopy(groups: RefGroup[], pages: Pick<PageEntry, "name">[] = []): Promise<void> {
  const owner = graphOwner();
  const forest: ExportNode[] = pages.map(page => ({ raw: page.name, children: [] }));
  let changed = 0;
  let count = pages.length;
  try {
    for (const group of groups) {
      if (!owner()) return;
      const loaded = pageByName(group.page);
      const exact = loaded?.kind === group.kind && (!group.path || loaded.id === group.path);
      let format = exact ? formatForPage(group.page) : "md";
      let children: ExportNode[];
      if (exact) {
        const ids: string[] = [];
        const propertyRoots: BlockDto[] = [];
        for (const block of group.blocks) {
          if (block.page_property) {
            if (samePagePropertyResult(block, loaded!.preBlock, format)) propertyRoots.push(block);
            else changed++;
            continue;
          }
          const durable = existingBlockId(block.raw, format);
          const id = durable ? resolveBlockRef({ uuid: durable, page: group.page, pageKind: group.kind, path: group.path }) : block.id;
          const source = id ? node(id) : undefined;
          if (!source || source.page !== group.page || !sameQueryResult(block, source, format)) changed++;
          else ids.push(source.id);
        }
        count += ids.length + propertyRoots.length;
        children = [...blockDtosToExportNodes(propertyRoots, format), ...exportNodesFor(ids)];
      } else {
        const result = await readOwned(owner, group.path ? backend().getPageByPath(group.path) : backend().getPage(group.page, group.kind));
        if (result.kind === "stale") return;
        const dto = result.value;
        if (!dto || dto.name !== group.page || dto.kind !== group.kind || (group.path && (!("id" in dto) || dto.id !== group.path))) throw new Error("source unavailable");
        const byId = new Map<string, BlockDto>();
        const visit = (blocks: BlockDto[]) => { for (const block of blocks) { byId.set(block.id, block); visit(block.children); } };
        visit(dto.blocks);
        const sourceFormat = dto.format ?? "md";
        format = sourceFormat;
        const roots: BlockDto[] = [];
        for (const block of group.blocks) {
          if (block.page_property) {
            if (samePagePropertyResult(block, dto.pre_block, sourceFormat)) roots.push(block);
            else changed++;
            continue;
          }
          const durable = existingBlockId(block.raw, sourceFormat);
          const candidates = durable ? [...byId.values()].filter(source => sameQueryResult(block, source, sourceFormat)) : [byId.get(block.id)];
          const source = candidates.length === 1 ? candidates[0] : undefined;
          if (!source || !sameQueryResult(block, source, sourceFormat)) changed++;
          else roots.push(source);
        }
        count += roots.length;
        children = blockDtosToExportNodes(uniqueResultRoots(roots), sourceFormat);
      }
      if (children.length) forest.push({ raw: group.page, format, children });
    }
    if (owner()) {
      noticeChangedResults(changed);
      if (forest.length) openExportNodesModal(forest, count);
    }
  } catch {
    if (owner()) pushToast(t("query-calm-copy-failed"), "error");
  }
}
