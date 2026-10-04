/** Sidebar labels use the native title tooltip already used by Tine's controls.
 * Only an overflowing label exposes its full title. Measurement is O(1) at
 * hover and while a hovered label resizes; no graph reads or persistent state.
 */
import { createEffect, onCleanup, type JSX } from "solid-js";
import { EmojiText } from "../render/emoji";

/** Render an emoji-safe label; horizontal overflow on pointer hover exposes
 * fullTitle (or text). Recheck hovered width/content changes, clear on leave. */
export function SidebarTitle(props: { text: string; fullTitle?: string }): JSX.Element {
  let label!: HTMLSpanElement;
  let hovered = false;
  let observer: ResizeObserver | undefined;
  const refresh = () => {
    label.title = hovered && label.scrollWidth > label.clientWidth ? props.fullTitle ?? props.text : "";
  };
  const leave = () => {
    hovered = false;
    observer?.disconnect();
    observer = undefined;
    label.removeAttribute("title");
  };
  createEffect(() => { props.text; props.fullTitle; if (hovered) refresh(); });
  onCleanup(leave);
  return <span ref={label} class="nav-page-label"
    onMouseEnter={() => {
      hovered = true;
      refresh();
      if (typeof ResizeObserver !== "undefined") {
        observer = new ResizeObserver(refresh);
        observer.observe(label);
      }
    }} onMouseLeave={leave}><EmojiText text={props.text} /></span>;
}
