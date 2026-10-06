// OG-MULTIWINDOW P4: secondary workspace windows (desktop only).
//
// A workspace window is a native OS window whose Document is driven entirely
// by THIS JavaScript context: the main webview calls `window.open`, Tauri's
// `on_new_window` hook (src-tauri/src/workspace_windows.rs) wraps the
// engine-linked webview in a decorated native window, and Solid renders the
// window's own pane tree into it. There is one model, one save engine, one
// undo history; the popup runs no script of its own and has no IPC surface.
//
// Lifecycle invariant: every window is disposed exactly once, whichever door
// closes it (its last pane, the native title bar, main closing or reloading, a
// graph switch, a session restore). Disposal ends an edit typed in the window,
// asks the save engine to flush it, forgets the window's pane tree, and
// destroys the native window: `popup.close()` alone leaves the native window
// registered on all three desktop platforms (spike OG-SPIKEMW C7).
import { createEffect, createRoot } from "solid-js";
import { clearDelegatedEvents, delegateEvents, DelegatedEvents } from "solid-js/web";
import { isTauri } from "./backend";
import { isPublishedExport } from "./publishedBackend";
import { platformKind } from "./nativeChrome";
import { endEdit } from "./editorController";
import { flushAll } from "./document";
import {
  createWindowLayout,
  dropWindowLayout,
  installWorkspaceWindowCloser,
  restorePaneLayout,
  type LayoutNode,
} from "./panes";
import { scheduleSessionSave } from "./router";
import type { PaneSnapshot } from "./router";
import type { Route } from "./routeTypes";
import type { ParsedWindow, WindowGeometry, WorkspaceWindowSession } from "./session";
import { pushToast } from "./toasts";
import { applyZoomToWebview, interfaceZoom } from "./zoom";
import { MAX_WORKSPACE_WINDOWS, isHTMLElementNode, mainWindow, registerWindow } from "./windowRealm";

type Realm = Window & typeof globalThis;

/** Why a window closes. Only "user" and "native" leave the rest of the app
 * running, so only they save the session (the others are part of a larger
 * transition that saves or replaces it). */
export type WorkspaceCloseReason = "user" | "native" | "quit" | "reload" | "graph-switch" | "restore";

/** Tauri's invoke, loaded once on first use (absent outside Tauri). */
let coreModule: Promise<typeof import("@tauri-apps/api/core")> | undefined;
async function nativeInvoke<T = unknown>(command: string, args: Record<string, unknown>): Promise<T> {
  const { invoke } = await (coreModule ??= import("@tauri-apps/api/core"));
  return invoke<T>(command, args);
}

interface Entry {
  readonly id: string;
  label: string | null;
  popup: Realm | null;
  disposed: boolean;
  /** A title-bar close is waiting for in-flight input (closeWorkspaceWindowByLabel). */
  closing: boolean;
  readonly teardown: (() => void)[];
}

/** Open workspace windows by JS window id, in opening order. Bounded by
 * MAX_WORKSPACE_WINDOWS (openWorkspaceWindow refuses past it). */
const live = new Map<string, Entry>();
let nextId = 0;

/** Renders a window's shell (src/components/WorkspaceWindowShell.tsx) into its
 * mount; installed from main.tsx so this module does not import App. */
export type WorkspaceShellRenderer = (windowId: string, mount: HTMLElement) => () => void;
let shellRenderer: WorkspaceShellRenderer | undefined;
export function installWorkspaceWindowShell(render: WorkspaceShellRenderer): () => void {
  shellRenderer = render;
  return () => { if (shellRenderer === render) shellRenderer = undefined; };
}

/** Desktop Tauri only: mobile has one window, a browser has no native hook,
 * and a published export has no writable graph to share. */
export function workspaceWindowsSupported(): boolean {
  return isTauri() && platformKind === "desktop" && !isPublishedExport();
}

export function workspaceWindowCount(): number {
  return live.size;
}

export interface OpenWorkspaceWindowSpec {
  /** A one-pane window showing this snapshot, when no `layout` is given. */
  snapshot?: PaneSnapshot;
  /** A restored pane tree (pane ids unique across the session). */
  layout?: LayoutNode;
  snapshots?: Map<string, PaneSnapshot>;
  focusedPaneId?: string;
  geometry?: WindowGeometry | null;
  /** Restoring a saved session: no save is scheduled and no focus is taken. */
  restoring?: boolean;
}

/** Open a workspace window. The pane tree is installed synchronously (so a
 * session saved meanwhile already lists the window); the native window opens
 * asynchronously. Returns the window id, or null when refused. */
