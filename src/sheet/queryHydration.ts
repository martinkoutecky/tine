import { backend } from "../backend";
import { ensurePageLoaded, pageByName } from "../store";
import type { RefGroup } from "../types";
import { graphEpoch, graphMeta } from "../ui";

export const SHEET_RENDER_PAGE = 200;
const HYDRATE_CONCURRENCY = 4;
let activeHydrations = 0;
const hydrationQueue: (() => void)[] = [];
const pageHydrations = new Map<string, Promise<void>>();

function runNextHydration() {
  while (activeHydrations < HYDRATE_CONCURRENCY && hydrationQueue.length) {
    activeHydrations++;
    hydrationQueue.shift()!();
  }
}

function limited(task: () => Promise<void>): Promise<void> {
  return new Promise<void>((resolve) => {
    hydrationQueue.push(() => {
      void task().catch(() => {}).finally(() => {
        activeHydrations--;
        resolve();
        runNextHydration();
      });
    });
    runNextHydration();
  });
}

/** Hydrate only pages represented in the currently rendered query-result window.
 * Query DTOs are sufficient for read-only display; full pages are needed only to
 * enable editing. A small worker pool prevents an IPC/file-load stampede. */
export async function hydrateVisibleQueryPages(
  rows: readonly { page: string }[],
  groups: readonly RefGroup[] | undefined,
): Promise<void> {
  const byPage = new Map<string, RefGroup>();
  for (const group of groups ?? []) byPage.set(group.page, group);
  const pending: RefGroup[] = [];
  const seen = new Set<string>();
  for (const row of rows) {
    if (pageByName(row.page) || seen.has(row.page)) continue;
    const group = byPage.get(row.page);
    if (!group) continue;
    seen.add(row.page);
    pending.push(group);
  }
  await Promise.all(pending.map((group) => {
    const epoch = graphEpoch();
    const root = graphMeta()?.root ?? "";
    const key = `${root}\0${epoch}\0${group.kind}\0${group.page}`;
    const existing = pageHydrations.get(key);
    if (existing) return existing;
    const job = limited(async () => {
      if (pageByName(group.page)) return;
      const dto = await backend().getPage(group.page, group.kind);
      if (dto && graphEpoch() === epoch && (graphMeta()?.root ?? "") === root) {
        ensurePageLoaded(dto);
      }
    }).finally(() => pageHydrations.delete(key));
    pageHydrations.set(key, job);
    return job;
  }));
}
