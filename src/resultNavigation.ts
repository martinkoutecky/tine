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
