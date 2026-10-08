import { createMemo, type JSX } from "solid-js";
import { childIds, node, type FeedPage } from "../document";
import { facetsOf } from "../render/facets";
import { OPEN_MARKERS } from "../markers";

/** Theme API 0.2 compact today-summary (master 1488588b8, ADR 0059). */
const IN_PROGRESS_MARKERS: ReadonlySet<string> = new Set([
  "DOING",
  "NOW",
  "STARTED",
  "IN-PROGRESS",
]);

export interface PageTaskSummary {
  open: number;
  inProgress: number;
}

/** Count open and in-progress task markers in the already-loaded page tree.
 * Facets come from the shared facet cache, so this adds no graph scan, IPC or
 * second task parser. O(blocks on the page). */
export function summarizePageTasks(page: FeedPage): PageTaskSummary {
  let open = 0;
  let inProgress = 0;
  const pending = [...page.roots];
  while (pending.length > 0) {
    const id = pending.pop()!;
    const block = node(id);
    if (!block) continue;
    pending.push(...childIds(id));
    const marker = facetsOf(block.raw, page.format).marker;
    if (!marker || !OPEN_MARKERS.has(marker)) continue;
    open += 1;
    if (IN_PROGRESS_MARKERS.has(marker)) inProgress += 1;
  }
  return { open, inProgress };
}

/** "N tasks today, M in progress" for today's journal title row; shown only
 * while the style theme selects `todayTaskSummary: "compact"`. */
export function TodayTaskSummary(props: { page: FeedPage }): JSX.Element {
  const summary = createMemo(() => summarizePageTasks(props.page));
  const tasks = () => `${summary().open} ${summary().open === 1 ? "task" : "tasks"} today`;
  const progress = () => `${summary().inProgress} in progress`;

  return (
    <div class="today-task-summary" aria-label={`${tasks()}, ${progress()}`}>
      <span>{tasks()}</span>
      <span aria-hidden="true">, </span>
      <span>{progress()}</span>
    </div>
  );
}
