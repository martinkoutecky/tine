import { createMemo, type JSX } from "solid-js";
import { collapsibleDescendantIds, node as docNode, setCollapsedDescendants } from "../../document";
import type { CollapseSurfaceApi } from "../Block";
export { seenRowClasses } from "../../seen/SeenHeader";

// Row chrome of `Block` that only some rows need, split out so `Block.tsx` (over
// the 1,500-line ceiling) does not grow (og I3, master 0350c00b6 / ce9a796fb).

export interface ThreadLineDecoration {
  enabled: boolean;
  active: boolean;
  standard: boolean;
}
export const NO_THREAD_LINES: ThreadLineDecoration = { enabled: false, active: false, standard: false };

/** The plugin thread-lines classes of the `.ls-block` row. */
export function rowDecorationClasses(threadLines: ThreadLineDecoration): Record<string, boolean> {
  return {
    "plugin-thread-lines": threadLines.enabled,
    "plugin-thread-lines-active": threadLines.active,
    "plugin-thread-lines-standard": threadLines.standard,
  };
}

/** The children container's "collapse/expand every descendant" left border. Its
 *  two derivations (the descendant list and whether any of them is folded) live
 *  HERE rather than in `Block`, so a leaf never allocates them and the subtree
 *  walk stays memoized only for containers that render this control. */
export function CollapseAllBorder(props: { id: string; readOnly: boolean; surface: CollapseSurfaceApi | null }): JSX.Element {
  const collapseSurface = props.surface;
  const collapsibleDescendants = createMemo(() => collapsibleDescendantIds(props.id));
  const hasCollapsedDescendant = createMemo(() =>
    collapsibleDescendants().some((id) => {
      const descendant = docNode(id);
      return descendant
        ? collapseSurface?.collapsed(id, descendant.collapsed) ?? descendant.collapsed
        : false;
    })
  );
  const toggleCollapsedDescendants = () => {
    const ids = collapsibleDescendants();
    if (!ids.length || (props.readOnly && !collapseSurface)) return;
    // OG semantics: any folded descendant means “expand all”; only a completely
    // open subtree means “collapse all”. The guide parent itself stays open.
    const next = !hasCollapsedDescendant();
    if (collapseSurface) collapseSurface.setMany(ids, next);
    else setCollapsedDescendants(props.id, next);
  };
  return (
    <button
      type="button"
      class="block-children-left-border"
      aria-label={hasCollapsedDescendant() ? "Expand all descendants" : "Collapse all descendants"}
      aria-expanded={!hasCollapsedDescendant()}
      disabled={collapsibleDescendants().length === 0 || (props.readOnly && !collapseSurface)}
      title={hasCollapsedDescendant() ? "Expand all descendants" : "Collapse all descendants"}
      onClick={(event) => {
        event.stopPropagation();
        toggleCollapsedDescendants();
      }}
    />
  );
}
