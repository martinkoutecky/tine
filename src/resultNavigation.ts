/** Shared quick-switcher/Search arrow pattern. -1 means input focus, before
 * entering the rows; thereafter arrows wrap in displayed order. */
export function nextResultIndex(current: number, direction: number, count: number): number {
  if (!count) return -1;
  if (current < 0) return direction > 0 ? 0 : count - 1;
  return (current + direction + count) % count;
}

/** Search-tab pattern, where focus moves into the rows: the input sits above
 * the first row and the ends do not wrap. -1 means the input; `null` means
 * stay where focus is (GH #697 follow-up). */
export function steppedResultIndex(current: number, direction: number, count: number): number | null {
  if (!count) return null;
  const next = Math.max(-1, Math.min(count - 1, current + direction));
  return next === current ? null : next;
}

/** The step a bare ArrowUp/ArrowDown asks of a focused result list (an element
 * inside `[data-result-list]`: a search box or its results), or 0 when the
 * key is not one or its target is outside such a list (a modified arrow stays
 * with the global shortcuts, composition with the IME). The global map leaves a
 * nonzero step to the list, so it never also moves a block, cell or pane selection
 * behind it (GH #996, #1007). */
export function resultListStep(event: KeyboardEvent): -1 | 0 | 1 {
  if (event.isComposing || event.keyCode === 229 || event.ctrlKey || event.altKey || event.metaKey || event.shiftKey) return 0;
  const direction = event.key === "ArrowDown" ? 1 : event.key === "ArrowUp" ? -1 : 0;
  if (!direction) return 0;
  const target = event.target as Element | null;
  return typeof target?.closest === "function" && target.closest("[data-result-list]") ? direction : 0;
}
