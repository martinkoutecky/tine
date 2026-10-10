// The live comparison of one tracked page against its seen baseline (ADR 0073).
// Cost model (D-10: the cost of an edit scales with the edit):
//   - creating it hashes every block once (one memo per block);
//   - a keystroke re-runs only the edited block's memo: one hash, and the
//     count moves by at most one;
//   - a structural edit (new, deleted or moved block) re-walks the outline's
//     structure, O(page) id comparisons and no hashing of unchanged blocks;
//   - a new baseline (Mark seen) re-compares every block, O(page).
import { createComputed, createMemo, createRoot, createSignal, mapArray, onCleanup, type Accessor } from "solid-js";
import { createStore } from "solid-js/store";
import { loadedPage, node } from "../document";
import { pageBlockIds, seenBlockHash } from "./hash";

export interface SeenTracker {
  /** Is this block's own content absent from the baseline? */
  changed(id: string): boolean;
  /** How many of the page's blocks are changed. */
  count: Accessor<number>;
  dispose(): void;
}

/** Compare `pageName`'s blocks with `baseline()` until disposed. A block is
 *  changed when its hash is not in the baseline: new and edited blocks are
 *  changed, a moved unchanged block is not, and a deleted block simply leaves
 *  the page (an edit also removes its old content, so deletions are not
 *  counted: the hash set cannot tell the two apart). */
export function createSeenTracker(pageName: string, baseline: Accessor<ReadonlySet<string>>): SeenTracker {
  return createRoot((dispose) => {
    const [changedById, setChangedById] = createStore<Record<string, true | undefined>>({});
    const [count, setCount] = createSignal(0);
    const format = createMemo(() => loadedPage(pageName)?.format ?? "md");
    const ids = createMemo(() => pageBlockIds(pageName));
    const rows = mapArray(ids, (id) => {
      const hash = createMemo(() => {
        const current = node(id);
        return current ? seenBlockHash(current.raw, format()) : null;
      });
      createComputed(() => {
        const value = hash();
        if (value === null || baseline().has(value)) return;
        setChangedById(id, true);
        setCount((n) => n + 1);
        onCleanup(() => {
          setChangedById(id, undefined);
          setCount((n) => n - 1);
        });
      });
    });
    // Materialize the per-block rows; mapArray only runs while something reads it.
    createComputed(rows);
    return { changed: (id: string) => !!changedById[id], count, dispose };
  });
}
