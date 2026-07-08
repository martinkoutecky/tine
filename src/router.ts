// Tab-based routing with per-tab navigation history. Each tab holds a back/
// forward stack of routes; the active tab's current route drives the page view.
// Middle-click opens links in a new tab. The whole tab session is persisted, so
// relaunching restores every tab in order, with its zoom state, and the same tab
// focused. `route()`/`openPage()`/`openJournals()` keep their old meaning via
// focused-pane shims, so existing call sites are unchanged.

import { createSignal, type Accessor } from "solid-js";
import {
  pushRecent,
  resolveAlias,
} from "./ui";
import { doc, persistentBlockRef, extendFeedForScroll } from "./store";
import { backend } from "./backend";
import { renderedBlocks } from "./lazyObserve";
import { navReuseTabs } from "./navSettings";
import { isMobilePlatform } from "./nativeChrome";

export type Route =
  | { kind: "journals" }
  | {
      kind: "page";
      name: string;
      pageKind: "journal" | "page";
      block?: string;
      /** Graph-root-relative file to pin this view to - set ONLY to reach a
       *  duplicate-day stray that shares a (kind,name) with the canonical file
       *  (#21). Absent for normal pages, which resolve by name. */
      path?: string;
    };

export interface Tab {
  id: string;
  // Navigation history (back/forward stack) and the cursor into it.
  history: Route[];
  pos: number;
  pinned: boolean;
}

export interface SerializedTab {
  history: Route[];
  pos: number;
  pinned: boolean;
}

export interface PaneSnapshot {
  tabs: SerializedTab[];
  activeIndex: number;
  scrolls?: (number | null)[];
}

export interface PaneRouter {
  paneId: string;
  tabs: Accessor<Tab[]>;
  activeId: Accessor<string>;
  setScrollerElement(el: HTMLElement | null): void;
  activeTab(): Tab;
  tabRoute(t: Tab): Route;
  route(): Route;
  restoreScrollFor(r: Route): void;
  openPage(
    name: string,
    pageKind?: "journal" | "page",
    opts?: { inPlace?: boolean }
  ): void;
  openJournals(opts?: { inPlace?: boolean }): void;
  resetTabsToJournals(): void;
  openFile(
    path: string,
    name: string,
    pageKind?: "journal" | "page",
    opts?: { inPlace?: boolean }
  ): void;
  focusBlock(id: string | null): void;
  openPageAtBlock(name: string, pageKind: "journal" | "page", blockId: string): void;
  openInNewTab(r: Route, foreground?: boolean): void;
  openPageInNewTab(name: string, pageKind?: "journal" | "page", block?: string): void;
  canGoBack(): boolean;
  canGoForward(): boolean;
  goBack(): void;
  goForward(): void;
  setActiveTab(id: string): void;
  closeActiveTab(): void;
  closeTab(id: string): Promise<void>;
  reopenClosedTab(): void;
  activateAdjacentTab(dir: 1 | -1): void;
  activateNextTab(): void;
  activatePrevTab(): void;
  togglePin(id: string): void;
  reorderTab(dragId: string, targetId: string): void;
  snapshot(): PaneSnapshot;
  restoreSnapshot(snapshot: PaneSnapshot): boolean;
  duplicateActiveSnapshot(): PaneSnapshot;
  scheduleSessionSave(): void;
  flushSession(): Promise<void>;
  restoreSession(): Promise<void>;
}

export function tabRoute(t: Tab): Route {
  return t.history[t.pos];
}

export function routeTitle(r: Route): string {
  if (r.kind === "journals") return "Journals";
  if (r.name.startsWith("hls__")) return r.name.slice(5);
  return r.name;
}

export function sameRoute(a: Route, b: Route): boolean {
  if (a.kind !== b.kind) return false;
  if (a.kind === "journals") return true;
  const bb = b as typeof a;
  return (
    a.name === bb.name && a.pageKind === bb.pageKind && a.block === bb.block && a.path === bb.path
  );
}

const CLOSED_CAP = 10;
const MOBILE_HISTORY_STATE = { tineRouter: true };

/** Stable partition: all pinned tabs first (in their relative order), then the
 *  unpinned ones. Pinned tabs always sit to the left of the strip (matches the
 *  OG plugin), so a pinned tab can't be visually stranded among unpinned ones. */