export function openWorkspaceWindow(spec: OpenWorkspaceWindowSpec): string | null {
  if (!workspaceWindowsSupported()) return null;
  if (live.size >= MAX_WORKSPACE_WINDOWS) {
    pushToast(`At most ${MAX_WORKSPACE_WINDOWS} extra windows can be open at once.`, "error");
    return null;
  }
  const id = `ws-${++nextId}`;
  if (spec.layout && spec.snapshots) {
    if (!restorePaneLayout(spec.layout, spec.snapshots, spec.focusedPaneId, id)) return null;
  } else if (spec.snapshot) {
    createWindowLayout(id, spec.snapshot);
  } else {
    return null;
  }
  const entry: Entry = { id, label: null, popup: null, disposed: false, closing: false, teardown: [] };
  live.set(id, entry);
  if (!spec.restoring) scheduleSessionSave();
  void launch(entry, spec.geometry ?? null, !spec.restoring);
  return id;
}

/** Open `route` in a new workspace window: one pane, one tab. */
export function openRouteInNewWindow(route: Route): string | null {
  return openWorkspaceWindow({ snapshot: { tabs: [{ history: [route], pos: 0, pinned: false }], activeIndex: 0 } });
}

async function launch(entry: Entry, geometry: WindowGeometry | null, activate: boolean): Promise<void> {
  try {
    const label = await nativeInvoke<string>("workspace_window_prepare", { geometry });
    if (entry.disposed) return;
    entry.label = label;
    // The target name is the fresh native label: WebView2 hangs reopening a
    // reused name (spike OG-SPIKEMW, Windows).
    const popup = mainWindow.open("about:blank", label, "popup") as Realm | null;
    if (!popup) throw new Error("window.open returned no window");
    entry.popup = popup;
    await documentReady(popup);
    if (entry.disposed) return void destroyNative(entry);
    bootstrap(entry, popup);
    if (activate) popup.focus();
  } catch {
    console.error("workspace window failed to open");
    if (!entry.disposed) {
      pushToast("Couldn't open a new window.", "error");
      disposeWorkspaceWindow(entry.id, "user");
    }
  }
}

/** Wait `ms` on whichever clock fires first: the popup's own (the window the
 * user is in, which the engine does not throttle) or main's (the popup's may
 * stop as it opens or closes). Main's alone is throttled to seconds while main
 * is minimized (hosted run 37495725604: up to 3 s on Windows), and animation
 * frames stop entirely there (review F1). */
function sleepOnEitherClock(popup: Realm, ms: number): Promise<void> {
  return new Promise((resolve) => {
    let fired = false;
    const fire = () => { if (!fired) { fired = true; resolve(); } };
    try { popup.setTimeout(fire, ms); } catch { /* the popup's realm is going away */ }
    mainWindow.setTimeout(fire, ms);
  });
}

/** A popup's about:blank Document appears asynchronously; poll it. Bounded by
 * elapsed time, not by ticks, so a throttled clock cannot stretch the bound. */
async function documentReady(popup: Realm): Promise<void> {
  const started = Date.now();
  while (Date.now() - started < 5000) {
    try { if (popup.document?.body) return; } catch { /* not yet reachable */ }
    await sleepOnEitherClock(popup, 25);
  }
  throw new Error("workspace window document never appeared");
}

/** Mirror main's stylesheets and <html>/<body> attributes (theme, accent,
 * platform) into the popup, now and whenever they change. Interface zoom is a
 * native per-webview level, not an attribute: mirrorZoom carries it. */
function mirrorStyles(doc: Document): () => void {
  const source = mainWindow.document;
  const SHEETS = "link[rel=stylesheet], style";
  // One copy per source node, replaced only when that node changes (review
  // F9): re-cloning every sheet on each head mutation re-parsed the whole
  // theme in every window per keystroke-driven <style> update.
  const copies = new Map<Element, { copy: Element; signature: string }>();
  const signature = (node: Element) =>
    node.tagName === "LINK" ? `${node.outerHTML}|${(node as HTMLLinkElement).href}` : node.outerHTML;
  const copyOf = (node: Element) => {
    const copy = node.cloneNode(true) as Element;
    if (node.tagName === "LINK") (copy as HTMLLinkElement).href = (node as HTMLLinkElement).href;
    return doc.importNode(copy, true);
  };
  const mirror = () => {
    const sources = Array.from(source.head.querySelectorAll(SHEETS));
    const present = new Set(sources);
    for (const [node, entry] of copies) {
      if (!present.has(node)) { entry.copy.remove(); copies.delete(node); }
    }
    let previous: Element | null = null;
    for (const node of sources) {
      const now = signature(node);
      let entry = copies.get(node);
      if (!entry || entry.signature !== now) {
        const copy = copyOf(node);
        if (entry) entry.copy.replaceWith(copy);
        entry = { copy, signature: now };
        copies.set(node, entry);
      }
      // Keep main's order; only a new or out-of-place copy moves.
      if (previous ? previous.nextElementSibling !== entry.copy : !entry.copy.isConnected) {
        if (previous) previous.after(entry.copy);
        else doc.head.prepend(entry.copy);
      }
      previous = entry.copy;
    }
    const html = doc.documentElement;
    for (const name of html.getAttributeNames()) html.removeAttribute(name);
    for (const attr of Array.from(source.documentElement.attributes)) html.setAttribute(attr.name, attr.value);
    doc.body.className = source.body.className;
    const style = source.body.getAttribute("style");
    if (style === null) doc.body.removeAttribute("style");
    else doc.body.setAttribute("style", style);
  };
  mirror();
  const observer = new MutationObserver(mirror);
  observer.observe(source.head, { childList: true, subtree: true, characterData: true, attributes: true });
  observer.observe(source.documentElement, { attributes: true });
  observer.observe(source.body, { attributes: true });
  return () => observer.disconnect();
}

