// Tab-based routing with per-tab navigation history. Each tab holds a back/
// forward stack of routes; the active tab's current route drives the page view.
// Middle-click opens links in a new tab; pinned tabs persist across sessions.
// `route()`/`openPage()`/`openJournals()` keep their old meaning (acting on the
// active tab) so existing call sites are unchanged.

import { createSignal } from "solid-js";
import { pushRecent, resolveAlias } from "./ui";

export type Route =
  | { kind: "journals" }
  | { kind: "page"; name: string; pageKind: "journal" | "page" };

export interface Tab {
  id: string;
  // Navigation history (back/forward stack) and the cursor into it.
  history: Route[];
  pos: number;
  pinned: boolean;
}

let counter = 0;
const newId = () => `tab-${counter++}`;

const [tabs, setTabs] = createSignal<Tab[]>(restore());
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

function sameRoute(a: Route, b: Route): boolean {
  if (a.kind !== b.kind) return false;
  if (a.kind === "journals") return true;
  return a.name === (b as typeof a).name && a.pageKind === (b as typeof a).pageKind;
}

// Navigate the active tab to a new route, pushing it onto the history stack
// (dropping any forward entries — standard browser behaviour). Re-navigating to
// the current route is a no-op so the back stack doesn't fill with duplicates.
function navigate(r: Route) {
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

export function openPage(name: string, pageKind: "journal" | "page" = "page") {
  // Resolve aliases so the route + working-set key use the canonical page name.
  if (pageKind === "page") name = resolveAlias(name);
  navigate({ kind: "page", name, pageKind });
  pushRecent(name, pageKind);
}

export function openJournals() {
  navigate({ kind: "journals" });
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

export function openInNewTab(r: Route) {
  // Open in a *background* tab without switching to it — matches a browser's
  // middle-click. Use openPage if you want to navigate there.
  const id = newId();
  setTabs([...tabs(), { id, history: [r], pos: 0, pinned: false }]);
  persist();
}

export function openPageInNewTab(name: string, pageKind: "journal" | "page" = "page") {
  if (pageKind === "page") name = resolveAlias(name);
  openInNewTab({ kind: "page", name, pageKind });
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
}

export function goForward() {
  if (!canGoForward()) return;
  setTabs(tabs().map((t) => (t.id === activeId() ? { ...t, pos: t.pos + 1 } : t)));
}

export function setActiveTab(id: string) {
  setActiveId(id);
}

export function closeTab(id: string) {
  const list = tabs();
  if (list.length === 1) return; // always keep one tab
  const idx = list.findIndex((t) => t.id === id);
  const next = list.filter((t) => t.id !== id);
  setTabs(next);
  if (activeId() === id) setActiveId(next[Math.max(0, idx - 1)].id);
  persist();
}

export function togglePin(id: string) {
  setTabs(tabs().map((t) => (t.id === id ? { ...t, pinned: !t.pinned } : t)));
  persist();
}

/** Move the dragged tab to the position of the target tab. */
export function reorderTab(dragId: string, targetId: string) {
  if (dragId === targetId) return;
  const list = [...tabs()];
  const from = list.findIndex((t) => t.id === dragId);
  const to = list.findIndex((t) => t.id === targetId);
  if (from < 0 || to < 0) return;
  const [moved] = list.splice(from, 1);
  list.splice(to, 0, moved);
  setTabs(list);
  persist();
}

// ---- persistence (pinned tabs only) ----

const KEY = "logseq-claude.pinnedTabs";

function persist() {
  try {
    const pinned = tabs()
      .filter((t) => t.pinned)
      .map((t) => tabRoute(t));
    localStorage.setItem(KEY, JSON.stringify(pinned));
  } catch {
    // ignore (e.g. storage disabled)
  }
}

function restore(): Tab[] {
  let pinnedRoutes: Route[] = [];
  try {
    const raw = localStorage.getItem(KEY);
    if (raw) pinnedRoutes = JSON.parse(raw);
  } catch {
    pinnedRoutes = [];
  }
  const pinned: Tab[] = pinnedRoutes.map((r) => ({
    id: newId(),
    history: [r],
    pos: 0,
    pinned: true,
  }));
  // Always start on a journals tab.
  return [
    { id: newId(), history: [{ kind: "journals" }], pos: 0, pinned: false },
    ...pinned,
  ];
}
