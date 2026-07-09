import { createContext } from "solid-js";
import type { PaneRouter } from "./router";

export interface PaneContextValue {
  paneId: string;
  router: PaneRouter;
}

export const PaneContext = createContext<PaneContextValue>();
