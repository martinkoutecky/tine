import { msg, render } from "../i18n";
import type { KnownGraph } from "../backend";
import type { LoadGraphPathOutcome } from "../graph";
import type { ContextMenuAction } from "../ui";
import { copyTineLink } from "./blockLinkCopy";
import { pushToast } from "../toasts";
import { bindingOwner, writeOwned } from "../owned";
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
  const owner = bindingOwner();
  const open = (newWindow: boolean): void => {
    void writeOwned(owner, deps.openKnown(graph.path, newWindow))
      .catch((error) => reportGraphOpenFailure(error, () => open(newWindow)));
  };
  return [
    ...(deps.desktop ? [{
      get label() { return render(deps.isCurrent ? msg("graph-row-menu-open-in-a-new-window-already-open-here") : msg("graph-row-menu-open-in-a-new-window")); },
      disabled: deps.isCurrent, run: () => open(true),
    }] : []),
    { get label() { return render(deps.isCurrent ? msg("graph-row-menu-open-here-current-graph") : msg("graph-row-menu-open-here")); },
      disabled: deps.isCurrent, run: () => open(false) },
    ...(deps.desktop ? [{ get label() { return render(msg("graph-row-menu-show-in-folder")); }, run: () => {
      void writeOwned(owner, deps.reveal(graph.path)).catch((error) => pushToast(msg("graph-row-menu-could-not-show-the-graph-folder", { error: String(error) }), "error"));
    } }] : []),
    { get label() { return render(msg("graph-row-menu-copy-link")); }, run: () => void copyTineLink({ root: graph.path }) },
    { get label() { return render(msg("graph-row-menu-copy-path")); }, run: () => {
      void writeOwned(owner, deps.copyPath(graph.path)).catch((error) => pushToast(msg("graph-row-menu-could-not-copy-graph-path", { error: String(error) }), "error"));
    } },
    { get label() { return render(msg("graph-row-menu-remove-from-this-list")); }, danger: true, run: () => {
      void writeOwned(owner, deps.forget(graph.path)).catch((error) => pushToast(msg("graph-row-menu-could-not-remove-graph", { error: String(error) }), "error"));
    } },
  ];
}
