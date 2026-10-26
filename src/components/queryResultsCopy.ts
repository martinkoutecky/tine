import { exportNodesFor, formatForPage, pageByName } from "../document";
import { noticeChangedResults, samePagePropertyResult, resolveResultRow } from "./queryResultIdentity";
import { resultTicket, resultsTicketActionable } from "../queryResultsEpoch";
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
  const validators: (() => boolean)[] = [];
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
          validators.push(() => block.page_property ? resultsTicketActionable(resultTicket(block))
            : !!resolveResultRow({ id: block.id, dto: block, page: group.page, kind: group.kind, path: group.path }));
          if (block.page_property) {
            if (resultsTicketActionable(resultTicket(block)) && samePagePropertyResult(block, loaded!.preBlock, format)) propertyRoots.push(block);
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
        const result = await readOwned(owner, group.path ? backend().getPageByPath(group.path) : backend().getPage(group.page, group.kind));
        if (result.kind === "stale") return;
        const dto = result.value;
        if (!dto || dto.name !== group.page || dto.kind !== group.kind || (group.path && (!("id" in dto) || dto.id !== group.path))) throw new Error("source unavailable");
        const sourceFormat = dto.format ?? "md";
        format = sourceFormat;
        const roots: BlockDto[] = [];
        for (const block of group.blocks) {
          validators.push(() => block.page_property ? resultsTicketActionable(resultTicket(block))
            : !!resolveResultRow({ id: block.id, dto: block, page: group.page, kind: group.kind, path: group.path }, "action", dto));
          if (block.page_property) {
            if (resultsTicketActionable(resultTicket(block)) && samePagePropertyResult(block, dto.pre_block, sourceFormat)) roots.push(block);
            else changed++;
            continue;
          }
          const source = resolveResultRow({ id: block.id, dto: block, page: group.page, kind: group.kind, path: group.path }, "action", dto);
          if (!source) changed++;
          else roots.push(source);
        }
        count += roots.length;
        children = blockDtosToExportNodes(uniqueResultRoots(roots), sourceFormat);
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
