// The margin column's contract with the blocks it serves (vision §3.7 "Layout"),
// kept apart from the column itself so Block can read it without importing the
// column (which renders Blocks).

import { createContext } from "solid-js";

export interface MarginApi {
  /** Comments are drawn in the margin on this page section now. */
  active: () => boolean;
  /** A main-column block row whose comments the margin draws, while mounted. */
  register(parentId: string, row: HTMLElement): () => void;
  /** Ask for one measurement pass in the next frame (coalesced). */
  schedule(): void;
  /** The comment whose thread and quoted passage are emphasised. */
  activeComment: () => string | null;
  setActiveComment(id: string | null): void;
}

/** Where a block is drawn: the main column (`thread` null) or inside a margin
 * thread (its root comment and the commented block). */
export interface MarginPlacement {
  api: MarginApi;
  thread: { root: string; parent: string } | null;
}

export const MarginContext = createContext<MarginPlacement | null>(null);