export function partitionPinned(list: Tab[]): Tab[] {
  return [...list.filter((t) => t.pinned), ...list.filter((t) => !t.pinned)];
}

let sessionPersistence = {
  schedule: () => {},
  flush: async () => {},
  restore: async () => {},
};

let lastTabCloseHandler: (paneId: string) => boolean = () => false;
let navigationInterceptor: (paneId: string, route: Route, opts: { sticky?: boolean }) => boolean =
  () => false;

export function installSessionPersistence(handlers: {
  schedule: () => void;
  flush: () => Promise<void>;
  restore: () => Promise<void>;
}) {
  sessionPersistence = handlers;
}

export function installLastTabCloseHandler(handler: (paneId: string) => boolean) {
  lastTabCloseHandler = handler;
}

export function installNavigationInterceptor(
  handler: (paneId: string, route: Route, opts: { sticky?: boolean }) => boolean
) {
  navigationInterceptor = handler;
}

export function createPaneRouter(paneId = "main"): PaneRouter {
  let counter = 0;
  const newId = () => `tab-${counter++}`;

  // Start on a single journals tab. The saved session (if any) is loaded
  // asynchronously from the backend by restoreSession(), called once at startup
  // before first paint - see main.tsx. (localStorage isn't durably persisted in
  // this WebKitGTK app, so the session round-trips through a real backend file.)
  const [tabs, setTabs] = createSignal<Tab[]>([
    { id: newId(), history: [{ kind: "journals" }], pos: 0, pinned: false },
  ]);
  const [activeId, setActiveId] = createSignal<string>(tabs()[0].id);

  // Recently-closed tabs (most-recent last), for Ctrl+Shift+T "reopen closed tab".
  // In-memory only - like a browser, a reopen doesn't survive a restart (the whole
  // session is restored from disk on launch anyway). Capped so it can't grow without
  // bound over a long session.
  const closedTabs: { history: Route[]; pos: number }[] = [];

  // ---- per-route scroll restoration (Firefox-style) ------------------------
  //
  // Remember where this pane's scroller was when we LEAVE a history entry, and
  // put it back when that entry is shown again (Alt+back/forward, or switching
  // tabs). Keyed by the route OBJECT in the tab's history array: that reference
  // is stable across goBack/goForward (only `pos` moves), while a freshly-pushed
  // route has no saved offset - so a forward navigation to a new page still
  // starts at the top, and only a return to a previously-seen entry restores its
  // scroll.
  const scrollByRoute = new WeakMap<Route, number>();
  let scrollerElement: HTMLElement | null = null;

  let mobileHistoryDepth = 0;
  let mobileHistoryBackPending = false;
  let handlingMobilePopState = false;
  let mobileHistoryListenerAttached = false;
  function setScrollerElement(el: HTMLElement | null) {
    scrollerElement = el;
  }

  function activeTab(): Tab {
    return tabs().find((t) => t.id === activeId()) ?? tabs()[0];
  }

  function route(): Route {
    return tabRoute(activeTab());
  }

  function mobileHistoryAvailable(): boolean {
    return (
      isMobilePlatform &&
      typeof window !== "undefined" &&
      !!window.history &&
      typeof window.history.pushState === "function" &&
      typeof window.history.back === "function" &&
      typeof window.addEventListener === "function"
    );
  }

  function mainScroller(): HTMLElement | null {
    if (scrollerElement) {
      if (scrollerElement.isConnected) return scrollerElement;
      scrollerElement = null;
    }
    if (typeof document === "undefined") return null; // no-DOM (unit tests)
    return document.querySelector(".main-content");
  }

  /** Record the current scroll offset against the active tab's current route.
   *  Called right before any navigation that leaves the current entry. */
  function rememberScroll() {
    const el = mainScroller();
    if (el) scrollByRoute.set(tabRoute(activeTab()), el.scrollTop);
  }

  function applyRouterBack() {
    rememberScroll(); // save this entry's scroll before stepping back
    setTabs(tabs().map((t) => (t.id === activeId() ? { ...t, pos: t.pos - 1 } : t)));
    persist();
  }

  function ensureMobileHistoryBridge() {
    if (!mobileHistoryAvailable() || mobileHistoryListenerAttached) return;
    mobileHistoryListenerAttached = true;
    window.addEventListener("popstate", () => {
      if (mobileHistoryDepth > 0) mobileHistoryDepth--;
      mobileHistoryBackPending = false;
      if (!canGoBack()) return;
      handlingMobilePopState = true;
      try {
        goBack();
      } finally {
        handlingMobilePopState = false;
      }
    });
  }

  function pushMobileHistoryEntry() {
    if (!mobileHistoryAvailable() || !canGoBack()) return;
    ensureMobileHistoryBridge();
    try {
      window.history.pushState(MOBILE_HISTORY_STATE, "", window.location?.href);
      mobileHistoryDepth++;
    } catch {
      // History is best-effort: if a WebView refuses pushState, router navigation still works.
    }
  }

  function requestMobileHistoryBack(): boolean {
    if (!mobileHistoryAvailable() || handlingMobilePopState || mobileHistoryDepth <= 0) {
      return false;
    }
    if (mobileHistoryBackPending) return true;
    ensureMobileHistoryBridge();
    mobileHistoryBackPending = true;
    try {
      window.history.back();
      return true;
    } catch {
      mobileHistoryBackPending = false;
      return false;
    }
  }

  /** Restore the saved scroll for a route once its content has rendered. A route
   *  with no saved offset (a new page) goes to the top. Retries for ~1s while the
   *  page is still growing toward a deep offset (blocks / linked refs render and
   *  measure asynchronously, so the target height isn't there on the first frame).
   *  If the saved offset is DEEPER than the loaded content - a journal-feed position
   *  in days that haven't been lazy-loaded yet - it pulls in more feed days until the
   *  target is reachable (or the feed is exhausted), so re-opening lands where you
   *  left off instead of at the bottom of the default window. */
  function restoreScrollFor(r: Route) {
    if (typeof requestAnimationFrame === "undefined") return; // no-DOM (unit tests)
    const target = scrollByRoute.get(r) ?? 0;
    let tries = 0;
    let extending = false;
    // Defer to a frame so the new page's content is laid out first - otherwise a
    // synchronous reset races the content swap (a new page wouldn't actually land
    // at the top). Then for a deep offset keep nudging while the page grows.
    const tick = () => {
      const el = mainScroller();
      if (!el) return;
      const max = Math.max(0, el.scrollHeight - el.clientHeight);
      el.scrollTop = Math.min(target, max);
      if (target <= 0 || el.scrollTop >= target - 1) return; // top, or reached
      // Target still out of reach. If the content is genuinely too SHORT (the feed's
      // deeper days aren't loaded), grow it and keep going; otherwise it's just async
      // layout catching up, so wait a frame. A deep offset can need several day-loads
      // (each async), so allow more tries while the feed is still growing.
      if (max < target - 1 && !extending) {
        extending = true;
        void extendFeedForScroll().then((grew) => {
          extending = false;
          if (grew ? tries++ < 400 : tries++ < 60) requestAnimationFrame(tick);
        });
        return;
      }
      if (tries++ < 60) requestAnimationFrame(tick);
    };
    requestAnimationFrame(tick);
  }

  // Navigate the active tab to a new route, pushing it onto the history stack
  // (dropping any forward entries - standard browser behaviour). Re-navigating to
  // the current route is a no-op so the back stack doesn't fill with duplicates.
  //
  // User navigations (`opts.sticky`) first reuse an already-open exact route when
  // that preference is on. Sticky (pinned) tabs otherwise stay on their content:
  // a pinned active tab opens the destination in a NEW foreground tab instead of
  // replacing the pinned view. Zoom (focusBlock) navigates in place, so it doesn't
  // pass `sticky`. Programmatic resets (graph switch, post-delete) pass `inPlace`
  // to force the active tab regardless of pin or reuse.
  function navigate(r: Route, opts: { sticky?: boolean } = {}) {
    if (navigationInterceptor(paneId, r, opts)) return;
    rememberScroll(); // capture where we are before leaving this entry
    const active = activeTab();
    if (opts.sticky && navReuseTabs()) {
      const existing = tabs().find((t) => sameRoute(tabRoute(t), r));
      if (existing) {
        if (existing.id !== active.id) {
          setActiveTab(existing.id);
          pushMobileHistoryEntry();
        }
        return;
      }
    }
    if (sameRoute(tabRoute(active), r)) return;
    if (opts.sticky && active.pinned) {
      openInNewTab(r, true);
      return;
    }
    setTabs(
      tabs().map((t) => {
        if (t.id !== activeId()) return t;
        if (sameRoute(tabRoute(t), r)) return t;
        const history = [...t.history.slice(0, t.pos + 1), r];
        return { ...t, history, pos: history.length - 1 };
      })
    );
    persist();
    pushMobileHistoryEntry();
  }

  function openPage(
    name: string,
    pageKind: "journal" | "page" = "page",
    opts: { inPlace?: boolean } = {}
  ) {
    // Resolve aliases so the route + working-set key use the canonical page name.
    if (pageKind === "page") name = resolveAlias(name);
    navigate({ kind: "page", name, pageKind }, { sticky: !opts.inPlace });
    pushRecent(name, pageKind);
  }

  function openJournals(opts: { inPlace?: boolean } = {}) {
    navigate({ kind: "journals" }, { sticky: !opts.inPlace });
  }

  /** Collapse the whole tab session down to a single fresh Journals tab and focus
   *  it. Used on a genuine graph SWITCH: every existing tab's history (and any
   *  pin/zoom) points at pages from the OLD graph that don't exist in the new one,
   *  so they're discarded wholesale rather than remapped - mirroring OG, which
   *  keeps one graph open at a time and reloads the whole workspace on switch. */
  function resetTabsToJournals() {
    const id = newId();
    setTabs([{ id, history: [{ kind: "journals" }], pos: 0, pinned: false }]);
    setActiveId(id);
    persist();
  }

  /** Open a SPECIFIC file by its graph-root-relative path - the way to reach a
   *  duplicate-day stray (`journals/Friday, 26-06-2026.org`) that shares a
   *  (kind,name) with the canonical day and so isn't reachable by name (#21).
   *  `name`/`pageKind` drive the title + routing; `path` pins the file the loader
   *  fetches and the editor saves back to. */
  function openFile(
    path: string,
    name: string,
    pageKind: "journal" | "page" = "journal",
    opts: { inPlace?: boolean } = {}
  ) {
    navigate({ kind: "page", name, pageKind, path }, { sticky: !opts.inPlace });
    pushRecent(name, pageKind);
  }

  /** Zoom the active tab into a block (or back out, when null). Zoom is part of the
   *  route, so it joins the per-tab back/forward history and a block can be opened
   *  pre-zoomed in its own tab via openPageInNewTab(name, kind, uuid).
   *
   *  Zooming navigates to the block's OWN page (not whichever route you're on), so
   *  it works from the journals feed, a linked-reference, or the command palette -
   *  not only when you're already on that page. Same destination as a middle-click,
   *  just in the current tab. persistentBlockRef pins the uuid (writes id:: once)
   *  so a zoomed tab survives a reload/restart, exactly like the new-tab path. */
  function focusBlock(id: string | null) {
    if (id === null) {
      // Zoom out: stay on the current page, drop the block. (No-op off a page.)
      const r = route();
      if (r.kind === "page") navigate({ kind: "page", name: r.name, pageKind: r.pageKind });
      return;
    }
    if (!doc.byId[id]) return; // block no longer loaded - nothing to zoom into
    const ref = persistentBlockRef(id);
    navigate({ kind: "page", name: ref.page, pageKind: ref.pageKind, block: ref.uuid });
  }

  /** Open a page and scroll the given block into view (block search results jump
   *  to the specific block, not just the page top). */
  function openPageAtBlock(name: string, pageKind: "journal" | "page", blockId: string) {
    // Pre-latch the target so its body renders eagerly (not as a deferred raw-text
    // placeholder) - a heavy target (table/image) then lands at its true height
    // instead of growing after the scroll. See AstBody / docs/adr (P1 lazy body).
    renderedBlocks.add(blockId);
    openPage(name, pageKind);
    // Let the page render, then scroll + briefly highlight the target block.
    let tries = 0;
    const tick = () => {
      const el = document.querySelector(`.ls-block[data-block-id="${blockId}"]`);
      if (el) {
        el.scrollIntoView({ block: "center", behavior: "smooth" });
        el.classList.add("block-flash");
        setTimeout(() => el.classList.remove("block-flash"), 1200);
      } else if (tries++ < 20) {
        setTimeout(tick, 50);
      }
    };
    setTimeout(tick, 60);
  }

  function openInNewTab(r: Route, foreground = false) {
    if (foreground && navigationInterceptor(paneId, r, {})) return;
    // Open a new tab. Default is *background* (no focus switch) - matches a
    // browser's middle-click. `foreground` is used by the sticky-tab redirect, so
    // a click on a pinned tab lands you on the new tab. New tabs are unpinned, so
    // appending keeps the pinned-left invariant (all pinned precede all unpinned).
    const id = newId();
    setTabs([...tabs(), { id, history: [r], pos: 0, pinned: false }]);
    if (foreground) setActiveId(id);
    persist();
  }

  function openPageInNewTab(
    name: string,
    pageKind: "journal" | "page" = "page",
    block?: string
  ) {
    if (pageKind === "page") name = resolveAlias(name);
    openInNewTab({ kind: "page", name, pageKind, block });
    pushRecent(name, pageKind);
  }

  // ---- back / forward ----

  function canGoBack(): boolean {
    return activeTab().pos > 0;
  }

  function canGoForward(): boolean {
    const t = activeTab();
    return t.pos < t.history.length - 1;
  }

  function goBack() {
    if (!canGoBack()) return;
    if (requestMobileHistoryBack()) return;
    applyRouterBack();
  }

  function goForward() {
    if (!canGoForward()) return;
    rememberScroll(); // save this entry's scroll before stepping forward
    setTabs(tabs().map((t) => (t.id === activeId() ? { ...t, pos: t.pos + 1 } : t)));
    persist();
    pushMobileHistoryEntry();
  }

  function setActiveTab(id: string) {
    const next = tabs().find((t) => t.id === id);
    if (next && navigationInterceptor(paneId, tabRoute(next), {})) return;
    rememberScroll(); // save the outgoing tab's scroll so switching back restores it
    setActiveId(id);
    persist();
  }

  /** Close the currently-active tab (Ctrl+W). No-op when it's the only tab. */
  function closeActiveTab() {
    void closeTab(activeId());
  }

  async function closeTab(id: string) {
    const list = tabs();
    if (list.length === 1) {
      if (route().kind !== "journals" && lastTabCloseHandler(paneId)) return;
      return; // feed pane keeps its last tab
    }
    const t = list.find((x) => x.id === id);
    // Pinned = sticky = "I want to keep this": confirm before closing, so an
    // accidental Ctrl+W (or middle-click) doesn't drop it. Uses the GTK dialog
    // (backend.confirm), NOT window.confirm - the latter silently returns true in
    // this WebKitGTK build, so the tab would close without ever asking. Unpinned
    // tabs skip the await and close synchronously (no behaviour change there).
    if (t?.pinned && !(await backend().confirm(`Close pinned tab “${routeTitle(tabRoute(t))}”?`))) return;
    // Save the current scroll against the (about-to-close) active tab's route, so a
    // later Ctrl+Shift+T reopen lands back where it was. The route object survives
    // in `closedTabs`, so its scrollByRoute entry is still live on reopen.
    if (activeId() === id) rememberScroll();
    const idx = list.findIndex((x) => x.id === id);
    const next = list.filter((x) => x.id !== id);
    // Remember the closed tab (its full history + position) so Ctrl+Shift+T can
    // reopen it. Most-recent last; cap the stack.
    if (t) {
      closedTabs.push({ history: t.history, pos: t.pos });
      if (closedTabs.length > CLOSED_CAP) closedTabs.shift();
    }
    setTabs(next);
    if (activeId() === id) setActiveId(next[Math.max(0, idx - 1)].id);
    persist();
  }

  /** Reopen the most-recently-closed tab (Ctrl+Shift+T), restoring its full
   *  back/forward history and the entry it was on, and focusing it. No-op if no
   *  tab has been closed this session (the stack is in-memory only). */
  function reopenClosedTab() {
    const last = closedTabs.pop();
    if (!last || !last.history.length) return;
    const id = newId();
    const pos = Math.min(Math.max(0, last.pos | 0), last.history.length - 1);
    // New tabs are unpinned, so appending preserves the pinned-left invariant.
    setTabs([...tabs(), { id, history: last.history, pos, pinned: false }]);
    setActiveId(id);
    persist();
  }

  /** Activate the next (dir=1) / previous (dir=-1) tab in strip order, wrapping
   *  around - browser-style Ctrl+PgDn / Ctrl+PgUp. No-op with a single tab. */
  function activateAdjacentTab(dir: 1 | -1) {
    const list = tabs();
    if (list.length < 2) return;
    const i = list.findIndex((t) => t.id === activeId());
    const next = list[(i + dir + list.length) % list.length];
    setActiveTab(next.id);
  }
  const activateNextTab = () => activateAdjacentTab(1);
  const activatePrevTab = () => activateAdjacentTab(-1);

  /** Toggle pin on a tab and move it to the pinned/unpinned boundary: pinning a
   *  tab slides it to the right end of the pinned group (rightmost pinned);
   *  unpinning slides it to the left end of the unpinned group. Both land it at the
   *  same spot - the boundary - which is what the OG plugin does on double-click. */
  function togglePin(id: string) {
    const list = tabs();
    const t = list.find((x) => x.id === id);
    if (!t) return;
    const updated = { ...t, pinned: !t.pinned };
    const rest = list.filter((x) => x.id !== id);
    const pinned = rest.filter((x) => x.pinned);
    const unpinned = rest.filter((x) => !x.pinned);
    setTabs([...pinned, updated, ...unpinned]);
    persist();
  }

  /** Move the dragged tab to the position of the target tab, then re-assert the
   *  pinned-left invariant (a cross-group drag snaps back to its own group). */
  function reorderTab(dragId: string, targetId: string) {
    if (dragId === targetId) return;
    const list = [...tabs()];
    const from = list.findIndex((t) => t.id === dragId);
    const to = list.findIndex((t) => t.id === targetId);
    if (from < 0 || to < 0) return;
    const [moved] = list.splice(from, 1);
    list.splice(to, 0, moved);
    setTabs(partitionPinned(list));
    persist();
  }

  function snapshot(): PaneSnapshot {
    const list = tabs();
    rememberScroll();
    return {
      tabs: list.map((t) => ({ history: t.history, pos: t.pos, pinned: t.pinned })),
      activeIndex: Math.max(0, list.findIndex((t) => t.id === activeId())),
      scrolls: list.map((t) => scrollByRoute.get(t.history[t.pos]) ?? null),
    };
  }

  function parseSnapshot(s: PaneSnapshot): { tabs: Tab[]; active: number } | null {
    const scrolls = Array.isArray(s?.scrolls) ? s.scrolls : [];
    const restored: Tab[] = [];
    (s?.tabs ?? []).forEach((t, i) => {
      if (!t || !Array.isArray(t.history) || t.history.length === 0) return;
      const pos = Math.min(Math.max(0, t.pos | 0), t.history.length - 1);
      const tab: Tab = { id: newId(), history: t.history, pos, pinned: !!t.pinned };
      restored.push(tab);
      const sc = scrolls[i];
      if (typeof sc === "number" && sc > 0) scrollByRoute.set(tab.history[pos], sc);
    });
    if (!restored.length) return null;
    const activeTabId = restored[Math.min(Math.max(0, s.activeIndex | 0), restored.length - 1)].id;
    const ordered = partitionPinned(restored);
    const active = Math.max(0, ordered.findIndex((t) => t.id === activeTabId));
    return { tabs: ordered, active };
  }

  function restoreSnapshot(s: PaneSnapshot): boolean {
    const parsed = parseSnapshot(s);
    if (!parsed) return false;
    setTabs(parsed.tabs);
    setActiveId(parsed.tabs[parsed.active].id);
    return true;
  }

  function duplicateActiveSnapshot(): PaneSnapshot {
    rememberScroll();
    const active = activeTab();
    return {
      tabs: [{ history: active.history.map((r) => ({ ...r })), pos: active.pos, pinned: active.pinned }],
      activeIndex: 0,
      scrolls: [scrollByRoute.get(active.history[active.pos]) ?? null],
    };
  }

  function persist() {
    sessionPersistence.schedule();
  }

  const scheduleSessionSave = persist;

  async function flushSession(): Promise<void> {
    await sessionPersistence.flush();
  }

  async function restoreSession(): Promise<void> {
    await sessionPersistence.restore();
  }

  return {
    paneId,
    tabs,
    activeId,
    setScrollerElement,
    activeTab,
    tabRoute,
    route,
    restoreScrollFor,
    openPage,
    openJournals,
    resetTabsToJournals,
    openFile,
    focusBlock,
    openPageAtBlock,
    openInNewTab,
    openPageInNewTab,
    canGoBack,
    canGoForward,
    goBack,
    goForward,
    setActiveTab,
    closeActiveTab,
    closeTab,
    reopenClosedTab,
    activateAdjacentTab,
    activateNextTab,
    activatePrevTab,
    togglePin,
    reorderTab,
    snapshot,
    restoreSnapshot,
    duplicateActiveSnapshot,
    scheduleSessionSave,
    flushSession,
    restoreSession,
  };
}

