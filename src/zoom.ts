// Interface zoom (UI scale) — like a browser's Ctrl +/-/0. Scales the WHOLE
// webview uniformly (fonts, spacing, images, the PDF pane, everything) via the
// native WebKit zoom-level, so it stays transparent to layout/caret math (unlike
// the CSS `zoom:` property, which desyncs getBoundingClientRect from clientX).
//
// It's a per-machine display preference → persisted in localStorage, NOT in
// config.edn (that's the graph config shared with OG Logseq over Syncthing).
//
// Routing: Ctrl +/-/0 zoom the INTERFACE when the notes pane is focused, and the
// PDF (its own render scale) when the PDF pane is focused. The split is driven by
// `activePane` (ui.ts) — this handler bails when the PDF pane is active, and
// PdfViewer.onKeyZoom bails when it isn't.
import { createSignal } from "solid-js";
import { getCurrentWebview } from "@tauri-apps/api/webview";
import { isTauri } from "./backend";
import { activePane } from "./ui";

const ZOOM_KEY = "logseq-claude.zoom";
const MIN = 0.5;
const MAX = 3.0;
// DOM WheelEvent has no trackpad phase/momentum flag. Treat wheel events separated
// by at most this gap as one gesture so a modifier added during a macOS momentum
// tail is consumed without becoming an accidental zoom.
export const ZOOM_WHEEL_MOMENTUM_TAIL_MS = 400;

export type WheelZoomGestureState = {
  lastWheelTimeStamp?: number;
  lastPlainWheelTimeStamp?: number;
};

export type WheelZoomGestureDecision = {
  consume: boolean;
  zoom: boolean;
  state: WheelZoomGestureState;
};

function load(): number {
  try {
    const v = parseFloat(localStorage.getItem(ZOOM_KEY) ?? "");
    if (Number.isFinite(v) && v >= MIN && v <= MAX) return v;
  } catch {
    // ignore
  }
  return 1;
}

const [interfaceZoom, setInterfaceZoomSig] = createSignal(load());
export { interfaceZoom };

const clamp = (z: number) => Math.min(MAX, Math.max(MIN, Math.round(z * 100) / 100));

/** Push the current zoom to the webview. Tauri-only (no-op on the web build).
 *  Call once at startup to restore the saved level. */
export function applyZoom(): void {
  if (!isTauri()) return;
  void getCurrentWebview()
    .setZoom(interfaceZoom())
    .catch(() => {});
}

function setZoom(z: number) {
  const next = clamp(z);
  setInterfaceZoomSig(next);
  // Store only non-default values; an empty/absent key reads back as 100%.
  try {
    if (next === 1) localStorage.removeItem(ZOOM_KEY);
    else localStorage.setItem(ZOOM_KEY, String(next));
  } catch {
    // ignore
  }
  applyZoom();
}

export const zoomIn = () => setZoom(interfaceZoom() * 1.1);
export const zoomOut = () => setZoom(interfaceZoom() / 1.1);
export const zoomReset = () => setZoom(1);

export function decideWheelZoomGesture(
  state: WheelZoomGestureState,
  hasZoomModifier: boolean,
  timeStamp: number
): WheelZoomGestureDecision {
  const sameGesture =
    state.lastWheelTimeStamp !== undefined &&
    timeStamp >= state.lastWheelTimeStamp &&
    timeStamp - state.lastWheelTimeStamp <= ZOOM_WHEEL_MOMENTUM_TAIL_MS;
  const nextState: WheelZoomGestureState = {
    lastWheelTimeStamp: timeStamp,
    lastPlainWheelTimeStamp: sameGesture ? state.lastPlainWheelTimeStamp : undefined,
  };

  if (!hasZoomModifier) {
    nextState.lastPlainWheelTimeStamp = timeStamp;
    return { consume: false, zoom: false, state: nextState };
  }

  return {
    consume: true,
    zoom: !(sameGesture && nextState.lastPlainWheelTimeStamp !== undefined),
    state: nextState,
  };
}

/** Ctrl/Cmd +/-/0 → interface zoom, but ONLY when the notes pane is focused; the
 *  PDF pane owns those keys for its own zoom (PdfViewer.onKeyZoom). Capture-phase
 *  so it precedes — and preventDefault suppresses — the webview's built-in page
 *  zoom. Returns an uninstaller. */
export function installInterfaceZoomKeys(): () => void {
  const onKey = (e: KeyboardEvent) => {
    if (!(e.ctrlKey || e.metaKey) || e.altKey) return;
    if (activePane() === "pdf") return; // PDF pane focused → it zooms the PDF
    if (e.key === "=" || e.key === "+") {
      e.preventDefault();
      zoomIn();
    } else if (e.key === "-" || e.key === "_") {
      e.preventDefault();
      zoomOut();
    } else if (e.key === "0") {
      e.preventDefault();
      zoomReset();
    }
  };
  window.addEventListener("keydown", onKey, true);
  return () => window.removeEventListener("keydown", onKey, true);
}

/** Ctrl/Cmd + mouse-wheel → interface zoom, routed by POINTER (not focus): a wheel
 *  over the PDF pane is left to PdfViewer's own Ctrl+wheel zoom. Modified wheel
 *  events are always consumed to suppress the webview's built-in page zoom, but
 *  macOS momentum tails where the modifier appeared mid-gesture do not change our
 *  zoom. Deliberate zoom bursts are coalesced into at most one gentle step per
 *  frame so the whole-UI relayout stays smooth. Returns an uninstaller. */
export function installInterfaceZoomWheel(): () => void {
  let pending = 0;
  let raf = 0;
  let wheelZoomState: WheelZoomGestureState = {};
  const flush = () => {
    raf = 0;
    const d = pending;
    pending = 0;
    if (d !== 0) setZoom(interfaceZoom() * (d < 0 ? 1.1 : 1 / 1.1));
  };
  const onWheel = (e: WheelEvent) => {
    const t = e.target as Element | null;
    if (t?.closest?.(".pdf-pane")) return; // over the PDF → its own wheel-zoom owns it
    const decision = decideWheelZoomGesture(wheelZoomState, e.ctrlKey || e.metaKey, e.timeStamp);
    wheelZoomState = decision.state;
    if (!decision.consume) return; // plain scroll → normal scrolling
    e.preventDefault(); // own the gesture; stop the webview's native page zoom
    // Fully own ctrl+wheel: stop it reaching the feed's smooth-scroll handler
    // (Lenis listens on `.main-content`), which would otherwise scroll while zooming.
    e.stopPropagation();
    if (!decision.zoom) return;
    pending += e.deltaY; // sign-based step is robust to wheel vs. line deltaMode
    if (!raf) raf = requestAnimationFrame(flush);
  };
  window.addEventListener("wheel", onWheel, { passive: false, capture: true });
  return () => {
    window.removeEventListener("wheel", onWheel, { capture: true } as EventListenerOptions);
    if (raf) cancelAnimationFrame(raf);
  };
}
