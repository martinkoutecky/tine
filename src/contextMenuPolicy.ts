import { isMobilePlatform } from "./nativeChrome";

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
  target: EventTarget | null,
  mobile = isMobilePlatform,
): boolean {
  return !mobile && !editableTarget(target);
}