export const mainPaneRouter = createPaneRouter("main");

let focusedRouterProvider: () => PaneRouter = () => mainPaneRouter;
let mainRouterProvider: () => PaneRouter = () => mainPaneRouter;

export function installPaneRouterRegistry(registry: {
  focusedRouter: () => PaneRouter;
  mainRouter: () => PaneRouter;
}) {
  focusedRouterProvider = registry.focusedRouter;
  mainRouterProvider = registry.mainRouter;
}

function focusedRouterInstance(): PaneRouter {
  return focusedRouterProvider();
}

function mainRouterInstance(): PaneRouter {
  return mainRouterProvider();
}

export const tabs: Accessor<Tab[]> = () => focusedRouterInstance().tabs();
export const activeId: Accessor<string> = () => focusedRouterInstance().activeId();

export function activeTab(): Tab {
  return focusedRouterInstance().activeTab();
}

export function route(): Route {
  return focusedRouterInstance().route();
}

export function restoreScrollFor(r: Route) {
  focusedRouterInstance().restoreScrollFor(r);
}

export function openPage(
  name: string,
  pageKind: "journal" | "page" = "page",
  opts: { inPlace?: boolean } = {}
) {
  focusedRouterInstance().openPage(name, pageKind, opts);
}

export function openJournals(opts: { inPlace?: boolean } = {}) {
  focusedRouterInstance().openJournals(opts);
}

