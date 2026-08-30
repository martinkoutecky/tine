import { backend } from "./backend";
import { isMobilePlatform } from "./nativeChrome";
import {
  installSessionPersistence,
  mintPdfViewId,
  sameRoute,
  type PaneSnapshot,
  type PdfRoute,
  type Route,
  type SerializedTab,
} from "./router";
import {
  applySidebarSession,
  favoritesSectionExpanded,
  clearLegacyRecentSource,
  legacyRecentPages,
  recentSectionExpanded,
  recentPages,
  rightSidebar,
  rightSidebarOpen,
  sidebarOpen,
  type SidebarItem,
  type RecentItem,
  type SidebarSessionState,
  sanitizeRecent,
  setRecentPages,
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
import { parsePersistedPdfTarget, type GraphSessionUiStateSchema, type PersistedPdfTarget } from "./uiStateRegistry";

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

export interface PersistedSession extends PaneSnapshot, Partial<GraphSessionUiStateSchema> {
  leftSidebar?: boolean;
  rightSidebar?: boolean;
  rightSidebarItems?: SidebarItem[];
  favoritesSectionExpanded?: boolean;
  recentSectionExpanded?: boolean;
  layout?: PersistedLayoutNode;
  focusedPaneId?: string;
  recentPages?: RecentItem[];
}

let saveTimer: ReturnType<typeof setTimeout> | undefined;

function invalidPersistedPdf(message: string): Route {
  return { kind: "invalid", title: "Unavailable PDF", message };
}

function validRoute(r: unknown, seenViewIds: Set<string>): Route | null {
  if (!r || typeof r !== "object") return null;
  const o = r as Record<string, unknown>;
  if (o.kind === "journals") return { kind: "journals" };
  if (o.kind === "query") {
    if (!(typeof o.id === "string" && o.id.length > 0 && o.id.length <= 128
      && (o.sourceKind === "search" || o.sourceKind === "dsl")
      && typeof o.source === "string" && o.source.length <= 65_536
      && (o.presentation === "search" || o.presentation === "list"
        || o.presentation === "table" || o.presentation === "board"))) return null;
    return { kind: "query", id: o.id, sourceKind: o.sourceKind, source: o.source, presentation: o.presentation };
  }
  if (o.kind === "invalid") {
    if (typeof o.title !== "string" || !o.title || o.title.length > 256
      || typeof o.message !== "string" || !o.message || o.message.length > 4096) return null;
    return { kind: "invalid", title: o.title, message: o.message };
  }
  if (o.kind === "pdf") {
    if (!(typeof o.viewId === "string" && o.viewId.length > 0 && o.viewId.length <= 128
      && typeof o.filename === "string" && o.filename.length > 0 && o.filename.length <= 4096
      && typeof o.label === "string" && o.label.length <= 4096
      && (o.page === undefined || (Number.isSafeInteger(o.page) && Number(o.page) > 0 && Number(o.page) <= 5_000))
      && (o.scale === undefined || (typeof o.scale === "number" && Number.isFinite(o.scale)
        && o.scale >= 0.05 && o.scale <= 20)))) {
      return invalidPersistedPdf("This saved PDF tab is malformed and was not opened.");
    }
    const viewId = seenViewIds.has(o.viewId) ? mintPdfViewId(seenViewIds) : o.viewId;
    seenViewIds.add(viewId);
    return {
      kind: "pdf",
      viewId,
      filename: o.filename,
      label: o.label,
      ...(o.page !== undefined ? { page: Number(o.page) } : {}),
      ...(o.scale !== undefined ? { scale: o.scale } : {}),
    };
  }
  if (o.kind !== "page" || typeof o.name !== "string" || o.name.length > 4096
    || (o.pageKind !== "journal" && o.pageKind !== "page")) return null;
  if (o.path !== undefined && (typeof o.path !== "string" || o.path.length > 4096)) return null;
  if (o.block !== undefined && (typeof o.block !== "string" || o.block.length > 4096)) return null;
  return {
    kind: "page", name: o.name, pageKind: o.pageKind,
    ...(o.path ? { path: o.path } : {}),
    ...(o.block ? { block: o.block } : {}),
  };
}

function parseSnapshotValue(raw: unknown, seenViewIds: Set<string>): PaneSnapshot | null {
  const s = raw as Partial<PaneSnapshot> | null | undefined;
  if (!s || !Array.isArray(s.tabs)) return null;
  const tabs: SerializedTab[] = [];
  for (const t of s.tabs as Partial<SerializedTab>[]) {
    if (!t || !Array.isArray(t.history) || !t.history.length) continue;
    const history = t.history.map((route) => validRoute(route, seenViewIds)).filter((route): route is Route => !!route);
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

function previousNonJournalsRoute(t: SerializedTab): Route | null {
  for (let i = t.pos - 1; i >= 0; i--) {
    const r = t.history[i];
    if (r?.kind !== "journals") return r;
  }
  for (const r of t.history) {
    if (r?.kind !== "journals") return r;
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
        const repl = previousNonJournalsRoute(tab);
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
  journalsSeen: { value: boolean },
  seenViewIds: Set<string>,
): LayoutNode | null {
  if (!raw || typeof raw !== "object") return null;
  const o = raw as Record<string, unknown>;
  if (o.kind === "pane") {
    const paneId = typeof o.paneId === "string" && o.paneId ? o.paneId : null;
    if (!paneId) return null;
    const snap = parseSnapshotValue(o, seenViewIds);
    if (!snap) return null;
    const sanitized = sanitizeJournals(snap, journalsSeen);
    if (!sanitized) return null;
    snapshots.set(paneId, sanitized);
    return { kind: "pane", paneId };
  }
  if (o.kind === "split") {
    if (o.dir !== "row" && o.dir !== "col") return null;
    const children = Array.isArray(o.children) ? o.children : [];
    const a = parseLayoutNode(children[0], snapshots, journalsSeen, seenViewIds);
    const b = parseLayoutNode(children[1], snapshots, journalsSeen, seenViewIds);
    if (a && b) {
      const ratio = typeof o.ratio === "number" ? Math.min(0.85, Math.max(0.15, o.ratio)) : 0.5;
      return { kind: "split", dir: o.dir, ratio, children: [a, b] };
    }
    return a ?? b;
  }
  return null;
}

function serializeRoute(route: Route): Route {
  if (route.kind === "journals") return { kind: "journals" };
  if (route.kind === "query") {
    return {
      kind: "query", id: route.id, sourceKind: route.sourceKind,
      source: route.source, presentation: route.presentation,
    };
  }
  if (route.kind === "pdf") {
    return {
      kind: "pdf", viewId: route.viewId, filename: route.filename, label: route.label,
      ...(route.page !== undefined ? { page: route.page } : {}),
      ...(route.scale !== undefined ? { scale: route.scale } : {}),
    };
  }
  if (route.kind === "invalid") {
    return { kind: "invalid", title: route.title, message: route.message };
  }
  return {
    kind: "page", name: route.name, pageKind: route.pageKind,
    ...(route.path ? { path: route.path } : {}),
    ...(route.block ? { block: route.block } : {}),
  };
}

function serializeSnapshot(snapshot: PaneSnapshot): PaneSnapshot {
  return {
    tabs: snapshot.tabs.map((tab) => ({
      history: tab.history.map(serializeRoute),
      pos: tab.pos,
      pinned: tab.pinned,
    })),
    activeIndex: snapshot.activeIndex,
    ...(snapshot.scrolls ? { scrolls: [...snapshot.scrolls] } : {}),
  };
}

function serializeLayout(node: LayoutNode): PersistedLayoutNode {
  if (node.kind === "pane") {
    return { kind: "pane", paneId: node.paneId, ...serializeSnapshot(paneRouter(node.paneId).snapshot()) };
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
  const mirror = serializeSnapshot(paneRouter(mirrorId).snapshot());
  return {
    ...mirror,
    leftSidebar: sidebarOpen(),
    rightSidebar: rightSidebarOpen(),
    rightSidebarItems: rightSidebar(),
    favoritesSectionExpanded: favoritesSectionExpanded(),
    recentSectionExpanded: recentSectionExpanded(),
    layout: serializeLayout(layoutRoot()),
    focusedPaneId: focusedPaneId(),
    recentPages: recentPages(),
  };
}

export interface SessionMigrationEnvironment {
  mobile?: boolean;
  legacyPdfWidth?: number;
  viewportWidth?: number;
}

function containsEquivalentPdf(snapshots: Map<string, PaneSnapshot>, filename: string): boolean {
  for (const snapshot of snapshots.values()) {
    for (const tab of snapshot.tabs) {
      if (tab.history.some((route) => route.kind === "pdf" && route.filename === filename)) return true;
    }
  }
  return false;
}

function migratedPdfRoute(target: PersistedPdfTarget, seenViewIds: Set<string>): PdfRoute {
  const viewId = mintPdfViewId(seenViewIds);
  seenViewIds.add(viewId);
  return { kind: "pdf", viewId, filename: target.filename, label: target.label };
}

function migrateLegacyPdf(
  parsed: { layout: LayoutNode; snapshots: Map<string, PaneSnapshot>; focusedPaneId: string },
  target: PersistedPdfTarget | null,
  environment: SessionMigrationEnvironment,
  seenViewIds: Set<string>,
): void {
  if (!target || containsEquivalentPdf(parsed.snapshots, target.filename)) return;
  const route = migratedPdfRoute(target, seenViewIds);
  if (environment.mobile ?? isMobilePlatform) {
    const snapshot = parsed.snapshots.get(parsed.focusedPaneId) ?? parsed.snapshots.values().next().value;
    if (!snapshot) return;
    const tab = snapshot.tabs[snapshot.activeIndex];
    tab.history = [...tab.history.slice(0, tab.pos + 1), route];
    tab.pos = tab.history.length - 1;
    return;
  }
  const used = new Set(parsed.snapshots.keys());
  let paneId = "pdf-migrated";
  let suffix = 1;
  while (used.has(paneId)) paneId = `pdf-migrated-${suffix++}`;
  parsed.snapshots.set(paneId, {
    tabs: [{ history: [route], pos: 0, pinned: false }],
    activeIndex: 0,
    scrolls: [null],
  });
  const viewport = environment.viewportWidth;
  const width = environment.legacyPdfWidth;
  const ratio = typeof viewport === "number" && viewport > 0 && typeof width === "number" && width > 0
    ? Math.min(0.85, Math.max(0.15, (viewport - width) / viewport))
    : 0.65;
  parsed.layout = {
    kind: "split", dir: "row", ratio,
    children: [parsed.layout, { kind: "pane", paneId }],
  };
}

export function parsePersistedSession(raw: string, environment: SessionMigrationEnvironment = {}): {
  layout: LayoutNode;
  snapshots: Map<string, PaneSnapshot>;
  focusedPaneId: string;
  sidebar: SidebarSessionState;
  recent: RecentItem[];
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
    const recent = s.recentPages === undefined ? legacyRecentPages() : sanitizeRecent(s.recentPages);
    const restoredPdfTarget = parsePersistedPdfTarget(s.pdfTarget);
    const mobile = environment.mobile ?? isMobilePlatform;
    const seenViewIds = new Set<string>();
    if (s.layout && !mobile) {
      const snapshots = new Map<string, PaneSnapshot>();
      const layout = parseLayoutNode(s.layout, snapshots, { value: false }, seenViewIds);
      if (layout && snapshots.size) {
        const parsed = {
          layout,
          snapshots,
          focusedPaneId: typeof s.focusedPaneId === "string" ? s.focusedPaneId : "main",
          sidebar,
          recent,
        };
        migrateLegacyPdf(parsed, restoredPdfTarget, environment, seenViewIds);
        return parsed;
      }
    }
    if (s.layout && mobile) {
      const snapshots = new Map<string, PaneSnapshot>();
      const parsedLayout = parseLayoutNode(s.layout, snapshots, { value: false }, seenViewIds);
      if (parsedLayout && snapshots.size) {
        const feedId =
          [...snapshots].find(([, snap]) =>
            sameRoute(currentRoute(snap.tabs[snap.activeIndex]), { kind: "journals" })
          )?.[0] ?? [...snapshots.keys()][0];
        const parsed: {
          layout: LayoutNode;
          snapshots: Map<string, PaneSnapshot>;
          focusedPaneId: string;
          sidebar: SidebarSessionState;
          recent: RecentItem[];
        } = {
          layout: { kind: "pane", paneId: "main" },
          snapshots: new Map([["main", snapshots.get(feedId)!]]),
          focusedPaneId: "main",
          sidebar,
          recent,
        };
        migrateLegacyPdf(parsed, restoredPdfTarget, { ...environment, mobile: true }, seenViewIds);
        return parsed;
      }
    }
    const legacy = parseSnapshotValue(s, seenViewIds);
    if (!legacy) return null;
    const parsed: {
      layout: LayoutNode;
      snapshots: Map<string, PaneSnapshot>;
      focusedPaneId: string;
      sidebar: SidebarSessionState;
      recent: RecentItem[];
    } = {
      layout: { kind: "pane", paneId: "main" },
      snapshots: new Map([["main", legacy]]),
      focusedPaneId: "main",
      sidebar,
      recent,
    };
    migrateLegacyPdf(parsed, restoredPdfTarget, environment, seenViewIds);
    return parsed;
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
  setRecentPages(parsed.recent);
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
    clearLegacyRecentSource();
  } catch {
    // best-effort
  }
}

export function scheduleSessionSave() {
  clearTimeout(saveTimer);
  saveTimer = setTimeout(() => {
    void backend().saveSession(JSON.stringify(buildPersistedSession()))
      .then(clearLegacyRecentSource)
      .catch(() => {});
  }, 150);
}

export async function restoreSession(): Promise<void> {
  try {
    let raw: string | null = null;
    try {
      raw = await backend().loadSession();
    } catch {
      return;
    }
    if (!raw) {
      setRecentPages(legacyRecentPages());
      return;
    }
    const parsed = parsePersistedSession(raw);
    if (!parsed) return;
    applySidebarSession(parsed.sidebar);
    setRecentPages(parsed.recent);
    if (!pristineDefault()) return;
    applyParsedSession(parsed);
  } finally {
    // The registry is graph-scoped, so the early pre-bind restore may fail and
    // the post-bind restore in graph.ts retries it. Keep startup best-effort just
    // like the existing session restore; a bad registry must not block the app.
    try {
      const { initializeWorkspaces } = await import("./workspaces");
      await initializeWorkspaces();
    } catch {
      // unavailable before graph binding, older backend, or invalid registry
    }
  }
}

installSessionPersistence({
  schedule: scheduleSessionSave,
  flush: flushSession,
  restore: restoreSession,
});
