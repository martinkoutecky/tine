import { backend } from "./backend";

/** Computed styles normally serialize sRGB as rgb()/rgba(). Only opaque
 * colours can replace the native fallback without guessing a backdrop. */
export function systemBarBackground(value: string): string | null {
  const match = value.match(/^rgba?\(\s*([\d.]+)[,\s]+([\d.]+)[,\s]+([\d.]+)(?:\s*[,/]\s*([\d.]+))?\s*\)$/);
  if (match) {
    const channels = match.slice(1, 4).map(Number);
    if (channels.some((n) => !Number.isFinite(n) || n < 0 || n > 255)
      || (match[4] !== undefined && Number(match[4]) !== 1)) return null;
    return "#" + channels.map((n) => Math.round(n).toString(16).padStart(2, "0")).join("");
  }
  // CSS Color 4 computed values (e.g. oklch/display-p3) need browser conversion
  // into the Android bridge's sRGB colour space.
  if (!globalThis.CSS?.supports("color", value) || value === "transparent" || !value) return null;
  const canvas = document.createElement("canvas");
  canvas.width = canvas.height = 1;
  const ctx = canvas.getContext("2d", { willReadFrequently: true });
  if (!ctx) return null;
  ctx.fillStyle = value;
  ctx.fillRect(0, 0, 1, 1);
  const [r, g, b, a] = ctx.getImageData(0, 0, 1, 1).data;
  return a === 255 ? "#" + [r, g, b].map((n) => n.toString(16).padStart(2, "0")).join("") : null;
}

/** One Android-only observer; no document-content traversal or polling. */
export function watchSystemBarBackground(update: () => void): () => void {
  let pending = false;
  let stopped = false;
  const schedule = () => {
    if (pending) return;
    pending = true;
    queueMicrotask(() => { pending = false; if (!stopped) update(); });
  };
  const observer = new MutationObserver(schedule);
  observer.observe(document.head, { childList: true, subtree: true, characterData: true, attributes: true });
  for (const element of [document.documentElement, document.body]) {
    observer.observe(element, { attributes: true });
  }
  document.head.addEventListener("load", schedule, true);
  // A theme may animate its background: the mutation sees the transition's
  // starting colour, so also sample its final (or cancelled) computed colour.
  const onTransition = (event: TransitionEvent) => {
    if (event.target === document.body && event.propertyName === "background-color") schedule();
  };
  document.body.addEventListener("transitionend", onTransition);
  document.body.addEventListener("transitioncancel", onTransition);
  window.addEventListener("resize", schedule);
  return () => {
    stopped = true;
    observer.disconnect();
    document.head.removeEventListener("load", schedule, true);
    document.body.removeEventListener("transitionend", onTransition);
    document.body.removeEventListener("transitioncancel", onTransition);
    window.removeEventListener("resize", schedule);
  };
}

let watching = false;
let last: string | undefined;
let dark = false;
let reportError: () => void;
function update() {
  const background = document.body && typeof getComputedStyle === "function"
    ? systemBarBackground(getComputedStyle(document.body).backgroundColor) : null;
  const key = `${dark}:${background}`;
  if (key === last) return;
  last = key;
  void backend().setSystemBarAppearance(dark, background ?? undefined)
    .catch(() => { last = undefined; reportError(); });
}

export function syncSystemBarAppearance(isDark: boolean, onError: () => void) {
  dark = isDark;
  reportError = onError;
  update();
  if (watching) return;
  watching = true;
  void backend().appPlatform().then((platform) => {
    if (platform === "android") { watchSystemBarBackground(update); update(); }
  }).catch(onError);
}
