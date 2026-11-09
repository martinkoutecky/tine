// The margin column's contract with the blocks it serves (vision §3.7 "Layout"),
// kept apart from the column itself so Block can read it without importing the
// column (which renders Blocks).

import { createContext, type Accessor } from "solid-js";
import type { CommentChildren } from "../commentChildren";

export interface MarginApi {
  /** Comments are drawn in the margin on this page section now. */
  active: () => boolean;
  /** A main-column block row whose comments the margin draws, while mounted,
   * with the block's comment producer (`createCommentChildren`). */
  register(parentId: string, row: HTMLElement, comments: Accessor<CommentChildren>): () => void;
  /** Ask for one measurement pass in the next frame (coalesced). */
  schedule(): void;
  /** The comment whose thread and quoted passage are emphasised. */
  activeComment: () => string | null;
  setActiveComment(id: string | null): void;
}

/** A thread drawn on a margin, popover or sheet. `order` is the surface's
 * presented root order (`threadOrder`), shared with arrow-key navigation. */
export interface ThreadPlace { root: string; parent: string; order?: () => readonly string[] }

/** Where a block is drawn: the main column (`thread` null) or inside a margin
 * thread (its root comment and the commented block). */
export interface MarginPlacement {
  api: MarginApi;
  thread: ThreadPlace | null;
  /** A locally owned reference/embed surface, rather than an inherited margin. */
  surface?: string;
}

export const MarginContext = createContext<MarginPlacement | null>(null);
