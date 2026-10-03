import { attachBlockSwipe, swipeDisabledByTags, type BlockSwipeAction } from "../blockSwipe";
import { graphMeta } from "../graphSession";
import { pageIdentityKey } from "../pageIdentity";
import { pageRefsInText } from "../render/pageRefs";
import { clearSelection, indentSelection, node as docNode, outdentSelection, pageByName, selectBlock } from "../document";
import type { OutlineScope } from "../document";
import { dispatchFocusedEditorCommand, focusedEditorCommandBridge } from "../editorCommandBridge";
import { touchGesturePlatform } from "../nativeChrome";
import { openContextMenu } from "../ui";

export interface BlockRowSwipeDeps {
  id: string;
  scope: OutlineScope | null | undefined;
  editing(): boolean;
  readOnly(): boolean;
}

/** Runs a block-row swipe through the commands the keyboard and the selection
 *  toolbar already use, so it is undoable and obeys the same refusals
 *  (read-only pages, first sibling, depth limit, scope roots). Indent/outdent
 *  while the block is being edited go through the focused editor's own
 *  `editor/indent` / `editor/outdent` (commit-then-move, caret kept); on a
 *  resting row the block is selected, moved by `indentSelection` /
 *  `outdentSelection` and deselected. The OG "action bar" is Tine's block
 *  context menu opened at the release point on the selected block.
 *
 *  Returns the cleanup; a no-op off touch platforms. */
export function wireBlockSwipe(row: HTMLElement, deps: BlockRowSwipeDeps): () => void {
  return attachBlockSwipe(row, {
    platform: touchGesturePlatform(),
    // OG `:mobile :gestures/disabled-in-block-with-tags`, read at touchstart so
    // a config edit applies to the next touch. Refs come from the one lsdoc
    // parse of each ancestor block (never a regex over raw text).
    disabledByTags: (el) =>
      swipeDisabledByTags(
        el,
        (graphMeta()?.mobile_gestures_disabled_in_block_with_tags ?? []).map(pageIdentityKey),
        (blockId) => {
          const block = docNode(blockId);
          if (!block) return [];
          return pageRefsInText(block.raw, pageByName(block.page)?.format ?? "md").map(pageIdentityKey);
        },
      ),
    editing: deps.editing,
    run(action: BlockSwipeAction, x: number, y: number) {
      if (deps.readOnly()) return;
      if (deps.editing()) {
        if (focusedEditorCommandBridge()?.blockId !== deps.id) return;
        if (action === "indent") dispatchFocusedEditorCommand("editor/indent");
        else if (action === "outdent") dispatchFocusedEditorCommand("editor/outdent");
        // The action menu from an editing row opens on the same block; the
        // editor stays as it is (the menu is a transient layer above it).
        else openContextMenu(x, y, deps.id);
        return;
      }
      if (action === "actions") {
        selectBlock(deps.id, deps.scope ?? null);
        openContextMenu(x, y, deps.id);
        return;
      }
      selectBlock(deps.id, deps.scope ?? null);
      if (action === "indent") indentSelection();
      else outdentSelection();
      clearSelection();
    },
  });
}
