/** Outline-guide rule: expand if any item is folded, otherwise collapse. */
export function bulkCollapseTarget<T>(items: readonly T[], collapsed: (item: T) => boolean): boolean {
  return !items.some(collapsed);
}
