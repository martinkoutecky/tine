// Tab-based routing. Each tab holds a route; the active tab's route drives the
// page view. Middle-click opens links in a new tab; pinned tabs persist across
// sessions. `route()`/`openPage()`/`openJournals()` keep their old meaning
// (acting on the active tab) so existing call sites are unchanged.

import { createSignal } from "solid-js";
import { pushRecent } from "./ui";

export type Route =
  | { kind: "journals" }
  | { kind: "page"; name: string; pageKind: "journal" | "page" };

export interface Tab {
  id: string;
  route: Route;
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

export function route(): Route {
  return activeTab().route;
}

export function routeTitle(r: Route): string {
  if (r.kind === "journals") return "Journals";
  if (r.name.startsWith("hls__")) return r.name.slice(5);
  return r.name;
}

function updateActive(r: Route) {
  setTabs(tabs().map((t) => (t.id === activeId() ? { ...t, route: r } : t)));
  persist();
}

export function openPage(name: string, pageKind: "journal" | "page" = "page") {
  updateActive({ kind: "page", name, pageKind });
  pushRecent(name, pageKind);
}

export function openJournals() {
  updateActive({ kind: "journals" });
}

export function openInNewTab(r: Route) {
  const id = newId();
  setTabs([...tabs(), { id, route: r, pinned: false }]);
  setActiveId(id);
  persist();
}

export function openPageInNewTab(name: string, pageKind: "journal" | "page" = "page") {
  openInNewTab({ kind: "page", name, pageKind });
  pushRecent(name, pageKind);
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
      .map((t) => t.route);
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
  const pinned: Tab[] = pinnedRoutes.map((r) => ({ id: newId(), route: r, pinned: true }));
  // Always start on a journals tab.
  return [{ id: newId(), route: { kind: "journals" }, pinned: false }, ...pinned];
}
