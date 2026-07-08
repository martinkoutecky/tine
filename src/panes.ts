import { createContext, createSignal } from "solid-js";
import {
  createPaneRouter,
  installPaneRouterRegistry,
  mainPaneRouter,
  type PaneRouter,
} from "./router";
import { registerPaneFocusSetter } from "./ui";

export type LayoutNode =
  | {
      kind: "split";
      dir: "row" | "col";
      ratio: number;
      children: [LayoutNode, LayoutNode];
    }
  | { kind: "pane"; paneId: string };

export interface PaneContextValue {
  paneId: string;
  router: PaneRouter;
}

export const [layoutRoot, setLayoutRoot] = createSignal<LayoutNode>({
  kind: "pane",
  paneId: "main",
});
export const [focusedPaneId, setFocusedPaneId] = createSignal("main");

const routers = new Map<string, PaneRouter>([["main", mainPaneRouter]]);

export function paneRouter(paneId: string): PaneRouter {
  const existing = routers.get(paneId);
  if (existing) return existing;
  const router = createPaneRouter();
  routers.set(paneId, router);
  return router;
}

export function mainRouter(): PaneRouter {
  return paneRouter("main");
}

export function focusedRouter(): PaneRouter {
  const paneId = focusedPaneId();
  return paneId === "pdf" ? mainRouter() : paneRouter(paneId);
}

export const PaneContext = createContext<PaneContextValue>();

installPaneRouterRegistry({ focusedRouter, mainRouter });
registerPaneFocusSetter(setFocusedPaneId);
