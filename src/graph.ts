// Opening / switching the active graph from the UI (native folder picker),
// persisting the choice so it reopens next launch.

import { backend } from "./backend";
import { setGraphMeta, setWorkflow, bumpGraphEpoch, setRightSidebar } from "./ui";
import { resetStore } from "./store";
import { openJournals } from "./router";

const GRAPH_KEY = "tine.graphPath";

export function persistedGraphPath(): string {
  try {
    return localStorage.getItem(GRAPH_KEY) ?? "";
  } catch {
    return "";
  }
}

/** Load a graph by path ("" → backend uses env/CLI). Updates meta, persists a
 *  non-empty path, and reloads the views. */
export async function loadGraphPath(path: string): Promise<void> {
  const meta = await backend().loadGraph(path);
  // New graph → drop the old graph's working set and any open sidebar items.
  resetStore();
  setRightSidebar([]);
  setGraphMeta(meta ?? null);
  setWorkflow(meta?.preferred_workflow === "todo" ? "todo" : "now");
  if (path) {
    try {
      localStorage.setItem(GRAPH_KEY, path);
    } catch {
      // ignore
    }
  }
  bumpGraphEpoch();
  openJournals();
}

/** Pick a folder and open it as the graph. No-op if cancelled. */
export async function switchGraph(): Promise<void> {
  const path = await backend().pickFolder();
  if (path) await loadGraphPath(path);
}
