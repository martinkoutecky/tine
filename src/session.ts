import { backend } from "./backend";
import { isMobilePlatform } from "./nativeChrome";
import {
  installSessionPersistence,
  sameRoute,
  type PaneSnapshot,
  type Route,
  type SerializedTab,
} from "./router";
import {
  applySidebarSession,
  favoritesSectionExpanded,
  recentSectionExpanded,
  rightSidebar,
  rightSidebarOpen,
  sidebarOpen,
  type SidebarItem,
  type SidebarSessionState,
} from "./ui";
import {
  feedPaneId,
  focusedPaneId,
  layoutPaneIds,
  layoutRoot,
  mainRouter,
  paneRouter,
  resetPaneLayoutToSingle,
  restorePaneLayout,
  type LayoutNode,
} from "./panes";

export type PersistedLayoutNode =
  | {
      kind: "split";
      dir: "row" | "col";
      ratio: number;
      children: [PersistedLayoutNode, PersistedLayoutNode];
    }
  | ({
      kind: "pane";
      paneId: string;
    } & PaneSnapshot);

export interface PersistedSession extends PaneSnapshot {
  leftSidebar?: boolean;
  rightSidebar?: boolean;
  rightSidebarItems?: SidebarItem[];
  favoritesSectionExpanded?: boolean;
  recentSectionExpanded?: boolean;
  layout?: PersistedLayoutNode;
  focusedPaneId?: string;
}

let saveTimer: ReturnType<typeof setTimeout> | undefined;

function validRoute(r: unknown): r is Route {
  if (!r || typeof r !== "object") return false;
  const o = r as Record<string, unknown>;
  if (o.kind === "journals") return true;
  return o.kind === "page" && typeof o.name === "string" && (o.pageKind === "journal" || o.pageKind === "page");
}

function parseSnapshotValue(raw: unknown): PaneSnapshot | null {
  const s = raw as Partial<PaneSnapshot> | null | undefined;
  if (!s || !Array.isArray(s.tabs)) return null;
  const tabs: SerializedTab[] = [];
  for (const t of s.tabs as Partial<SerializedTab>[]) {
    if (!t || !Array.isArray(t.history) || !t.history.length) continue;
    const history = t.history.filter(validRoute);
    if (!history.length) continue;
    tabs.push({
      history,
      pos: Math.min(Math.max(0, t.pos ?? 0), history.length - 1),
      pinned: !!t.pinned,
    });
  }
  if (!tabs.length) return null;
  return {
    tabs,
    activeIndex: Math.min(Math.max(0, s.activeIndex ?? 0), tabs.length - 1),
    scrolls: Array.isArray(s.scrolls)
      ? s.scrolls.map((x) => (typeof x === "number" && x > 0 ? x : null))
      : undefined,
  };
}

function currentRoute(t: SerializedTab): Route {
  return t.history[Math.min(Math.max(0, t.pos | 0), t.history.length - 1)];
}

function previousPageRoute(t: SerializedTab): Route | null {
  for (let i = t.pos - 1; i >= 0; i--) {
    const r = t.history[i];
    if (r?.kind === "page") return r;
  }
  for (const r of t.history) {
    if (r?.kind === "page") return r;
  }
  return null;
}

function sanitizeJournals(snapshot: PaneSnapshot, journalsSeen: { value: boolean }): PaneSnapshot | null {
  const tabs: SerializedTab[] = [];
  const scrolls: (number | null)[] = [];
  let activeIndex = 0;
  snapshot.tabs.forEach((tab, i) => {
    const active = currentRoute(tab);
    let next = tab;
    if (active.kind === "journals") {
      if (journalsSeen.value) {
        const repl = previousPageRoute(tab);
        if (!repl) return;
        const history = [...tab.history];
        history[tab.pos] = repl;
        next = { ...tab, history };
      } else {
        journalsSeen.value = true;
      }
    }
    if (i === snapshot.activeIndex) activeIndex = tabs.length;
    tabs.push(next);
    scrolls.push(snapshot.scrolls?.[i] ?? null);
  });
  if (!tabs.length) return null;
  return { tabs, activeIndex: Math.min(activeIndex, tabs.length - 1), scrolls };
}

function parseLayoutNode(
  raw: unknown,
  snapshots: Map<string, PaneSnapshot>,
  journalsSeen: { value: boolean }
): LayoutNode | null {
  if (!raw || typeof raw !== "object") return null;
  const o = raw as Record<string, unknown>;
  if (o.kind === "pane") {
    const paneId = typeof o.paneId === "string" && o.paneId ? o.paneId : null;
    if (!paneId) return null;
    const snap = parseSnapshotValue(o);
    if (!snap) return null;
    const sanitized = sanitizeJournals(snap, journalsSeen);
    if (!sanitized) return null;
    snapshots.set(paneId, sanitized);
    return { kind: "pane", paneId };
  }
  if (o.kind === "split") {
    if (o.dir !== "row" && o.dir !== "col") return null;
    const children = Array.isArray(o.children) ? o.children : [];
    const a = parseLayoutNode(children[0], snapshots, journalsSeen);
    const b = parseLayoutNode(children[1], snapshots, journalsSeen);
    if (a && b) {
      const ratio = typeof o.ratio === "number" ? Math.min(0.85, Math.max(0.15, o.ratio)) : 0.5;
      return { kind: "split", dir: o.dir, ratio, children: [a, b] };
    }
    return a ?? b;
  }
  return null;
}

