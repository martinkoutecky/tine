import { createComputed, createSignal, onCleanup, untrack, type JSX } from "solid-js";
import { t } from "../i18n";
import { childIds, node, onBlockRawChanged, type FeedPage } from "../document";
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
  const [summary, setSummary] = createSignal<PageTaskSummary>({ open: 0, inProgress: 0 });
  const markers = new Map<string, string | null>();
  const count = (marker: string | null): PageTaskSummary => ({
    open: marker && OPEN_MARKERS.has(marker) ? 1 : 0,
    inProgress: marker && IN_PROGRESS_MARKERS.has(marker) ? 1 : 0,
  });
  // Structure observes only membership. Raw writes update one contribution via
  // the document's shared notification, without one computation per block.
  createComputed(() => {
    const page = props.page;
    const format = page.format;
    markers.clear();
    const pending = [...page.roots];
    let open = 0, inProgress = 0;
    while (pending.length) {
      const id = pending.pop()!;
      const block = node(id);
      if (!block) continue;
      pending.push(...childIds(id));
      const marker = untrack(() => facetsOf(block.raw, format).marker);
      markers.set(id, marker);
      const value = count(marker);
      open += value.open; inProgress += value.inProgress;
    }
    setSummary({ open, inProgress });
  });
  onCleanup(onBlockRawChanged(id => {
    if (!markers.has(id)) return;
    const block = node(id);
    if (!block) return; // a structural producer removed it; the walk settles membership
    const marker = facetsOf(block.raw, props.page.format).marker;
    const previous = markers.get(id)!;
    if (marker === previous) return;
    markers.set(id, marker);
    const before = count(previous), after = count(marker);
    setSummary(value => ({ open: value.open + after.open - before.open,
      inProgress: value.inProgress + after.inProgress - before.inProgress }));
  }));
  const tasks = () => t("journals-prose-tasks-today", { count: summary().open });
  const progress = () => t("journals-prose-in-progress", { count: summary().inProgress });

  return (
    <div class="today-task-summary" aria-label={t("journals-prose-task-summary", { tasks: tasks(), progress: progress() })}>
      <span>{tasks()}</span>
      <span aria-hidden="true">, </span>
      <span>{progress()}</span>
    </div>
  );
}
