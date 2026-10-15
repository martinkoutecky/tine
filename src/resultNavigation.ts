/** Shared quick-switcher/Search arrow pattern. -1 means input focus, before
 * entering the rows; thereafter arrows wrap in displayed order. */
export function nextResultIndex(current: number, direction: number, count: number): number {
  if (!count) return -1;
  if (current < 0) return direction > 0 ? 0 : count - 1;
  return (current + direction + count) % count;
}
