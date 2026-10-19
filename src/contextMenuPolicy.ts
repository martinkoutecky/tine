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
