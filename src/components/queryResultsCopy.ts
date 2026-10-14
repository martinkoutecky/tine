import { exportNodesFor, formatForPage, node, pageByName } from "../document";
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
  try {
    for (const group of groups) {
      if (!owner()) return;
      const loaded = pageByName(group.page);
      const exact = loaded?.kind === group.kind && (!group.path || loaded.id === group.path);
      const format = exact ? formatForPage(group.page) : "md";
      let children: ExportNode[];
      if (exact && group.blocks.every(block => node(block.id)?.page === group.page)) {
        children = exportNodesFor(group.blocks.map(block => block.id));
      } else {
        const result = await readOwned(owner, group.path ? backend().getPageByPath(group.path) : backend().getPage(group.page, group.kind));
        if (result.kind === "stale") return;
        const dto = result.value;
        if (!dto || dto.name !== group.page || dto.kind !== group.kind || (group.path && (!("id" in dto) || dto.id !== group.path))) throw new Error("source unavailable");
        const byId = new Map<string, BlockDto>();
        const visit = (blocks: BlockDto[]) => { for (const block of blocks) { byId.set(block.id, block); visit(block.children); } };
        visit(dto.blocks);
        const roots = group.blocks.map(block => block.page_property ? block : byId.get(block.id));
        if (roots.some(block => !block)) throw new Error("source block unavailable");
        children = blockDtosToExportNodes(uniqueResultRoots(roots as BlockDto[]), dto.format ?? "md");
      }
      forest.push({ raw: group.page, format, children });
    }
    if (owner() && forest.length) openExportNodesModal(forest, pages.length + groups.reduce((sum, group) => sum + group.blocks.length, 0));
  } catch {
    if (owner()) pushToast(t("query-calm-copy-failed"), "error");
  }
}
