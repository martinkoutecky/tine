import { existingBlockId } from "../blockIdentity";
import { pushToast } from "../toasts";
import { t } from "../i18n";
import type { BlockDto, Format, PageDto, PageKind } from "../types";
import { blockRegions } from "../render/parse";
import { formatForPage, node, pageByName, pageRoots, type Node as ReadonlyNode } from "../document";
import { resultTicket, resultsEpoch, resultsTicketCurrent } from "../queryResultsEpoch";

export interface ResultRow { id: string; page: string; kind?: PageKind; path?: string; dto?: BlockDto; detached?: boolean }
const mounted = new WeakMap<BlockDto, { epoch: number; source: ReadonlyNode }>();
const dtoIndexes = new WeakMap<PageDto, Map<string, BlockDto[]>>();
function dtoIndex(page: PageDto): Map<string, BlockDto[]> {
  let index = dtoIndexes.get(page);
  if (index) return index;
  index = new Map();
  const visit = (blocks: BlockDto[]) => { for (const block of blocks) {
    const matches = index!.get(block.id) ?? []; matches.push(block); index!.set(block.id, matches);
    visit(block.children);
  } };
  visit(page.blocks); dtoIndexes.set(page, index); return index;
}

/** I-12: the only result -> live-node boundary. An already promoted node is
 * a live object, retained while its editor has ordinary unpublished input.
 * New promotions and actions require settled provenance; structural changes
 * retire retained objects too. Authored identities resolve uniquely. */
export function resolveResultRow(row: ResultRow, mode?: "action" | "mount"): ReadonlyNode | null;
export function resolveResultRow(row: ResultRow, mode: "action", sourcePage: PageDto & { id?: string }): BlockDto | null;
export function resolveResultRow(row: ResultRow, mode: "action" | "mount" = "action", sourcePage?: PageDto & { id?: string }): ReadonlyNode | BlockDto | null {
  if (row.detached) return null;
  if (sourcePage) {
    if (!row.dto || sourcePage.name !== row.page || sourcePage.kind !== row.kind || (row.path && sourcePage.id !== row.path)) return null;
    const format = sourcePage.format ?? "md";
    const index = dtoIndex(sourcePage);
    if (resultsTicketCurrent(resultTicket(row.dto))) {
      const matches = index.get(row.dto.id) ?? [];
      return matches.length === 1 ? matches[0] : null;
    }
    const durable = existingBlockId(row.dto.raw, format);
    if (!durable && !resultsTicketCurrent(resultTicket(row.dto))) return null;
    const matches: BlockDto[] = [];
    const visit = (blocks: BlockDto[]) => { for (const block of blocks) {
      if (durable ? existingBlockId(block.raw, format) === durable : block.id === row.dto!.id) matches.push(block);
      visit(block.children);
    } };
    visit(sourcePage.blocks);
    return matches.length === 1 ? matches[0] : null;
  }
  const page = pageByName(row.page);
  if (!page || (row.kind && page.kind !== row.kind) || (row.path && page.id !== row.path)) return null;
  if (!row.dto) { const source = node(row.id); return source?.page === row.page ? source : null; }
  const ticket = resultTicket(row.dto);
  const retained = mode === "mount" ? mounted.get(row.dto) : undefined;
  // Eviction is hydration, not an epoch: a retained source must still be live.
  if (retained && retained.epoch === resultsEpoch() && node(retained.source.id) === retained.source) return retained.source;
  const current = resultsTicketCurrent(ticket);
  let source = current ? node(row.dto.id) : undefined;
  if (current && !source) return null;
  const durable = !source && existingBlockId(row.dto.raw, formatForPage(row.page));
  if (durable) {
    const candidates: ReadonlyNode[] = [];
    const pending = [...pageRoots(row.page)];
    const seen = new Set<string>();
    while (pending.length) {
      const id = pending.pop()!;
      if (seen.has(id)) continue;
      seen.add(id);
      const candidate = node(id);
      if (!candidate || candidate.page !== row.page) continue;
      if (existingBlockId(candidate.raw, page.format) === durable) candidates.push(candidate);
      pending.push(...candidate.children);
    }
    if (candidates.length === 1) source = candidates[0];
  }
  if (!source || source.page !== row.page) return null;
  if (mode === "mount") mounted.set(row.dto, { epoch: resultsEpoch(), source });
  return source;
}

export function noticeChangedResults(count: number): void {
  if (count) pushToast(t("query-calm-results-changed", { count }), "error");
}

/** Page-property hits are synthetic read-only rows. Validate their parser-owned
 * properties against the current physical header rather than a runtime node. */
export function samePagePropertyResult(hit: BlockDto, header: string | null, format: Format): boolean {
  if (!header) return false;
  const properties = (raw: string) => blockRegions(raw, format).properties
    .filter(property => property.primary).map(property => [property.key.toLowerCase(), property.value]);
  const selected = properties(hit.raw);
  return selected.length > 0 && JSON.stringify(selected) === JSON.stringify(properties(header));
}
