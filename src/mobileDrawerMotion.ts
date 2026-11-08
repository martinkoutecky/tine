import { createEffect, createSignal, onCleanup } from "solid-js";
import { mobileDrawerMode, type DrawerSide } from "./mobileDrawers";
import { rootIsRtl } from "./render/textDirection";
import { windowOf } from "./windowRealm";

// OG 6e7afa8eb, container.css:78,394,428: 300ms ease transform,
// 400ms ease scrim. Intentional differences: both edges use transform (OG's
// right sidebar animates width); close state/navigation change immediately;
// closed surfaces unmount, and Reduce Motion skips every animation.
export const DRAWER_SLIDE_MS = 300;
export const DRAWER_FADE_MS = 400;
type Surface = DrawerSide | "scrim";
// Exactly three slots, released with their Solid owner; never graph-keyed state.
const drags: Partial<Record<Surface, { drag(distance: number): void; release(): void; width(): number }>> = {};

export function dragMobileDrawer(side: DrawerSide, distance: number) {
  const panel = drags[side];
  if (!panel) return;
  panel.drag(distance);
  drags.scrim?.drag(Math.min(1, Math.max(0, distance / panel.width())));
}
export function dragOpeningMobileDrawer(distance: number) {
  const width = drags.left?.width();
  if (width) dragMobileDrawer("left", Math.max(0, width - distance));
}
export function settleMobileDrawer(side: DrawerSide) {
  drags[side]?.release();
  drags.scrim?.release();
}

/** Presence belongs to the animated shell, never to the visibility store. The
 * store changes at once; the inert departing surface lives for at most one
 * animation, then Solid disposes its content and subscriptions. */
export function createDrawerMotion(open: () => boolean, surface: Surface) {
  const [present, setPresent] = createSignal(open());
  const [element, setElement] = createSignal<HTMLElement>();
  const [reduced, setReduced] = createSignal(false);
  createEffect(() => {
    const el = element();
    if (!el) return;
    const query = windowOf(el).matchMedia?.("(prefers-reduced-motion: reduce)");
    const update = () => setReduced(query?.matches ?? false);
    update();
    query?.addEventListener?.("change", update);
    onCleanup(() => query?.removeEventListener?.("change", update));
  });
  const property = surface === "scrim" ? "opacity" : "transform";
  const duration = surface === "scrim" ? DRAWER_FADE_MS : DRAWER_SLIDE_MS;
  const closed = () => surface === "scrim" ? "0" : `translateX(${(surface === "left" ? -1 : 1) * (rootIsRtl() ? -1 : 1) * 100}%)`;
  const opened = surface === "scrim" ? "1" : "translateX(0px)";
  let animation: Animation | undefined;
  let timer: ReturnType<typeof setTimeout> | undefined;
  let previous: HTMLElement | undefined;
  let generation = 0;
  const cancel = () => {
    generation++;
    animation?.cancel(); animation = undefined;
    if (timer) clearTimeout(timer);
    timer = undefined;
  };
  const animate = (fresh = false) => {
    const el = element();
    if (!el) return;
    const visible = open();
    // Sample before cancelling so a reversal/drag settles from its current
    // visual position rather than jumping back to either endpoint.
    const value = windowOf(el).getComputedStyle(el)[property];
    const from = fresh && visible ? closed() : value === "none" || !value ? opened : value;
    cancel();
    el.style[property] = "";
    if (!mobileDrawerMode() || reduced() || typeof el.animate !== "function") {
      if (!visible) setPresent(false);
      return;
    }
    const token = generation;
    const finish = () => {
      if (token !== generation) return;
      cancel();
      if (!open()) setPresent(false);
    };
    animation = el.animate([{ [property]: from }, { [property]: visible ? opened : closed() }], { duration, easing: "ease", fill: "both" });
    animation.onfinish = finish;
    // WebViews may suppress finish events when a window becomes hidden.
    timer = setTimeout(finish, duration + 32);
  };
  createEffect(() => { if (open()) setPresent(true); });
  createEffect(() => {
    open(); mobileDrawerMode(); reduced();
    const el = element();
    if (!el) { cancel(); return; }
    animate(el !== previous); previous = el;
  });
  const drag = {
    width: () => element()?.offsetWidth || 1,
    drag(distance: number) {
      const el = element();
      if (!el || !open() || reduced()) return;
      cancel();
      el.style[property] = surface === "scrim" ? String(1 - distance)
        : `translateX(${Math.min(el.offsetWidth, Math.max(0, distance)) * (surface === "left" ? -1 : 1) * (rootIsRtl() ? -1 : 1)}px)`;
    },
    release: () => animate(),
  };
  drags[surface] = drag;
  onCleanup(() => { cancel(); if (drags[surface] === drag) delete drags[surface]; });
  return { present, ref: (el: HTMLElement) => {
    setElement(el);
    onCleanup(() => {
      if (previous === el) previous = undefined;
      if (element() === el) setElement(undefined);
    });
  } };
}
