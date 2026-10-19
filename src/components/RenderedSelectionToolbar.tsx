import { For, Show, createEffect, createSignal, onCleanup, untrack, type JSX } from "solid-js";
import { t } from "../i18n";
import { touchPrimary } from "../inputCapability";
import { blurFocusedEditor } from "../editorCommandBridge";
import { editingId, endEdit } from "../editorController";
import type { SelectionAction } from "../editor/selectionActions";
import {
  RENDERED_BLOCK_ATTR,
  applyRenderedSelectionAction,
  renderedSelectionActions,
  renderedSelectionTarget,
  selectRenderedRange,
  type RenderedSelectionTarget,
} from "../editor/renderedSelection";
import { selectionActionLabel } from "./SelectionActionLabel";
import { listen, onEachWindow, selectionOf } from "../windowRealm";

// GH #375 (D-20261018-LP2): on a touch-first device, text selected inside a
// RENDERED block keeps its native selection, handles and menu; this toolbar
// formats it without entering edit mode. One `selectionchange` listener for
// the window, attached only while the primary pointer is a finger, and
// following that answer if it changes after mount. Per event it maps the one
// block that holds the selection (O(block bytes)); nothing runs per block.
//
// A press on a button may clear the native selection before its click arrives
// (the platform owns that). The toolbar therefore keeps the target it last
// read until the press finishes, and an action checks that the block's text is
// still the text the target was read from before it writes.
const PRESS_WINDOW_MS = 700;

export function RenderedSelectionToolbar(): JSX.Element {
  const [target, setTarget] = createSignal<RenderedSelectionTarget | null>(null);
  let pressing = false;
  let pressTimer: ReturnType<typeof setTimeout> | undefined;
  const endPress = () => {
    pressing = false;
    if (pressTimer !== undefined) clearTimeout(pressTimer);
    pressTimer = undefined;
  };
  onCleanup(endPress);

  // The selection's own document: a workspace window owns its selections.
  const refresh = (doc: Document) => {
    let next = renderedSelectionTarget(doc.getSelection());
    if (!next) {
      if (!pressing) setTarget(null);
      return;
    }
    // D-20261018-LP2 (b): a selection in another block ends the edit in
    // progress through the editor's own exit (blur: commit, normalize, save).
    // Its commit may change a copy of the selected block, so read again.
    if (editingId() !== null) {
      const selection = doc.getSelection()!;
      const { anchorNode, anchorOffset, focusNode, focusOffset } = selection;
      if (!blurFocusedEditor()) endEdit("rendered-selection");
      // An engine may drop the document selection with the text control's
      // focus (jsdom does). Put the same range back; the platform's own
      // handles then depend on the engine, as for any script-set selection.
      if ((selection.rangeCount === 0 || selection.isCollapsed) && anchorNode?.isConnected && focusNode?.isConnected) {
        selection.setBaseAndExtent(anchorNode, anchorOffset, focusNode, focusOffset);
      }
      next = renderedSelectionTarget(selection);
    }
    setTarget(next);
  };

  createEffect(() => {
    if (!touchPrimary()) {
      setTarget(null);
      return;
    }
    untrack(() => {
      onCleanup(onEachWindow((win) =>
        listen(win, "selectionchange", () => refresh(win.document), { on: "document" })));
    });
  });

  const run = (action: SelectionAction) => {
    const snapshot = target();
    endPress();
    if (!snapshot) return;
    // The live selection wins when it still lies in a rendered block.
    const doc = snapshot.root.ownerDocument;
    const live = renderedSelectionTarget(selectionOf(snapshot.root)) ?? snapshot;
    const row = live.root.closest(".ls-block");
    const edit = applyRenderedSelectionAction(live, action);
    if (!edit) {
      refresh(doc);
      return;
    }
    const root = live.root.isConnected
      ? live.root
      : Array.from(row?.querySelectorAll<HTMLElement>(`[${RENDERED_BLOCK_ATTR}]`) ?? [])
        .find((el) => el.getAttribute(RENDERED_BLOCK_ATTR) === live.blockId) ?? null;
    if (root && selectRenderedRange(root, live.blockId, edit.start, edit.end, edit.direction ?? live.direction)) {
      refresh(doc);
      return;
    }
    // The result has no byte-exact rendered position (e.g. it is now a page
    // link's label): drop the selection rather than leave a stale one.
    selectionOf(doc)?.removeAllRanges();
    setTarget(null);
  };

  const beginPress = (e: PointerEvent) => {
    // Keep the press from moving focus or the native selection.
    e.preventDefault();
    pressing = true;
    if (pressTimer !== undefined) clearTimeout(pressTimer);
    const doc = (e.currentTarget as Element).ownerDocument;
    pressTimer = setTimeout(() => {
      endPress();
      refresh(doc);
    }, PRESS_WINDOW_MS);
  };

  return (
    <Show when={touchPrimary() && target()}>
      <div
        class="sel-toolbar sel-toolbar-mobile rendered-selection-toolbar"
        role="toolbar"
        aria-label={t("editor-block-selection-formatting")}
        data-rendered-selection-toolbar=""
        onPointerDown={beginPress}
        onMouseDown={(e) => e.preventDefault()}
      >
        <For each={renderedSelectionActions}>{(action) => (
          <button
            type="button"
            classList={{ "sel-action-page-link": action.id === "page-link" }}
            title={action.title}
            aria-label={action.title}
            data-selection-action={action.id}
            onClick={() => run(action)}
          >{selectionActionLabel(action)}</button>
        )}</For>
      </div>
    </Show>
  );
}