function serializeLayout(node: LayoutNode): PersistedLayoutNode {
  if (node.kind === "pane") {
    return { kind: "pane", paneId: node.paneId, ...paneRouter(node.paneId).snapshot() };
  }
  return {
    kind: "split",
    dir: node.dir,
    ratio: node.ratio,
    children: [serializeLayout(node.children[0]), serializeLayout(node.children[1])],
  };
}

export function buildPersistedSession(): PersistedSession {
  const ids = layoutPaneIds();
  const mirrorId = feedPaneId() ?? (ids.includes(focusedPaneId()) ? focusedPaneId() : ids[0]) ?? "main";
  const mirror = paneRouter(mirrorId).snapshot();
  return {
    ...mirror,
    leftSidebar: sidebarOpen(),
    rightSidebar: rightSidebarOpen(),
    rightSidebarItems: rightSidebar(),
    favoritesSectionExpanded: favoritesSectionExpanded(),
    recentSectionExpanded: recentSectionExpanded(),
    layout: serializeLayout(layoutRoot()),
    focusedPaneId: focusedPaneId(),
  };
}

export function parsePersistedSession(raw: string): {
  layout: LayoutNode;
  snapshots: Map<string, PaneSnapshot>;
  focusedPaneId: string;
  sidebar: SidebarSessionState;
} | null {
  try {
    const s = JSON.parse(raw) as PersistedSession;
    const sidebar = {
      left: s.leftSidebar,
      right: s.rightSidebar,
      items: s.rightSidebarItems,
      favoritesExpanded: s.favoritesSectionExpanded,
      recentExpanded: s.recentSectionExpanded,
    };
    if (s.layout && !isMobilePlatform) {
      const snapshots = new Map<string, PaneSnapshot>();
      const layout = parseLayoutNode(s.layout, snapshots, { value: false });
      if (layout && snapshots.size) {
        return {
          layout,
          snapshots,
          focusedPaneId: typeof s.focusedPaneId === "string" ? s.focusedPaneId : "main",
          sidebar,
        };
      }
    }
    if (s.layout && isMobilePlatform) {
      const snapshots = new Map<string, PaneSnapshot>();
      const parsed = parseLayoutNode(s.layout, snapshots, { value: false });
      if (parsed && snapshots.size) {
        const feedId =
          [...snapshots].find(([, snap]) =>
            sameRoute(currentRoute(snap.tabs[snap.activeIndex]), { kind: "journals" })
          )?.[0] ?? [...snapshots.keys()][0];
        return {
          layout: { kind: "pane", paneId: "main" },
          snapshots: new Map([["main", snapshots.get(feedId)!]]),
          focusedPaneId: "main",
          sidebar,
        };
      }
    }
    const legacy = parseSnapshotValue(s);
    if (!legacy) return null;
    return {
      layout: { kind: "pane", paneId: "main" },
      snapshots: new Map([["main", legacy]]),
      focusedPaneId: "main",
      sidebar,
    };
  } catch {
    return null;
  }
}

function pristineDefault(): boolean {
  const ids = layoutPaneIds();
  if (ids.length !== 1 || ids[0] !== "main") return false;
  const snap = mainRouter().snapshot();
  return snap.tabs.length === 1 && snap.tabs[0].history.length === 1 && snap.tabs[0].history[0].kind === "journals";
}

export function applyParsedSession(parsed: NonNullable<ReturnType<typeof parsePersistedSession>>) {
  applySidebarSession(parsed.sidebar);
  if (parsed.layout.kind === "pane" && parsed.layout.paneId === "main") {
    resetPaneLayoutToSingle(parsed.snapshots.get("main"));
  } else {
    restorePaneLayout(parsed.layout, parsed.snapshots, parsed.focusedPaneId);
  }
}

export async function flushSession(): Promise<void> {
  clearTimeout(saveTimer);
  try {
    await backend().saveSession(JSON.stringify(buildPersistedSession()));
  } catch {
    // best-effort
  }
}

export function scheduleSessionSave() {
  clearTimeout(saveTimer);
  saveTimer = setTimeout(() => {
    void backend().saveSession(JSON.stringify(buildPersistedSession())).catch(() => {});
  }, 150);
}

export async function restoreSession(): Promise<void> {
  let raw: string | null = null;
  try {
    raw = await backend().loadSession();
  } catch {
    return;
  }
  if (!raw) return;
  const parsed = parsePersistedSession(raw);
  if (!parsed) return;
  applySidebarSession(parsed.sidebar);
  if (!pristineDefault()) return;
  applyParsedSession(parsed);
}

installSessionPersistence({
  schedule: scheduleSessionSave,
  flush: flushSession,
  restore: restoreSession,
});
