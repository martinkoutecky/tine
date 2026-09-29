import { backend } from "./backend";
import { normalizeFriendlyPageMatchScope, normalizeQueryDisplayDraft } from "./editor/queryDisplayDraft";
import type { QueryRoute } from "./routeTypes";
import { graphOwner, readOwned, writeOwned } from "./owned";
import { dismissToast, pushToastUnique } from "./toasts";
import { isSinglePaneShell } from "./nativeChrome";
import {
  installSessionPersistence,
  mintPdfViewId,
  normalizeQueryPresentation,
  sameRoute,
  type PaneSnapshot,
  type Route,
  type SerializedTab,
} from "./router";
import { applySidebarSession, favoritesSectionExpanded, clearLegacyRecentSource, legacyRecentPages, recentSectionExpanded, recentPages, rightSidebar, rightSidebarOpen, sidebarOpen, type SidebarItem, type RecentItem, type SidebarSessionState, sanitizeRecent, setRecentPages } from "./ui";
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

/** Saved pane and sidebar state. workspaceId names the workspace that produced
 * this session; a different registry active ID can select its parked snapshot
 * during startup recovery. Missing fields retain legacy defaults. */
export interface PersistedSession extends PaneSnapshot {
  workspaceId?: string;
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
let currentWorkspaceId: string | null = null;
let restoredWorkspaceId: string | null = null;
let restoredSessionPresent: boolean | null = null;
let sessionIntentRevision = 0;
let restoreEvidence: { owner: ReturnType<typeof graphOwner>; snapshot: string; intent: string; present: boolean | null; workspaceId: string | null } | null = null;

function intentToken(): string {
  return JSON.stringify([sessionIntentRevision, ...layoutPaneIds().map((id) => [id, paneRouter(id).routeIntentRevision()])]);
}

export function setSessionWorkspaceId(id: string | null): void { currentWorkspaceId = id; }
export function discardWorkspaceRestoreEvidence(): void { restoreEvidence = null; restoredWorkspaceId = null; restoredSessionPresent = null; }

/** Capture one startup recovery decision before the registry read and clear the
 * session's workspace ID until a registry is installed. On landing,
 * a live route or session intent always wins over a parked workspace, including
 * when its serialized snapshot has returned to the same value. A refused parked
 * recovery is shown to the user. Cost is one session snapshot at each boundary;
 * malformed parked sessions reject. */
export function prepareWorkspaceRecovery(): (activeId: string, parked: PersistedSession) => PersistedSession {
  const evidence = restoreEvidence;
  restoreEvidence = null;
  const beforeClear = JSON.stringify(buildPersistedSession());
  const beforeClearIntent = intentToken();
  const intervened = !!evidence && (!evidence.owner() || evidence.snapshot !== beforeClear || evidence.intent !== beforeClearIntent);
  setSessionWorkspaceId(null);
  const startSnapshot = JSON.stringify(buildPersistedSession());
  const startIntent = intentToken();
  return (activeId, parked) => {
    const changed = JSON.stringify(buildPersistedSession()) !== startSnapshot || intentToken() !== startIntent
      || intervened;
    const wantsParked = !!evidence && (evidence.present === false || !!evidence.workspaceId && evidence.workspaceId !== activeId);
    if (changed && wantsParked) {
      pushToastUnique("Live changes were kept; workspace recovery was skipped.", "error");
      return buildPersistedSession();
    }
    if (wantsParked) {
      const parsed = parsePersistedSession(JSON.stringify(parked));
      if (!parsed) throw new Error("The active workspace snapshot is invalid");
      applyParsedSession(parsed);
      scheduleSessionSave();
    }
    return buildPersistedSession();
  };
}

function validRoute(r: unknown, seenViewIds: Set<string>): Route | null {
  if (!r || typeof r !== "object") return null;
  const o = r as Record<string, unknown>;
  if (o.kind === "journals") return { kind: "journals" };
  if (o.kind === "conflicts") return { kind: "conflicts" };
  if (o.kind === "query") {
    const presentation = normalizeQueryPresentation(o.presentation);
    if (!(typeof o.id === "string" && o.id.length > 0 && o.id.length <= 128
      && (o.sourceKind === "search" || o.sourceKind === "dsl")
      && typeof o.source === "string" && o.source.length <= 65_536
      && presentation)) return null;
    // A malformed OPTIONAL field costs only that field: the tab (and its history
    // entry) survives, and a bad membership mode is dropped, never widened.
    const optional: Partial<QueryRoute> = {};
    const scope = o.pageMatchScope === undefined ? null : normalizeFriendlyPageMatchScope(o.pageMatchScope);
    if (scope) optional.pageMatchScope = scope;
    for (const key of ["pagePresentation", "blockPresentation"] as const) {
      const value = o[key] === undefined ? null : normalizeQueryPresentation(o[key]);
      if (value) optional[key] = value;
    }
    for (const key of ["pageDisplay", "blockDisplay"] as const) {
      const value = o[key] === undefined ? null : normalizeQueryDisplayDraft(o[key]);
      if (value) optional[key] = value;
    }
    return { kind: "query", id: o.id, sourceKind: o.sourceKind, source: o.source, presentation, ...optional };
  }
  if (o.kind === "invalid") {
    const detail = o.message;
    if (typeof o.title !== "string" || !o.title || o.title.length > 256
      || typeof detail !== "string" || !detail || detail.length > 4096) return null;
    return { kind: "invalid", title: o.title, message: detail };
  }
  if (o.kind === "pdf") {
    const malformed = !(typeof o.viewId === "string" && o.viewId.length > 0 && o.viewId.length <= 128
      && typeof o.filename === "string" && o.filename.length > 0 && o.filename.length <= 4096
      && typeof o.label === "string" && o.label.length <= 4096
      && (o.page === undefined || (Number.isSafeInteger(o.page) && Number(o.page) > 0 && Number(o.page) <= 5000))
      && (o.scale === undefined || (typeof o.scale === "number" && Number.isFinite(o.scale)
        && o.scale >= 0.05 && o.scale <= 20)));
    if (malformed) return { kind: "invalid", title: "Unavailable PDF",
      message: "This saved PDF tab is malformed and was not opened." };
    const viewId = seenViewIds.has(o.viewId as string) ? mintPdfViewId(seenViewIds) : o.viewId as string;
    seenViewIds.add(viewId);
    return { kind: "pdf", viewId, filename: o.filename as string, label: o.label as string,
      ...(o.page !== undefined ? { page: Number(o.page) } : {}),
      ...(o.scale !== undefined ? { scale: Number(o.scale) } : {}) };
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
    ...(currentWorkspaceId ? { workspaceId: currentWorkspaceId } : {}),
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

export function parsePersistedSession(raw: string): {
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
    const singlePane = isSinglePaneShell();
    const seenViewIds = new Set<string>();
    if (s.layout && !singlePane) {
      const snapshots = new Map<string, PaneSnapshot>();
      const layout = parseLayoutNode(s.layout, snapshots, { value: false }, seenViewIds);
      if (layout && snapshots.size) {
        return {
          layout,
          snapshots,
          focusedPaneId: typeof s.focusedPaneId === "string" ? s.focusedPaneId : "main",
          sidebar,
          recent,
        };
      }
    }
    if (s.layout && singlePane) {
      const snapshots = new Map<string, PaneSnapshot>();
      const parsed = parseLayoutNode(s.layout, snapshots, { value: false }, seenViewIds);
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
          recent,
        };
      }
    }
    const legacy = parseSnapshotValue(s, seenViewIds);
    if (!legacy) return null;
    return {
      layout: { kind: "pane", paneId: "main" },
      snapshots: new Map([["main", legacy]]),
      focusedPaneId: "main",
      sidebar,
      recent,
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
  setRecentPages(parsed.recent);
  if (parsed.layout.kind === "pane" && parsed.layout.paneId === "main") {
    resetPaneLayoutToSingle(parsed.snapshots.get("main"));
  } else {
    restorePaneLayout(parsed.layout, parsed.snapshots, parsed.focusedPaneId);
  }
}

let sessionSaveFailure: { id: number; message: string } | null = null;

function reportSessionSaveFailure(error: unknown): void {
  const message = `Could not save session: ${String(error)}`;
  const priorMessage = sessionSaveFailure?.message;
  if (sessionSaveFailure && priorMessage !== message) dismissToast(sessionSaveFailure.id);
  sessionSaveFailure = {
    id: pushToastUnique(message, "error", { sticky: true, action: { label: "Retry", run: () => { void flushSession().catch(() => console.error("Session retry failed")); } } }),
    message,
  };
}

function clearSessionSaveFailure(): void {
  if (sessionSaveFailure) dismissToast(sessionSaveFailure.id);
  sessionSaveFailure = null;
}

/** Cancel a scheduled save and write the current session. A write failure shows
 * one Retry toast and rejects; stale graph ownership also rejects. Completion
 * certifies persistence for the current graph. Cost follows session bytes and backend latency. */
export async function flushSession(): Promise<void> {
  sessionIntentRevision++;
  const owner = graphOwner();
  clearTimeout(saveTimer);
  try {
    const result = await writeOwned(owner, backend().saveSession(JSON.stringify(buildPersistedSession())));
    if (result.kind !== "current") throw new Error("Graph changed during session save");
    clearLegacyRecentSource(); clearSessionSaveFailure();
  } catch (error) {
    if (owner()) reportSessionSaveFailure(error);
    throw error;
  }
}

export function scheduleSessionSave() {
  sessionIntentRevision++;
  const owner = graphOwner();
  clearTimeout(saveTimer);
  saveTimer = setTimeout(() => {
    if (!owner()) return;
    void writeOwned(owner, backend().saveSession(JSON.stringify(buildPersistedSession())))
      .then((result) => { if (result.kind === "current") { clearLegacyRecentSource(); clearSessionSaveFailure(); } })
      .catch(reportSessionSaveFailure);
  }, 150);
}

/** Best-effort load of this graph's saved session; never rejects. Saved tabs,
 * layout, sidebar and recents apply together only while the UI is the pristine
 * one-pane Journals default and no route or session intent changed during the
 * read. A missing file restores legacy recents; invalid or unreadable data
 * leaves the live UI. Load failure, graph change, or a live change skips registry
 * initialization. Otherwise registry initialization may apply its parked active
 * workspace and schedule a save; a refused recovery or registry error shows a
 * toast. Backend loads may migrate legacy session/registry files. Cost follows
 * session and registry bytes; recovery may write the session file. */
export async function restoreSession(): Promise<void> {
  const owner = graphOwner();
  discardWorkspaceRestoreEvidence();
  const initialSession = JSON.stringify(buildPersistedSession());
  const initialIntent = intentToken();
  const mayApply = () => owner() && JSON.stringify(buildPersistedSession()) === initialSession && intentToken() === initialIntent;
  let initializeRegistry = true;
  try {
    let raw: string | null = null;
    try {
      const result = await readOwned(owner, backend().loadSession());
      if (result.kind === "stale") return;
      raw = result.value;
    } catch {
      initializeRegistry = false;
      return;
    }
    if (!mayApply()) { initializeRegistry = false; return; }
    if (!raw) {
      restoredSessionPresent = false;
      setRecentPages(legacyRecentPages());
      return;
    }
    restoredSessionPresent = true;
    try {
      const id = (JSON.parse(raw) as PersistedSession).workspaceId;
      restoredWorkspaceId = typeof id === "string" && id.length > 0 && id.length <= 128 ? id : null;
    } catch { /* invalid session is handled below */ }
    const parsed = parsePersistedSession(raw);
    if (!parsed) return;
    if (!pristineDefault()) return;
    applyParsedSession(parsed);
  } finally {
    // The registry is graph-scoped, so the early pre-bind restore may fail and
    // the post-bind restore in graph.ts retries it. Keep startup best-effort just
    // like the existing session restore; a bad registry must not block the app.
    try {
      if (!initializeRegistry) return;
      restoreEvidence = { owner, snapshot: JSON.stringify(buildPersistedSession()), intent: intentToken(), present: restoredSessionPresent, workspaceId: restoredWorkspaceId };
      const { initializeWorkspaces } = await import("./workspaces");
      if (!owner()) return;
      await initializeWorkspaces();
    } catch (error) {
      if (owner()) pushToastUnique(`Could not restore workspaces: ${String(error)}`, "error");
    }
  }
}

installSessionPersistence({
  schedule: scheduleSessionSave,
  flush: flushSession,
  restore: restoreSession,
});
