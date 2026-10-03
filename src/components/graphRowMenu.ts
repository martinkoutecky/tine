import type { KnownGraph } from "../backend";
import type { LoadGraphPathOutcome } from "../graph";
import type { ContextMenuAction } from "../ui";
import { pushToast } from "../toasts";
import { graphOwner, writeOwned } from "../owned";
import { reportGraphOpenFailure } from "../graphOpenFailure";

export interface GraphRowMenuDeps {
  openKnown(path: string, newWindow: boolean): Promise<LoadGraphPathOutcome>;
  reveal(path: string): Promise<void>;
  copyPath(path: string): Promise<void>;
  forget(path: string): Promise<void>;
  desktop: boolean;
  isCurrent: boolean;
}

/** Actions for one remembered graph. Current-graph opens remain visible with
 * a reason but cannot run; mobile omits OS and peer-window actions. Each
 * operation reports its own failure. Cost: O(1), no I/O until an action runs. */
export function graphRowMenuActions(graph: KnownGraph, deps: GraphRowMenuDeps): ContextMenuAction[] {
  const owner = graphOwner();
  const open = (newWindow: boolean): void => {
    void writeOwned(owner, deps.openKnown(graph.path, newWindow))
      .catch((error) => reportGraphOpenFailure(error, () => open(newWindow)));
  };
  return [
    ...(deps.desktop ? [{
      label: deps.isCurrent ? "Open in a new window (already open here)" : "Open in a new window",
      disabled: deps.isCurrent, run: () => open(true),
    }] : []),
    { label: deps.isCurrent ? "Open here (current graph)" : "Open here",
      disabled: deps.isCurrent, run: () => open(false) },
    ...(deps.desktop ? [{ label: "Show in folder", run: () => {
      void writeOwned(owner, deps.reveal(graph.path)).catch((error) => pushToast(`Could not show the graph folder: ${String(error)}`, "error"));
    } }] : []),
    { label: "Copy path", run: () => {
      void writeOwned(owner, deps.copyPath(graph.path)).catch((error) => pushToast(`Could not copy graph path: ${String(error)}`, "error"));
    } },
    { label: "Remove from this list", danger: true, run: () => {
      void writeOwned(owner, deps.forget(graph.path)).catch((error) => pushToast(`Could not remove graph: ${String(error)}`, "error"));
    } },
  ];
}
