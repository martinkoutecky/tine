/** The one module that touches the global `window`/`document` realm directly
 * (OG-MULTIWINDOW P1/P2; VS Code's `getWindow`/`getActiveWindow` pattern).
 *
 * Tine can render into secondary workspace windows: popups opened with
 * `window.open("about:blank")` whose DOM the main window's JavaScript drives
 * (src/workspaceWindows.ts). There is one JS heap, one store and one editor,
 * but every popup has its own Window, Document, selection, focus, viewport,
 * animation-frame scheduler and observer constructors. Code that reaches for the
 * global `window`/`document` therefore silently addresses the MAIN window even
 * when the user is working in a popup. Resolve the realm instead:
 *
 * - from an element or event: `windowOf(el)`, `documentOf(el)`, `selectionOf(el)`;
 * - for "wherever the user is": `activeWindow()`, `activeDocument()`, `activeElement()`;
 * - for listeners every window needs (global shortcuts, outside-press, selection
 *   tracking): `onEachWindow(register)`, which also reaches windows opened later;
 * - for frames and observers: `requestFrame(target, cb)`, `newResizeObserver`,
 *   `newIntersectionObserver`.
 *
 * Timers are deliberately NOT routed through popups: a timer scheduled in a
 * popup's realm dies with the popup, and the save engine's debounce must
 * survive a closed window. Main-realm timers are throttled (not stopped) while
 * main is minimized: the hosted multi-window journey (run 37495725604) saved
 * typing in a workspace window 0.7 s (Linux), 1.3 s (macOS) and 3.1 s (Windows)
 * after main had been minimized for 5.5 minutes. The one exception is a
 * short poll tied to one window's own open or close (src/workspaceWindows.ts,
 * sleepOnEitherClock): it races the popup's clock against main's, so main's
 * fallback still fires if the popup's dies.
 * Animation frames are per window because WebKit stops main-realm frames
 * outright while main is minimized; `requestFrame` re-dispatches a pending frame
 * on the main realm when its window closes, so a callback is never lost.
 *
 * `src/windowRealm.guard.test.ts` bans the bare forms everywhere else. */
import { createContext, createSignal, useContext } from "solid-js";

export const MAIN_WINDOW_ID = "main";
/** Workspace windows one graph window may hold open at once (I-22). The
 * native side enforces the same bound (src-tauri/src/workspace_windows.rs). */
export const MAX_WORKSPACE_WINDOWS = 8;

const hasDom = typeof window !== "undefined" && typeof document !== "undefined";
/** Without a DOM at load (node-pool unit tests) the main window resolves through
 * whatever `window`/`document` globals a test installs, read at access time. */
function lateGlobalWindow(): Window {
  return new Proxy({} as Window, {
    get(_target, key) {
      const scope = globalThis as unknown as Record<PropertyKey, unknown>;
      const win = scope.window as Record<PropertyKey, unknown> | undefined;
      if (key === "document") return win?.document ?? scope.document;
      const source = win && key in win ? win : scope;
      const value = Reflect.get(source, key);
      return typeof value === "function" ? (value as (...args: unknown[]) => unknown).bind(source) : value;
    },
  });
}

/** The window the app's JavaScript was loaded into. */
export const mainWindow: Window = hasDom ? window : lateGlobalWindow();

type Registration = { id: string; win: Window; dispose: () => void };
const registry = new Map<string, Registration>();
type WindowHook = (win: Window, id: string) => (() => void) | void;
const hooks = new Set<{ register: WindowHook; disposers: Map<string, () => void> }>();
const [activeId, setActiveId] = createSignal(MAIN_WINDOW_ID);
const [registeredIds, setRegisteredIds] = createSignal<readonly string[]>([MAIN_WINDOW_ID]);

/** Reactive id of the Tine window the user most recently focused ("main" until
 * a popup takes focus). Pane commands, overlays and links resolve through it. */
export const activeWindowId = activeId;
/** Reactive list of registered window ids, main first. */
export const windowIds = registeredIds;

/** The id of the Tine window a component tree renders into. The main window's
 * tree uses the default; a workspace window's root provides its own id
 * (src/workspaceWindows.ts). Portals, overlays and pane trees read it. */
export const WindowContext = createContext<string>(MAIN_WINDOW_ID);

/** Id of the window the calling component renders into. */
export function useWindowId(): string {
  return useContext(WindowContext);
}

/** Window the calling component renders into (main when that window is gone). */
export function useOwnerWindow(): Window {
  return registry.get(useContext(WindowContext))?.win ?? mainWindow;
}

/** Record a logical window switch that has no OS focus event of its own (for
 * example a pane in another window became the focused pane). Unknown ids are
 * ignored. */
