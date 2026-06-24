// Tab-based routing with per-tab navigation history. Each tab holds a back/
// forward stack of routes; the active tab's current route drives the page view.
// Middle-click opens links in a new tab. The whole tab session is persisted, so
// relaunching restores every tab in order, with its zoom state, and the same tab
// focused. `route()`/`openPage()`/`openJournals()` keep their old meaning (acting
// on the active tab) so existing call sites are unchanged.

import { createSignal } from "solid-js";
import { pushRecent, resolveAlias } from "./ui";
import { doc, persistentBlockRef } from "./store";
import { backend } from "./backend";

export type Route =
  | { kind: "journals" }
  | { kind: "page"; name: string; pageKind: "journal" | "page"; block?: string };

export interface Tab {
  id: string;
  // Navigation history (back/forward stack) and the cursor into it.
  history: Route[];
  pos: number;
  pinned: boolean;
}

let counter = 0;
const newId = () => `tab-${counter++}`;

// Start on a single journals tab. The saved session (if any) is loaded
// asynchronously from the backend by restoreSession(), called once at startup
// before first paint — see main.tsx. (localStorage isn't durably persisted in
// this WebKitGTK app, so the session round-trips through a real backend file.)
const [tabs, setTabs] = createSignal<Tab[]>([
  { id: newId(), history: [{ kind: "journals" }], pos: 0, pinned: false },
]);
const [activeId, setActiveId] = createSignal<string>(tabs()[0].id);

export { tabs, activeId };

export function activeTab(): Tab {
  return tabs().find((t) => t.id === activeId()) ?? tabs()[0];
}

/** The route a tab is currently showing. */
export function tabRoute(t: Tab): Route {
  return t.history[t.pos];
}

export function route(): Route {
  return tabRoute(activeTab());
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
  return a.name === bb.name && a.pageKind === bb.pageKind && a.block === bb.block;
}