/** Keep the workspace window's native zoom equal to the interface zoom (review
 * F8): applied when the window opens and on every change. */
function mirrorZoom(label: string): () => void {
  return createRoot((dispose) => {
    createEffect(() => applyZoomToWebview(label, interfaceZoom()));
    return dispose;
  });
}

function bootstrap(entry: Entry, popup: Realm): void {
  const doc = popup.document;
  doc.title = "Tine";
  entry.teardown.push(mirrorStyles(doc));
  if (entry.label) entry.teardown.push(mirrorZoom(entry.label));
  delegateEvents([...DelegatedEvents], doc);
  entry.teardown.push(() => clearDelegatedEvents(doc));
  const mount = doc.createElement("div");
  mount.id = "root";
  doc.body.replaceChildren(mount);
  entry.teardown.push(registerWindow(entry.id, popup));
  if (!shellRenderer) throw new Error("workspace window shell not installed");
  entry.teardown.push(shellRenderer(entry.id, mount));
  const onPageHide = () => disposeWorkspaceWindow(entry.id, "native");
  popup.addEventListener("pagehide", onPageHide);
  const onResize = () => scheduleSessionSave();
  popup.addEventListener("resize", onResize);
  entry.teardown.push(() => {
    popup.removeEventListener("pagehide", onPageHide);
    popup.removeEventListener("resize", onResize);
  });
}

function geometryOf(entry: Entry): WindowGeometry | null {
  const popup = entry.popup;
  // Not yet open, or already gone: the window restores at the default place.
  if (!popup || entry.disposed || popup.closed) return null;
  const g = { x: popup.screenX, y: popup.screenY, width: popup.innerWidth, height: popup.innerHeight };
  return [g.x, g.y, g.width, g.height].every(Number.isFinite) && g.width > 0 && g.height > 0 ? g : null;
}

function destroyNative(entry: Entry): void {
  try { entry.popup?.close(); } catch { /* already closed */ }
  const label = entry.label;
  if (!label) return;
  void nativeInvoke("workspace_window_destroy", { label })
    .catch(() => console.error("workspace window native destroy failed"));
}

/** Close a workspace window. Exactly once per window; later calls (another
 * door racing the first) return false and do nothing. */
export function disposeWorkspaceWindow(id: string, reason: WorkspaceCloseReason): boolean {
  const entry = live.get(id);
  if (!entry || entry.disposed) return false;
  entry.disposed = true;
  live.delete(id);
  // End an edit typed in this window first: blurring its editor lets the
  // textarea's own handler commit the final buffer into the shared model.
  const doc = entry.popup && !entry.popup.closed ? entry.popup.document : null;
  const active = doc?.activeElement;
  const editing = !!doc?.querySelector("textarea.block-editor");
  if (isHTMLElementNode(active)) { try { active.blur(); } catch { /* closing */ } }
  if (editing) endEdit("blur");
  for (const stop of entry.teardown.splice(0).reverse()) {
    try { stop(); } catch { console.error("workspace window teardown failed"); }
  }
  dropWindowLayout(id);
  if (reason === "user" || reason === "native") {
    scheduleSessionSave();
    void flushAll();
  }
  destroyNative(entry);
  return true;
}

/** Close every workspace window (main closing, reloading, switching graph, or
 * restoring a session). */
export function closeAllWorkspaceWindows(reason: WorkspaceCloseReason): void {
  for (const id of [...live.keys()]) disposeWorkspaceWindow(id, reason);
}

/** Input quiet period, and its cap, before a title-bar close disposes. */
const INPUT_SETTLE_QUIET_MS = 150;
const INPUT_SETTLE_MAX_MS = 1000;

/** Resolve once `popup` has had no keyboard or text input for the quiet
 * period (at most the cap). The OS delivers keystrokes typed just before a
 * title-bar click to the page AFTER Tauri has relayed the close request (the
 * engine queues key events, the close travels another channel), so disposing
 * at once would drop the last characters: the native E2E lost the final
 * letter of text typed immediately before the close. The poll runs on
 * whichever clock fires first (sleepOnEitherClock). */