export function setActiveWindowId(id: string): void {
  if (registry.has(id)) setActiveId(id);
}

function trackFocus(id: string, win: Window): () => void {
  const onFocus = () => setActiveId(id);
  if (typeof win.addEventListener !== "function") return () => {}; // partial test globals
  win.addEventListener("focus", onFocus);
  return () => win.removeEventListener("focus", onFocus);
}

/** Register a window under `id` so the helpers below and every `onEachWindow`
 * hook reach it. Returns the unregister function (idempotent). Re-registering
 * an id replaces the previous registration. */
export function registerWindow(id: string, win: Window): () => void {
  registry.get(id)?.dispose();
  let live = true;
  const stopFocus = trackFocus(id, win);
  const registration: Registration = {
    id,
    win,
    dispose: () => {
      if (!live) return;
      live = false;
      stopFocus();
      for (const hook of hooks) {
        const dispose = hook.disposers.get(id);
        hook.disposers.delete(id);
        try { dispose?.(); } catch { console.error("window hook cleanup failed"); }
      }
      rescheduleFrames(win);
      if (registry.get(id) === registration) registry.delete(id);
      setRegisteredIds([...registry.keys()]);
      if (activeId() === id) setActiveId(MAIN_WINDOW_ID);
    },
  };
  registry.set(id, registration);
  setRegisteredIds([...registry.keys()]);
  for (const hook of hooks) attach(hook, id, win);
  return registration.dispose;
}

function attach(hook: { register: WindowHook; disposers: Map<string, () => void> }, id: string, win: Window) {
  try {
    const dispose = hook.register(win, id);
    if (dispose) hook.disposers.set(id, dispose);
  } catch {
    console.error("window hook failed");
  }
}

/** Run `register` for every registered window now and for each window
 * registered later; its returned cleanup runs when that window unregisters or
 * when the returned function is called. Exemplar: `installKeybindings`. */
export function onEachWindow(register: WindowHook): () => void {
  const hook = { register, disposers: new Map<string, () => void>() };
  hooks.add(hook);
  for (const { id, win } of registry.values()) attach(hook, id, win);
  return () => {
    hooks.delete(hook);
    for (const dispose of hook.disposers.values()) {
      try { dispose(); } catch { console.error("window hook cleanup failed"); }
    }
    hook.disposers.clear();
  };
}

export function windowById(id: string): Window | undefined {
  return registry.get(id)?.win;
}

export function windowIdOf(win: Window | null | undefined): string | undefined {
  if (!win) return undefined;
  for (const { id, win: candidate } of registry.values()) if (candidate === win) return id;
  return undefined;
}

export function registeredWindows(): Window[] {
  return [...registry.values()].map((r) => r.win);
}

type RealmTarget = EventTarget | Node | Event | Window | Document | null | undefined;

function isWindowLike(value: unknown): value is Window {
  return !!value && typeof value === "object" && (value as Window).window === value;
}

/** The window that owns `target`: an element's or text node's document's
 * view, a document's view, a window itself, or an event's target/view. Falls
 * back to the active window, never throws. Note: a node not yet inserted into a
 * popup still belongs to the main document; resolve after appending. */
export function windowOf(target?: RealmTarget): Window {
  if (!target) return activeWindow();
  if (isWindowLike(target)) return target;
  const node = target as Partial<Node> & Partial<Document>;
  const asDocument = target as unknown as Document;
  if (node.nodeType === 9 && asDocument.defaultView) return asDocument.defaultView;
  const owner = (target as Partial<Node>).ownerDocument?.defaultView;
  if (owner) return owner;
  const event = target as Partial<Event> & { view?: Window | null };
  if (typeof event.type === "string" && "target" in event) {
    if (event.target && event.target !== target) {
      const fromTarget = (event.target as Partial<Node>).ownerDocument?.defaultView
        ?? ((event.target as Partial<Node>).nodeType === 9 ? (event.target as Document).defaultView : null)
        ?? (isWindowLike(event.target) ? event.target : null);
      if (fromTarget) return fromTarget;
    }
    if (event.view && isWindowLike(event.view)) return event.view;
  }
  return activeWindow();
}

export function documentOf(target?: RealmTarget): Document {
  return windowOf(target).document;
}

/** The `<body>` overlays and measurement probes for `target` belong in. */
export function bodyOf(target?: RealmTarget): HTMLElement {
  return documentOf(target).body;
}

/** The registered window that has OS focus, else the most recently focused one,
 * else main. */