// Navigate the active tab to a new route, pushing it onto the history stack
// (dropping any forward entries — standard browser behaviour). Re-navigating to
// the current route is a no-op so the back stack doesn't fill with duplicates.
//
// Sticky (pinned) tabs: a pinned tab stays on its content. When `opts.sticky` is
// set (a user navigation, not a programmatic reset) and the active tab is pinned,
// the destination opens in a NEW foreground tab instead of replacing the pinned
// view — except a no-op (same route). Zoom (focusBlock) navigates in place, so it
// doesn't pass `sticky`. Programmatic resets (graph switch, post-delete) pass
// `inPlace` to force the active tab regardless of pin.
function navigate(r: Route, opts: { sticky?: boolean } = {}) {
  const active = activeTab();
  if (opts.sticky && active.pinned && !sameRoute(tabRoute(active), r)) {
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
}

export function openPage(
  name: string,
  pageKind: "journal" | "page" = "page",
  opts: { inPlace?: boolean } = {}
) {
  // Resolve aliases so the route + working-set key use the canonical page name.
  if (pageKind === "page") name = resolveAlias(name);
  navigate({ kind: "page", name, pageKind }, { sticky: !opts.inPlace });
  pushRecent(name, pageKind);
}

export function openJournals(opts: { inPlace?: boolean } = {}) {
  navigate({ kind: "journals" }, { sticky: !opts.inPlace });
}

/** Zoom the active tab into a block (or back out, when null). Zoom is part of the
 *  route, so it joins the per-tab back/forward history and a block can be opened
 *  pre-zoomed in its own tab via openPageInNewTab(name, kind, uuid).
 *
 *  Zooming navigates to the block's OWN page (not whichever route you're on), so
 *  it works from the journals feed, a linked-reference, or the command palette —
 *  not only when you're already on that page. Same destination as a middle-click,
 *  just in the current tab. persistentBlockRef pins the uuid (writes id:: once)
 *  so a zoomed tab survives a reload/restart, exactly like the new-tab path. */
export function focusBlock(id: string | null) {
  if (id === null) {
    // Zoom out: stay on the current page, drop the block. (No-op off a page.)
    const r = route();
    if (r.kind === "page") navigate({ kind: "page", name: r.name, pageKind: r.pageKind });
    return;
  }
  if (!doc.byId[id]) return; // block no longer loaded — nothing to zoom into
  const ref = persistentBlockRef(id);
  navigate({ kind: "page", name: ref.page, pageKind: ref.pageKind, block: ref.uuid });
}

/** Open a page and scroll the given block into view (block search results jump
 *  to the specific block, not just the page top). */
export function openPageAtBlock(name: string, pageKind: "journal" | "page", blockId: string) {
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

export function openInNewTab(r: Route, foreground = false) {
  // Open a new tab. Default is *background* (no focus switch) — matches a
  // browser's middle-click. `foreground` is used by the sticky-tab redirect, so
  // a click on a pinned tab lands you on the new tab. New tabs are unpinned, so
  // appending keeps the pinned-left invariant (all pinned precede all unpinned).
  const id = newId();
  setTabs([...tabs(), { id, history: [r], pos: 0, pinned: false }]);
  if (foreground) setActiveId(id);
  persist();
}

export function openPageInNewTab(
  name: string,
  pageKind: "journal" | "page" = "page",
  block?: string
) {
  if (pageKind === "page") name = resolveAlias(name);
  openInNewTab({ kind: "page", name, pageKind, block });
  pushRecent(name, pageKind);
}

// ---- back / forward ----

export function canGoBack(): boolean {
  return activeTab().pos > 0;
}

export function canGoForward(): boolean {
  const t = activeTab();
  return t.pos < t.history.length - 1;
}

export function goBack() {
  if (!canGoBack()) return;
  setTabs(tabs().map((t) => (t.id === activeId() ? { ...t, pos: t.pos - 1 } : t)));
  persist();
}

export function goForward() {
  if (!canGoForward()) return;
  setTabs(tabs().map((t) => (t.id === activeId() ? { ...t, pos: t.pos + 1 } : t)));
  persist();
}

export function setActiveTab(id: string) {
  setActiveId(id);
  persist();
}

/** Close the currently-active tab (Ctrl+W). No-op when it's the only tab. */
export function closeActiveTab() {
  closeTab(activeId());
}

export function closeTab(id: string) {
  const list = tabs();
  if (list.length === 1) return; // always keep one tab
  const t = list.find((x) => x.id === id);
  // Pinned = sticky = "I want to keep this": confirm before closing, so an
  // accidental Ctrl+W (or middle-click) doesn't drop it. A plain no-op would be
  // more confusing than a one-line prompt.
  if (t?.pinned && !confirm(`Close pinned tab “${routeTitle(tabRoute(t))}”?`)) return;
  const idx = list.findIndex((x) => x.id === id);
  const next = list.filter((x) => x.id !== id);
  setTabs(next);
  if (activeId() === id) setActiveId(next[Math.max(0, idx - 1)].id);
  persist();
}

/** Stable partition: all pinned tabs first (in their relative order), then the
 *  unpinned ones. Pinned tabs always sit to the left of the strip (matches the
 *  OG plugin), so a pinned tab can't be visually stranded among unpinned ones. */
function partitionPinned(list: Tab[]): Tab[] {
  return [...list.filter((t) => t.pinned), ...list.filter((t) => !t.pinned)];
}

/** Toggle pin on a tab and move it to the pinned/unpinned boundary: pinning a
 *  tab slides it to the right end of the pinned group (rightmost pinned);
 *  unpinning slides it to the left end of the unpinned group. Both land it at the
 *  same spot — the boundary — which is what the OG plugin does on double-click. */
export function togglePin(id: string) {
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
export function reorderTab(dragId: string, targetId: string) {
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

// ---- persistence (full session, backed by a real file via the backend) ----
//
// The whole tab session is saved on every change: each tab (not just pinned
// ones), in order, with its full back/forward history and which entry it's on,
// plus which tab is active. Routes already carry the zoomed-in block, so a tab
// zoomed into a bullet comes back zoomed. Persisted through the Rust backend to
// a file (WebKitGTK localStorage isn't durably persisted for this app), and
// restored once at startup by restoreSession().

interface PersistedSession {
  tabs: { history: Route[]; pos: number; pinned: boolean }[];
  activeIndex: number;
}

let saveTimer: ReturnType<typeof setTimeout> | undefined;

function persist() {
  // Light debounce: a burst of tab actions collapses to one backend write, and
  // it serializes writes so concurrent saves don't race. 150ms is short enough
  // that the user won't out-run it before quitting.
  clearTimeout(saveTimer);
  saveTimer = setTimeout(() => {
    const list = tabs();
    const session: PersistedSession = {
      tabs: list.map((t) => ({ history: t.history, pos: t.pos, pinned: t.pinned })),
      activeIndex: Math.max(0, list.findIndex((t) => t.id === activeId())),
    };
    void backend().saveSession(JSON.stringify(session)).catch(() => {});
  }, 150);
}

function parseSession(raw: string): { tabs: Tab[]; active: number } | null {
  try {
    const s = JSON.parse(raw) as PersistedSession;
    const restored: Tab[] = (s?.tabs ?? [])
      .filter((t) => t && Array.isArray(t.history) && t.history.length > 0)
      .map((t) => ({
        id: newId(),
        history: t.history,
        pos: Math.min(Math.max(0, t.pos | 0), t.history.length - 1),
        pinned: !!t.pinned,
      }));
    if (!restored.length) return null;
    // Keep the active tab pointing at the same tab after we re-sort pinned-left
    // (older sessions may have a mixed order).
    const activeTabId = restored[Math.min(Math.max(0, s.activeIndex | 0), restored.length - 1)].id;
    const ordered = partitionPinned(restored);
    const active = Math.max(0, ordered.findIndex((t) => t.id === activeTabId));
    return { tabs: ordered, active };
  } catch {
    return null;
  }
}

/** Write the session immediately, bypassing the debounce. Called on window
 *  close so a tab action taken right before quitting isn't lost. */
export async function flushSession(): Promise<void> {
  clearTimeout(saveTimer);
  const list = tabs();
  const session: PersistedSession = {
    tabs: list.map((t) => ({ history: t.history, pos: t.pos, pinned: t.pinned })),
    activeIndex: Math.max(0, list.findIndex((t) => t.id === activeId())),
  };
  try {
    await backend().saveSession(JSON.stringify(session));
  } catch {
    // best-effort — session restore is non-critical
  }
}

/** Load the saved tab session from the backend and apply it. Call once at
 *  startup, before first paint. No-op if nothing was saved, it's unreadable, or
 *  the user already changed the tabs (we never clobber live state). */
export async function restoreSession(): Promise<void> {
  let raw: string | null = null;
  try {
    raw = await backend().loadSession();
  } catch {
    return; // backend unavailable — keep the default journals tab
  }
  if (!raw) return;
  // Guard against clobbering: only restore while the strip is still the pristine
  // single-journals default (the user hasn't navigated during the async read).
  const cur = tabs();
  if (cur.length !== 1 || cur[0].history.length !== 1 || cur[0].history[0].kind !== "journals") return;
  const parsed = parseSession(raw);
  if (!parsed) return;
  setTabs(parsed.tabs);
  setActiveId(parsed.tabs[parsed.active].id);
}
