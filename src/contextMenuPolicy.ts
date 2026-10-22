import { touchPrimary } from "./inputCapability";
import { isLongPressContextMenu } from "./render/longPress";
import { isElementNode, isElementTag, isMouseEventValue } from "./windowRealm";

function editableTarget(target: EventTarget | null): boolean {
  const element = isElementNode(target) ? target : null;
  return !!element?.closest("textarea,input,select,[contenteditable='true']");
}

/** Desktop code editors with a caret offer the existing block actions; selected
 * text retains its native menu. Ordinary rows keep right-click, but on a
 * touch-first device (`touch`: the primary pointer is a finger, D-20261018-W1)
 * a long-press may open Tine's block menu only from the explicit bullet
 * affordance (a comment block has no bullet: its author chip is the handle). */
export function shouldOpenBlockContextMenu(
  target: EventTarget | null,
  touch = touchPrimary(),
): boolean {
  const mobile = touch;
  if (!mobile && isElementTag(target, "textarea") && target.classList.contains("code-edit") && target.selectionStart === target.selectionEnd) return true;
  if (editableTarget(target)) return false;
  const element = isElementNode(target) ? target : null;
  return !mobile || !!element?.closest(".bullet-container, .comment-handle");
}

/** Inline/page/reference text owns no explicit touch menu affordance. Android
 * WebView uses `contextmenu` to begin text selection, so with a finger as the
 * primary pointer leave the event wholly native. A mouse or trackpad
 * right-click (any device) remains unchanged. */
export function shouldOpenTextContextMenu(
  targetOrEvent: EventTarget | MouseEvent | null,
  touch = touchPrimary(),
): boolean {
  const mobile = touch;
  const event = isMouseEventValue(targetOrEvent) ? targetOrEvent : null;
  const target: EventTarget | null = event
    ? event.target
    : targetOrEvent as EventTarget | null;
  if (event && isLongPressContextMenu(event)) return !editableTarget(target);
  return !mobile && !editableTarget(target);
}

/** Marks an element that draws OTHER blocks inside a host block's content (a
 * query's results, an embed). A block-menu gesture inside it belongs to the
 * block row drawn there, or to nobody, never to the host (GH #726: a right
 * click on a query result opened the host's menu, so Cut deleted the query). */
export const BLOCK_SURFACE_ATTR = "data-block-surface";
const BLOCK_MENU_OWNER = `.block-main, .reference-excerpt-row, [${BLOCK_SURFACE_ATTR}]`;

/** True when `row` (a block's own `.block-main`) is the nearest block-menu
 * owner of `target`: no nested block row or foreign block surface lies between
 * them. O(depth) ancestor walk. */
export function ownsBlockContextMenu(target: EventTarget | null, row: Element | undefined): boolean {
  if (!row) return false;
  const element = isElementNode(target) ? target : null;
  return !!element && element.closest(BLOCK_MENU_OWNER) === row;
}
