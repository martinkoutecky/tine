import { createContext, type Accessor, type JSX } from "solid-js";

export interface SheetContainerOverlay {
  hovering: Accessor<boolean>;
  setCorner: (node: JSX.Element | null) => void;
}

export const SheetContainerOverlayContext = createContext<SheetContainerOverlay | null>(null);
