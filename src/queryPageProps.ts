import { backend } from "./backend";
import { ensurePageLoaded, pageByName, reportPageLoadRefusal } from "./document";
import { graphOwner, readOwned } from "./owned";
import { openPageProps } from "./ui";
import { pushToast } from "./toasts";
import type { PageKind } from "./types";

/** GH #619 item 8: edit a page result's properties from its row. The row only carries what the
 *  query answered, so the page is first LOADED into the working set (`ensurePageLoaded`, a read;
 *  an already-loaded page keeps its unsaved edits), and the existing properties panel then writes
 *  through `setPageProperty`, the one guarded page-property write path (base-revision guard,
 *  undo, normal save). This module adds no write of its own. One page read; stale after a graph
 *  switch. */
export async function openPagePropertiesFromRow(row: { name: string; kind: PageKind }, x: number, y: number): Promise<void> {
  const owner = graphOwner();
  if (!pageByName(row.name)) {
    const result = await readOwned(owner, backend().getPage(row.name, row.kind));
    if (result.kind === "stale") return;
    if (!result.value) {
      pushToast(`"${row.name}" is no longer there, so its properties cannot be edited.`, "error");
      return;
    }
    const refusal = ensurePageLoaded(result.value);
    if (refusal) {
      reportPageLoadRefusal(refusal, "Its properties were not opened.");
      return;
    }
  }
  if (!pageByName(row.name)) return;
  openPageProps(row.name, x, y);
}