export function activeWindow(): Window {
  for (const { win } of registry.values()) {
    try {
      if (win.document.hasFocus()) return win;
    } catch { /* a closing popup can refuse access */ }
  }
  return registry.get(activeId())?.win ?? registry.get(MAIN_WINDOW_ID)?.win ?? mainWindow;
}

export function activeDocument(): Document {
  return activeWindow().document;
}

/** The focused element of the active window (`document.activeElement` there). */
export function activeElement(): Element | null {
  return activeDocument().activeElement;
}

/** The DOM selection of `target`'s window, or of the active window. */
export function selectionOf(target?: RealmTarget): Selection | null {
  return windowOf(target).getSelection();
}

/** Whether `target`'s document currently has OS focus. */
export function documentHasFocus(target?: RealmTarget): boolean {
  try { return documentOf(target).hasFocus(); } catch { return false; }
}

/** True when any registered Tine window is visible (replaces `!document.hidden`
 * where the question is app-wide, e.g. background flush). */
export function anyWindowVisible(): boolean {
  for (const { win } of registry.values()) {
    try { if (win.document.visibilityState === "visible") return true; } catch { /* closing */ }
  }
  return false;
}

/** True when any registered Tine window has OS focus. */
export function anyWindowFocused(): boolean {
  for (const { win } of registry.values()) {
    try { if (win.document.hasFocus()) return true; } catch { /* closing */ }
  }
  return false;
}

/** Run `cb` whenever the user returns to the app: any Tine window gains focus
 * or becomes visible. Replaces the `window` focus + `document` visibilitychange
 * pair (OG-MULTIWINDOW P5). Returns the remover. */
export function onAppReturn(cb: () => void): () => void {
  return onEachWindow((win) => {
    const doc = win.document;
    const onVisibility = () => { if (doc.visibilityState === "visible") cb(); };
    win.addEventListener("focus", cb);
    doc.addEventListener("visibilitychange", onVisibility);
    return () => {
      win.removeEventListener("focus", cb);
      doc.removeEventListener("visibilitychange", onVisibility);
    };
  });
}

/** Element at viewport point (x, y) of `target`'s window. */
export function elementFromPointIn(target: RealmTarget, x: number, y: number): Element | null {
  const doc = documentOf(target);
  return typeof doc.elementFromPoint === "function" ? doc.elementFromPoint(x, y) : null;
}

export function elementsFromPointIn(target: RealmTarget, x: number, y: number): Element[] {
  const doc = documentOf(target);
  return typeof doc.elementsFromPoint === "function" ? doc.elementsFromPoint(x, y) : [];
}

export function createRangeIn(target?: RealmTarget): Range {
  return documentOf(target).createRange();
}

/** Layout viewport of `target`'s window. */
export function viewportOf(target?: RealmTarget): { width: number; height: number; dpr: number; visual: VisualViewport | null } {
  const win = windowOf(target);
  return { width: win.innerWidth, height: win.innerHeight, dpr: win.devicePixelRatio || 1, visual: win.visualViewport ?? null };
}

export function matchMediaIn(target: RealmTarget, query: string): MediaQueryList | null {
  const win = windowOf(target);
  return typeof win.matchMedia === "function" ? win.matchMedia(query) : null;
}

/** `querySelector` across every registered document, main first. */
export function queryAllWindows<E extends Element = Element>(selector: string): E | null {
  for (const { win } of registry.values()) {
    const found = win.document.querySelector<E>(selector);
    if (found) return found;
  }
  return null;
}

export function queryAllWindowsAll<E extends Element = Element>(selector: string): E[] {
  return [...registry.values()].flatMap(({ win }) => [...win.document.querySelectorAll<E>(selector)]);
}

/** Add `type` on `target`'s window (or document with `on: "document"`) and
 * return the remover. For listeners that belong to ONE window: a modal's keys,
 * a drag's move/up. For listeners every window needs, use `onEachWindow`. */
export function listen<K extends keyof WindowEventMap>(
  target: RealmTarget,
  type: K,
  listener: (event: WindowEventMap[K]) => void,
  options?: boolean | AddEventListenerOptions & { on?: "window" | "document" },
): () => void {
  const win = windowOf(target);
  const owner: EventTarget = typeof options === "object" && options.on === "document" ? win.document : win;
  const fn = listener as EventListener;
  owner.addEventListener(type, fn, options);
  return () => owner.removeEventListener(type, fn, options);
}

// ---- animation frames --------------------------------------------------------

const pendingFrames = new Map<Window, Map<number, FrameRequestCallback>>();

