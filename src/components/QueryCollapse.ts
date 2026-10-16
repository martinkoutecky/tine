import { blockWritable, collapseEpochOf, node, setCollapsedDeep, withUndoUnit } from "../document";
import { noticeChangedResults, resolveResultRow } from "./queryResultIdentity";
import { loadSourcePage } from "../sourcePage";
import { graphOwner } from "../owned";
import { t } from "../i18n";
import { pushToast } from "../toasts";
import type { BlockDto, RefGroup } from "../types";

/** Read current source folds, including Undo; unopened sources use their answer.
 * This does not hydrate pages just to choose a menu label. */
export function queryResultsHaveCollapsed(groups: RefGroup[]): boolean {
  for (const group of groups) for (const block of group.blocks) {
    if (block.page_property) continue;
    const source = resolveResultRow({ id: block.id, dto: block, page: group.page, kind: group.kind, path: group.path }, "mount");
    if (source) {
      const pending = [source.id];
      const seen = new Set<string>();
      while (pending.length) {
        const id = pending.pop()!;
        if (seen.has(id)) continue;
        seen.add(id);
        const current = node(id);
        if (!current?.children.length) continue;
        if (current.collapsed) return true;
        pending.push(...current.children);
      }
    } else {
      const pending: BlockDto[] = [block];
      while (pending.length) {
        const current = pending.pop()!;
        if (current.children.length && current.collapsed) return true;
        pending.push(...current.children);
      }
    }
  }
  return false;
}

/** Collapse is an ordinary source edit, never a new query property or writer. */
export async function collapseQueryResults(groups: RefGroup[], collapsed: boolean): Promise<ReadonlyMap<string, number> | null> {
  const owner = graphOwner();
  for (const group of groups) {
    if (!owner() || !await loadSourcePage({ page: group.page, pageKind: group.kind, path: group.path }, t("query-calm-fold-failed"))) return null;
  }
  if (!owner()) return null;
  const selected = new Set<string>();
  let changed = 0;
  for (const group of groups) {
    for (const block of group.blocks.filter(block => !block.page_property)) {
      const source = resolveResultRow({ id: block.id, dto: block, page: group.page, kind: group.kind, path: group.path });
      if (!source) changed++;
      else selected.add(source.id);
    }
  }
  noticeChangedResults(changed);
  if (changed) return null;
  const roots = [...selected].filter(id => {
    if (!node(id)?.children.length) return false;
    let parent = node(id)?.parent;
    while (parent) { if (selected.has(parent)) return false; parent = node(parent)?.parent; }
    return true;
  });
  if (roots.some(id => !blockWritable(id))) { pushToast(t("query-calm-fold-failed"), "error"); return null; }
  if (!roots.length) return new Map();
  const pages = [...new Set(roots.map(id => node(id)!.page))];
  const saved = withUndoUnit("query:collapse-results", pages, () => { for (const id of roots) setCollapsedDeep(id, collapsed); return true; });
  if (!saved) return null;
  const epochs = new Map<string, number>();
  const pending = [...roots];
  while (pending.length) {
    const id = pending.pop()!;
    if (epochs.has(id)) continue;
    epochs.set(id, collapseEpochOf(id)); pending.push(...node(id)?.children ?? []);
  }
  return epochs;
}
