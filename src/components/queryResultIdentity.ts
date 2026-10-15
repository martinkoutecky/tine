import { createContext } from "solid-js";
import { existingBlockId } from "../blockIdentity";
import { pushToast } from "../toasts";
import { t } from "../i18n";
import type { BlockDto, Format, PageDto, PageKind } from "../types";
import { blockRegions } from "../render/parse";
import { formatForPage, node, pageByName, pageInstanceGeneration, pageRoots, type Node as ReadonlyNode } from "../document";
import { resultTicket, resultsActionable, resultsEpoch, resultsTicketActionable } from "../queryResultsEpoch";
import { bindingCurrent, captureBinding } from "../binding";

export interface ResultRow { id: string; page: string; kind?: PageKind; path?: string; dto?: BlockDto; detached?: boolean }
const mounted = new WeakMap<BlockDto, { source: ReadonlyNode; instance: number | null }>();
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
    if (resultsTicketActionable(resultTicket(row.dto))) {
      const matches = index.get(row.dto.id) ?? [];
      return matches.length === 1 ? matches[0] : null;
    }
    const durable = existingBlockId(row.dto.raw, format);
    // Authored identity survives replacement, not a held or in-flight change.
    if (!durable || !resultsActionable()) return null;
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
  // Display outlives an epoch while the source page instance is unchanged: a
  // structural edit elsewhere (Enter on today's journal) must not drop every
  // shown result to its stand-in until the rerun. Local edits never reassign a
  // runtime id; installing backend content over a loaded page (the only same-ID
  // merge, workingSet upsertPage) starts a new instance and retires the row.
  // Actions keep the strict epoch.
  if (retained && node(retained.source.id) === retained.source && retained.source.page === row.page
    && retained.instance === pageInstanceGeneration(row.page)) return retained.source;
  const current = resultsTicketActionable(ticket);
  let source = current ? node(row.dto.id) : undefined;
  if (source?.page !== row.page) source = undefined;
  // Projected results already carry the parser's negative identity fact. Keep
  // missing positional rows cheap; older/unprojected DTOs still ask the parser.
  const durable = !source && row.dto.has_id !== false && resultsActionable() && existingBlockId(row.dto.raw, formatForPage(row.page));
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
  if (mode === "mount") mounted.set(row.dto, { source, instance: pageInstanceGeneration(row.page) });
  return source;
}

/** An already-open property draft owns the exact live block and page instance,
 * not its old positional result. Hydration/replacement retires that ownership;
 * unrelated epochs can settle without discarding the user's input. */
export function captureResultDraft(row: ResultRow): () => ReadonlyNode | null {
  const source = resolveResultRow(row);
  const page = pageByName(row.page);
  const binding = captureBinding();
  return () => source && bindingCurrent(binding) && resultsActionable()
    && pageByName(row.page) === page && node(source.id) === source ? source : null;
}

/** A query result surface's deferred-control target: `resultWriteTarget` for a
 * result locator, `liveWriteTarget` for a live ancestor/descendant shown with
 * it; undefined outside query results (ordinary source editing). */
export const ResultWriteTargetContext = createContext<(id: string) => (() => string | null) | undefined>(() => undefined);

/** A deferred control opened on a live block shown with a result (e.g. a
 * breadcrumb ancestor) has no result row of its own. Stores merge a same-ID
 * replacement into the existing node, so identity cannot tell; any result
 * change while the control is open refuses instead (invalidate, don't guess).
 * Epoch only, not settled: the user's own typing and saves leave results
 * unsettled without moving the epoch, and must not refuse. Also the query
 * editor's own pickers. */
export function liveWriteTarget(id: string): () => string | null {
  const epoch = resultsEpoch();
  const binding = captureBinding();
  return () => {
    const current = bindingCurrent(binding) && resultsEpoch() === epoch && !!node(id);
    if (!current) noticeChangedResults(1);
    return current ? id : null;
  };
}

/** Deferred controls keep the opening descriptor, and resolve again at commit. */
export function resultWriteTarget(row: ResultRow): () => string | null {
  const opening = { ...row };
  const binding = captureBinding();
  const sourceAtOpen = !opening.dto ? resolveResultRow(opening) : null;
  const pageAtOpen = !opening.dto ? pageByName(opening.page) : null;
  return () => {
    let source = bindingCurrent(binding) ? resolveResultRow(opening) : null;
    if (!opening.dto && (source !== sourceAtOpen || pageByName(opening.page) !== pageAtOpen)) source = null;
    if (!source) noticeChangedResults(1);
    return source?.id ?? null;
  };
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