function rescheduleFrames(win: Window) {
  const pending = pendingFrames.get(win);
  pendingFrames.delete(win);
  if (!pending?.size || win === mainWindow) return;
  for (const [id, cb] of pending) {
    try { win.cancelAnimationFrame(id); } catch { /* closing */ }
    // Still cancellable through the original cancel function until it runs.
    mainWindow.setTimeout(() => { if (pending.delete(id)) cb(mainWindow.performance.now()); }, 0);
  }
}

/** Schedule `cb` on the animation-frame clock of `target`'s window (the active
 * window when `target` is omitted). Returns a cancel function. A frame still
 * pending when its popup closes runs on the main realm instead of vanishing. */
export function requestFrame(target: RealmTarget, cb: FrameRequestCallback): () => void {
  const win = target === undefined ? activeWindow() : windowOf(target);
  if (typeof win?.requestAnimationFrame !== "function") {
    const handle = setTimeout(() => cb(Date.now()), 16);
    return () => clearTimeout(handle);
  }
  let pending = pendingFrames.get(win);
  if (!pending) pendingFrames.set(win, (pending = new Map()));
  const frames = pending;
  // A requestAnimationFrame that calls back synchronously (polyfills, test
  // doubles) has already run cb: it is not pending.
  let ran = false;
  let id = 0;
  id = win.requestAnimationFrame((time) => {
    ran = true;
    frames.delete(id);
    cb(time);
  });
  if (!ran) frames.set(id, cb);
  return () => {
    if (!frames.delete(id)) return;
    try { win.cancelAnimationFrame(id); } catch { /* closing */ }
  };
}

/** Resolve after the next frame of `target`'s window. */
export function nextFrame(target?: RealmTarget): Promise<number> {
  return new Promise((resolve) => { requestFrame(target, resolve); });
}

// ---- observers ---------------------------------------------------------------

type RealmWithObservers = Window & {
  ResizeObserver?: typeof ResizeObserver;
  IntersectionObserver?: typeof IntersectionObserver;
};

/** A ResizeObserver from `target`'s realm, or null where the realm has none
 * (jsdom, very old engines). */
export function newResizeObserver(target: RealmTarget, cb: ResizeObserverCallback): ResizeObserver | null {
  const Ctor = (windowOf(target) as RealmWithObservers).ResizeObserver;
  return typeof Ctor === "function" ? new Ctor(cb) : null;
}

/** An IntersectionObserver from `target`'s realm (implicit root = that window's
 * viewport), or null where the realm has none. */
export function newIntersectionObserver(
  target: RealmTarget,
  cb: IntersectionObserverCallback,
  options?: IntersectionObserverInit,
): IntersectionObserver | null {
  const Ctor = (windowOf(target) as RealmWithObservers).IntersectionObserver;
  return typeof Ctor === "function" ? new Ctor(cb, options) : null;
}

// ---- realm-agnostic type tests -----------------------------------------------
// `x instanceof HTMLElement` is false for a popup's nodes and events: each
// window has its own constructors. Test the DOM's own discriminators instead.

export function isElementNode(value: unknown): value is Element {
  return !!value && typeof value === "object" && (value as Node).nodeType === 1;
}

const XHTML_NS = "http://www.w3.org/1999/xhtml";

export function isHTMLElementNode(value: unknown): value is HTMLElement {
  return isElementNode(value) && value.namespaceURI === XHTML_NS;
}

export function isTextNode(value: unknown): value is Text {
  return !!value && typeof value === "object" && (value as Node).nodeType === 3;
}

export function isNodeValue(value: unknown): value is Node {
  return !!value && typeof value === "object" && typeof (value as Node).nodeType === "number"
    && typeof (value as Node).nodeName === "string";
}

/** Realm-agnostic event discriminators (`e instanceof PointerEvent` is false
 * for a popup's events). */
export function isPointerEventValue(value: unknown): value is PointerEvent {
  return !!value && typeof value === "object" && typeof (value as PointerEvent).pointerId === "number";
}

export function isKeyboardEventValue(value: unknown): value is KeyboardEvent {
  return !!value && typeof value === "object" && typeof (value as KeyboardEvent).key === "string"
    && typeof (value as Event).type === "string";
}

export function isMouseEventValue(value: unknown): value is MouseEvent {
  return !!value && typeof value === "object" && typeof (value as MouseEvent).clientX === "number"
    && typeof (value as MouseEvent).button === "number";
}

/** True for an element of tag `tag` (lower-case), in any realm. */
export function isElementTag<K extends keyof HTMLElementTagNameMap>(value: unknown, tag: K): value is HTMLElementTagNameMap[K] {
  return isElementNode(value) && value.localName === tag;
}

if (hasDom) registerWindow(MAIN_WINDOW_ID, mainWindow);
