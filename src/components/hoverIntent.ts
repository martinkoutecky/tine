import { onCleanup } from "solid-js";

/** Keep a hover surface alive while the pointer crosses its trigger/menu gap. */
export function hoverIntent(close: () => void) {
  let timer: ReturnType<typeof setTimeout> | undefined;
  const enter = () => { clearTimeout(timer); timer = undefined; };
  const leave = () => { enter(); timer = setTimeout(close, 220); };
  onCleanup(enter);
  return { enter, leave };
}