function inputSettled(popup: Realm): Promise<void> {
  return new Promise((resolve) => {
    let last = Date.now();
    const started = last;
    const events = ["keydown", "keyup", "beforeinput", "input", "compositionend"] as const;
    const touch = () => { last = Date.now(); };
    let doc: Document | null = null;
    try { doc = popup.document; } catch { doc = null; }
    for (const name of events) doc?.addEventListener(name, touch, true);
    const check = () => {
      const now = Date.now();
      if (now - last >= INPUT_SETTLE_QUIET_MS || now - started >= INPUT_SETTLE_MAX_MS) {
        for (const name of events) doc?.removeEventListener(name, touch, true);
        resolve();
      } else {
        void sleepOnEitherClock(popup, 25).then(check);
      }
    };
    void sleepOnEitherClock(popup, 25).then(check);
  });
}

/** The native window named `label` is gone: dispose it now (idempotent; an
 * already-disposed window is a no-op). Returns false when none matched. */
export function disposeWorkspaceWindowByLabel(label: string): boolean {
  for (const entry of live.values()) {
    if (entry.label === label) return disposeWorkspaceWindow(entry.id, "native");
  }
  return false;
}

/** The window's title-bar close: Rust held the close and named the label
 * (CLOSE_REQUESTED_EVENT in workspace_windows.rs). Disposal waits for input
 * already on its way to the window to land (inputSettled); any other door
 * arriving meanwhile disposes at once. A repeated request while settling is
 * ignored. Returns false when no open window has that label. */
export function closeWorkspaceWindowByLabel(label: string): boolean {
  for (const entry of live.values()) {
    if (entry.label !== label) continue;
    if (entry.closing) return true;
    entry.closing = true;
    const popup = entry.popup;
    if (!popup || popup.closed) disposeWorkspaceWindow(entry.id, "native");
    else void inputSettled(popup).then(() => disposeWorkspaceWindow(entry.id, "native"));
    return true;
  }
  return false;
}

/** Wire the doors that close or restore workspace windows. Called once at
 * startup from main.tsx with session.ts's installer (passed in, so modules
 * that only close windows do not load the session machinery); returns the
 * uninstaller. */
export function installWorkspaceWindows(
  installSession: (handlers: WorkspaceWindowSession) => () => void,
): () => void {
  const stops: (() => void)[] = [];
  stops.push(installWorkspaceWindowCloser((id) => disposeWorkspaceWindow(id, "user")));
  stops.push(installSession({
    list: () => [...live.values()].map((entry) => ({ id: entry.id, geometry: geometryOf(entry) })),
    closeAll: () => closeAllWorkspaceWindows("restore"),
    open: (windows: ParsedWindow[]) => {
      for (const w of windows) {
        openWorkspaceWindow({ layout: w.layout, snapshots: w.snapshots, focusedPaneId: w.focusedPaneId, geometry: w.geometry, restoring: true });
      }
    },
  }));
  // Main reloading or navigating away takes its JavaScript, and so every
  // popup's rendering, with it; Rust also destroys the native windows then.
  const onMainHide = () => closeAllWorkspaceWindows("reload");
  mainWindow.addEventListener("pagehide", onMainHide);
  stops.push(() => mainWindow.removeEventListener("pagehide", onMainHide));
  if (workspaceWindowsSupported()) {
    const unlisten: (() => void)[] = [];
    let stopped = false;
    void import("@tauri-apps/api/event")
      .then(({ listen }) => Promise.all([
        listen<string>("workspace-window-close-requested", (event) => {
          if (!closeWorkspaceWindowByLabel(event.payload)) {
            // Not ours (already disposed): destroy the stray native window.
            void nativeInvoke("workspace_window_destroy", { label: event.payload })
              .catch(() => console.error("workspace window native destroy failed"));
          }
        }),
        // A native window can be destroyed without a close request reaching
        // us (the insisted second close, an OS kill, a webview crash) and
        // without `pagehide` firing in its document: Rust reports every popup
        // destruction (DESTROYED_EVENT in workspace_windows.rs), so the window
        // never lingers here as a ghost whose panes nothing renders (review F1).
        listen<string>("workspace-window-destroyed", (event) => {
          disposeWorkspaceWindowByLabel(event.payload);
        }),
      ]))
      .then((handles) => { if (stopped) handles.forEach((stop) => stop()); else unlisten.push(...handles); })
      .catch(() => console.error("workspace window native listeners failed"));
    stops.push(() => { stopped = true; for (const stop of unlisten.splice(0)) stop(); });
  }
  return () => { for (const stop of stops.splice(0).reverse()) stop(); };
}

export function resetWorkspaceWindowsForTest(): void {
  closeAllWorkspaceWindows("restore");
  nextId = 0;
}
