import { isMobilePlatform } from "./nativeChrome";
import { isLongPressContextMenu } from "./render/longPress";

function editableTarget(target: EventTarget | null): boolean {
  const element = target instanceof Element ? target : null;
  return !!element?.closest("textarea,input,select,[contenteditable='true']");
}

/** Ordinary block rows keep desktop right-click, but Android long-press may
 * open Tine's block menu only from the explicit bullet affordance. */
export function shouldOpenBlockContextMenu(
  target: EventTarget | null,
  mobile = isMobilePlatform,
): boolean {
  if (editableTarget(target)) return false;
  const element = target instanceof Element ? target : null;
  return !mobile || !!element?.closest(".bullet-container");
}

/** Inline/page/reference text owns no explicit mobile menu affordance. Android
 * WebView uses `contextmenu` to begin text selection, so leave the event wholly
 * native there. Desktop right-click remains unchanged. */
export function shouldOpenTextContextMenu(
  targetOrEvent: EventTarget | MouseEvent | null,
  mobile = isMobilePlatform,
): boolean {
  const event = targetOrEvent instanceof MouseEvent ? targetOrEvent : null;
  const target: EventTarget | null = event
    ? event.target
    : targetOrEvent as EventTarget | null;
  if (event && isLongPressContextMenu(event)) return !editableTarget(target);
  return !mobile && !editableTarget(target);
}

/** Explicit page-navigation affordances own a mobile hold just like Sidebar
 * rows do. Ordinary block/editor text still uses shouldOpenTextContextMenu so
 * Android's native selection remains available away from links. */
export function shouldOpenPageContextMenu(
  target: EventTarget | null,
): boolean {
  return !editableTarget(target);
}