export function resetTabsToJournals() {
  mainRouterInstance().resetTabsToJournals();
}

export function openFile(
  path: string,
  name: string,
  pageKind: "journal" | "page" = "journal",
  opts: { inPlace?: boolean } = {}
) {
  focusedRouterInstance().openFile(path, name, pageKind, opts);
}

export function focusBlock(id: string | null) {
  focusedRouterInstance().focusBlock(id);
}

export function openPageAtBlock(name: string, pageKind: "journal" | "page", blockId: string) {
  focusedRouterInstance().openPageAtBlock(name, pageKind, blockId);
}

export function openInNewTab(r: Route, foreground = false) {
  focusedRouterInstance().openInNewTab(r, foreground);
}

export function openPageInNewTab(
  name: string,
  pageKind: "journal" | "page" = "page",
  block?: string
) {
  focusedRouterInstance().openPageInNewTab(name, pageKind, block);
}

export function canGoBack(): boolean {
  return focusedRouterInstance().canGoBack();
}

export function canGoForward(): boolean {
  return focusedRouterInstance().canGoForward();
}

export function goBack() {
  focusedRouterInstance().goBack();
}

export function goForward() {
  focusedRouterInstance().goForward();
}

export function setActiveTab(id: string) {
  focusedRouterInstance().setActiveTab(id);
}

export function closeActiveTab() {
  focusedRouterInstance().closeActiveTab();
}

export function closeTab(id: string) {
  return focusedRouterInstance().closeTab(id);
}

export function reopenClosedTab() {
  focusedRouterInstance().reopenClosedTab();
}

export function activateAdjacentTab(dir: 1 | -1) {
  focusedRouterInstance().activateAdjacentTab(dir);
}
export const activateNextTab = () => focusedRouterInstance().activateNextTab();
export const activatePrevTab = () => focusedRouterInstance().activatePrevTab();

export function togglePin(id: string) {
  focusedRouterInstance().togglePin(id);
}

export function reorderTab(dragId: string, targetId: string) {
  focusedRouterInstance().reorderTab(dragId, targetId);
}

export function scheduleSessionSave() {
  mainRouterInstance().scheduleSessionSave();
}

export function flushSession(): Promise<void> {
  return mainRouterInstance().flushSession();
}

export function restoreSession(): Promise<void> {
  return mainRouterInstance().restoreSession();
}
