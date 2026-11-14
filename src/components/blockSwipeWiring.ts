import { createEffect, onCleanup, untrack } from "solid-js";
import { attachBlockSwipe, swipeDisabledByTags, type BlockSwipeAction } from "../blockSwipe";
import { graphMeta } from "../graphSession";
import { pageIdentityKey } from "../pageIdentity";
import { block_reference_names } from "../render/wasm/lsdoc_wasm.js";
import { canIndentBlock, childIds, pageRoots, nextVisible, clearSelection, indentSelection, node as docNode, outdentSelection, pageByName, selectBlock } from "../document";
import type { OutlineScope } from "../document";
import { dispatchFocusedEditorCommand, focusedEditorCommandBridge } from "../editorCommandBridge";
import { touchGestures } from "../inputCapability";
import type { Format } from "../render/ast";
import { openContextMenu } from "../ui";
import { threadRootOf } from "../document";

/** A block's own refs for OG's `:mobile :gestures/disabled-in-block-with-tags`,
 *  case-folded: OG's `with-page-refs` joins headline tags, references, property
 *  keys, the task marker and the priority. That is the reference evidence's one
 *  answer (`reference_evidence/walk.rs`, compiled into wasm): one parse of `raw`
 *  plus one per property value or macro argument. */
export function swipeRefNames(raw: string, format: Format): string[] {
  return [...new Set(block_reference_names(raw, format === "org").map(pageIdentityKey))];
}

export interface BlockRowSwipeDeps {
  /** The block the row's position belongs to: a whole-block embed's root row
   *  stands for its host (D-20261030-EMB). */
  id: string;
  /** The block whose editor this row mounts, when it differs from `id`. */
  editorId?: string;
  scope: OutlineScope | null | undefined;
  surface?: string;
  editing(): boolean;
  readOnly(): boolean;
}

/** The block-row swipe for the current owner, following the input capability:
 *  attached while a touch input exists, detached (every listener and any cue
 *  removed) when it goes away, re-attached when it returns (D-20261018-W1: a
 *  touch laptop or convertible changes mode while the row stays mounted). One
 *  computation per row, as the former mount-time wiring was. */
export function followBlockSwipe(row: HTMLElement, deps: BlockRowSwipeDeps): void {
  createEffect(() => {
    if (!touchGestures()) return;
    untrack(() => onCleanup(wireBlockSwipe(row, deps)));
  });
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
 *  Cue availability uses the existing document reads: previous sibling and
 *  the kernel's indent verb (canIndentBlock) for indent, parent/scope-root for
 *  outdent. The first revealed indent cue costs O(page blocks + visible scope);
 *  cached for that gesture.
 *  Returns the cleanup; a no-op without a touch input. */
export function wireBlockSwipe(row: HTMLElement, deps: BlockRowSwipeDeps): () => void {
  return attachBlockSwipe(row, {
    enabled: touchGestures(),
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
          return swipeRefNames(block.raw, pageByName(block.page)?.format ?? "md");
        },
      ),
    editing: deps.editing,
    cueAllowed(action) {
      if (deps.readOnly()) return false;
      const block = docNode(deps.id);
      if (!block || pageByName(block.page)?.guide) return false;
      if (deps.editing() && focusedEditorCommandBridge()?.blockId !== (deps.editorId ?? deps.id)) return false;
      if (action === "actions") return true;
      if (threadRootOf(deps.id) !== null) return false;
      if (action === "outdent") return block.parent !== null && deps.scope?.forceExpandedRoot !== block.parent;
      const siblings = block.parent === null ? pageRoots(block.page) : childIds(block.parent);
      const index = siblings.indexOf(deps.id);
      if (index <= 0 || (deps.scope && nextVisible(siblings[index - 1], deps.scope) === null)) return false;
      return canIndentBlock(deps.id);
    },
    run(action: BlockSwipeAction, x: number, y: number) {
      if (deps.readOnly()) return;
      if (deps.editing()) {
        if (focusedEditorCommandBridge()?.blockId !== (deps.editorId ?? deps.id)) return;
        if (action === "indent") dispatchFocusedEditorCommand("editor/indent");
        else if (action === "outdent") dispatchFocusedEditorCommand("editor/outdent");
        // The action menu from an editing row opens on the same block; the
        // editor stays as it is (the menu is a transient layer above it).
        else openContextMenu(x, y, deps.id);
        return;
      }
      if (action === "actions") {
        selectBlock(deps.id, deps.scope ?? null, deps.surface);
        openContextMenu(x, y, deps.id);
        return;
      }
      selectBlock(deps.id, deps.scope ?? null, deps.surface);
      if (action === "indent") indentSelection();
      else outdentSelection();
      clearSelection();
    },
  });
}
