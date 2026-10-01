import type { DocState } from "../model";

/** Delete the detached subtree inside the caller's produce transaction. O(subtree nodes). */
export function removeSubtree(state: DocState, id: string): void {
  for (const child of state.byId[id].children) removeSubtree(state, child);
  delete state.byId[id];
}
