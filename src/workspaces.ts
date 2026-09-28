import { createSignal } from "solid-js";
import { backend } from "./backend";
import { graphOwner, readOwned, serializeDurable, writeOwned, type Owned, type Owner } from "./owned";
import { pushToast } from "./toasts";
import { applyParsedSession, buildPersistedSession, clearRestoredSessionWorkspaceId, flushSession, parsePersistedSession, restoredSessionWorkspaceId, scheduleSessionSave, setSessionWorkspaceId, wasSessionRestored, type PersistedSession } from "./session";

export interface Workspace {
  id: string;
  name: string;
  blob: PersistedSession;
}

export interface WorkspaceRegistry {
  version: 1;
  activeId: string;
  workspaces: Workspace[];
}

const [workspaceList, setWorkspaceList] = createSignal<Workspace[]>([]);
const [activeId, setActiveId] = createSignal("");
export { workspaceList as workspaces, activeId as activeWorkspaceId };

let operationQueue = {};

interface WorkspaceOperation {
  assert: () => void;
  owner: Owner;
  after: <T>(result: Promise<Owned<T>>) => Promise<T>;
}

async function enqueue<T>(operation: (scope: WorkspaceOperation) => Promise<T>): Promise<T> {
  const owner = graphOwner();
  const assert = () => {
    if (!owner()) throw new Error("The graph changed during the workspace operation");
  };
  const scope: WorkspaceOperation = {
    owner,
    assert,
    after: async (promise) => {
      const result = await promise;
      if (result.kind === "stale") throw new Error("The graph changed during the workspace operation");
      assert();
      return result.value;
    },
  };
  const run = () => { assert(); return operation(scope); };
  const result = await serializeDurable(operationQueue, owner, run);
  if (result.kind === "stale") throw new Error("The graph changed during the workspace operation");
  return result.value;
}

function cloneSession(session: PersistedSession): PersistedSession {
  return JSON.parse(JSON.stringify(session)) as PersistedSession;
}

export function defaultWorkspaceSession(): PersistedSession {
  const pane = {
    tabs: [{ history: [{ kind: "journals" as const }], pos: 0, pinned: false }],
    activeIndex: 0,
    scrolls: [null],
  };
  return {
    ...pane,
    leftSidebar: true,
    rightSidebar: false,
    rightSidebarItems: [],
    favoritesSectionExpanded: true,
    recentSectionExpanded: true,
    layout: { kind: "pane", paneId: "main", ...pane },
    focusedPaneId: "main",
    recentPages: [],
  };
}

function normalizeName(name: string): string {
  return name.trim().slice(0, 80);
}

function parseRegistry(raw: string): WorkspaceRegistry | null {
  try {
    const input = JSON.parse(raw) as Partial<WorkspaceRegistry>;
    if (input.version !== 1 || !Array.isArray(input.workspaces)) return null;
    const ids = new Set<string>();
    const valid: Workspace[] = [];
    for (const item of input.workspaces) {
      if (!item || typeof item.id !== "string" || !item.id || item.id.length > 128 || ids.has(item.id)) continue;
      if (typeof item.name !== "string") continue;
      const parsed = parsePersistedSession(JSON.stringify(item.blob));
      if (!parsed) continue;
      ids.add(item.id);
      valid.push({ id: item.id, name: normalizeName(item.name), blob: cloneSession(item.blob) });
    }
    if (!valid.length) return null;
    const requested = typeof input.activeId === "string" ? input.activeId : "";
    return {
      version: 1,
      activeId: ids.has(requested) ? requested : valid[0].id,
      workspaces: valid,
    };
  } catch {
    return null;
  }
}

function registry(): WorkspaceRegistry {
  const list = workspaceList();
  const current = activeId();
  if (!list.length || !list.some((workspace) => workspace.id === current)) {
    throw new Error("No workspace registry is loaded for this graph");
  }
  return { version: 1, activeId: current, workspaces: list };
}

async function persist(next: WorkspaceRegistry, scope: WorkspaceOperation): Promise<void> {
  scope.assert();
  let outcome: "durable" | "published-unsynced";
  try {
    outcome = await scope.after(writeOwned(scope.owner, backend().saveWorkspaces(JSON.stringify(next))));
  } catch (error) {
    if (!scope.owner()) throw error;
    // A transport failure may arrive after publication. Read the serialized
    // registry before another queued operation can build a replacement.
    scope.assert();
    try {
      const loaded = parseRegistry(await scope.after(readOwned(scope.owner, backend().loadWorkspaces())));
      if (loaded) install(loaded);
      else clearWorkspaces();
    } catch {
      scope.assert();
      clearWorkspaces();
    }
    throw error;
  }
  if (outcome === "published-unsynced")
    pushToast("Workspace changes are visible, but directory sync failed; they may not survive a power loss.", "error");
}

function install(next: WorkspaceRegistry) {
  setWorkspaceList(next.workspaces);
  setActiveId(next.activeId);
  setSessionWorkspaceId(next.activeId);
}

function applyWorkspace(workspace: Workspace) {
  const parsed = parsePersistedSession(JSON.stringify(workspace.blob));
  if (!parsed) throw new Error(`Workspace “${workspace.name || "Default"}” is invalid`);
  // Runtime workspace switches intentionally call the audited apply boundary
  // directly. restoreSession()'s pristineDefault() gate is launch-only.
  applyParsedSession(parsed);
  scheduleSessionSave();
}

function workspaceId(): string {
  const uuid = globalThis.crypto?.randomUUID?.();
  return uuid ? `workspace-${uuid}` : `workspace-${Date.now().toString(36)}-${Math.random().toString(36).slice(2)}`;
}

/** Clear the in-memory workspace registry, then load and validate the graph's
 * persisted registry. Use the current live session for its active workspace
 * without rewriting files. Invalid registry or backend failure rejects and
 * leaves the in-memory registry clear. Operations are serialized per graph;
 * cost follows registry and live-session bytes. */
export function initializeWorkspaces(): Promise<void> {
  return enqueue(async (scope) => {
    const restoredId = restoredSessionWorkspaceId();
    const sessionPresent = wasSessionRestored();
    clearWorkspaces();
    const initialSession = JSON.stringify(buildPersistedSession());
    const loaded = parseRegistry(await scope.after(readOwned(scope.owner, backend().loadWorkspaces())));
    if (!loaded) throw new Error("The named-workspace registry is invalid");
    if (sessionPresent === false || (restoredId && restoredId !== loaded.activeId)) {
      if (JSON.stringify(buildPersistedSession()) !== initialSession)
        throw new Error("The session changed while recovering the active workspace");
      install(loaded);
      applyWorkspace(loaded.workspaces.find((workspace) => workspace.id === loaded.activeId)!);
      return;
    }
    // A matching live session is newer than the registry's parked snapshot.
    const current = buildPersistedSession();
    loaded.workspaces = loaded.workspaces.map((workspace) =>
      workspace.id === loaded.activeId ? { ...workspace, blob: current } : workspace
    );
    install(loaded);
  });
}

/** Flush the live session, snapshot it into the active workspace and persist
 * the registry. Session failure toasts and rejects before registry persistence.
 * Registry failure rejects and rereads persisted state when possible. Cost follows
 * session and registry bytes. */
export function saveActiveWorkspace(): Promise<void> {
  return enqueue(async (scope) => {
    await scope.after(writeOwned(scope.owner, flushSession()));
    const current = registry();
    const next: WorkspaceRegistry = {
      ...current,
      workspaces: current.workspaces.map((workspace) =>
        workspace.id === current.activeId
          ? { ...workspace, blob: buildPersistedSession() }
          : workspace
      ),
    };
    await persist(next, scope);
    install(next);
  });
}

/** Flush the current session, save it in the registry, persist the
 * target as active, then apply the target session to the UI. Unknown ID rejects.
 * Registry failure rejects and attempts a disk reread. If applying the target
 * fails after persistence, the persisted active ID may already have changed.
 * Cost follows session and registry bytes. */
export function switchWorkspace(targetId: string): Promise<void> {
  return enqueue(async (scope) => {
    await scope.after(writeOwned(scope.owner, flushSession()));
    const current = registry();
    const target = current.workspaces.find((workspace) => workspace.id === targetId);
    if (!target) throw new Error("Workspace not found");
    const next: WorkspaceRegistry = {
      version: 1,
      activeId: targetId,
      workspaces: current.workspaces.map((workspace) =>
        workspace.id === current.activeId
          ? { ...workspace, blob: buildPersistedSession() }
          : workspace
      ),
    };
    await persist(next, scope);
    install(next);
    if (targetId !== current.activeId) {
      applyWorkspace(next.workspaces.find((workspace) => workspace.id === targetId)!);
    }
  });
}

/** Create and activate a workspace with a default Journals session. The name
 * is trimmed and limited to 80 characters; empty or duplicate names are
 * allowed. Flush the current session and persist the new registry before
 * applying the new workspace UI. Returns its generated ID on success;
 * persistence and graph-change errors reject. Cost follows session and registry
 * bytes. */
export function createWorkspace(name: string): Promise<string> {
  return enqueue(async (scope) => {
    await scope.after(writeOwned(scope.owner, flushSession()));
    const current = registry();
    const id = workspaceId();
    const fresh: Workspace = { id, name: normalizeName(name), blob: defaultWorkspaceSession() };
    const next: WorkspaceRegistry = {
      version: 1,
      activeId: id,
      workspaces: [
        ...current.workspaces.map((workspace) =>
          workspace.id === current.activeId
            ? { ...workspace, blob: buildPersistedSession() }
            : workspace
        ),
        fresh,
      ],
    };
    await persist(next, scope);
    install(next);
    applyWorkspace(fresh);
    return id;
  });
}

export function renameWorkspace(id: string, name: string): Promise<void> {
  return enqueue(async (scope) => {
    const current = registry();
    if (!current.workspaces.some((workspace) => workspace.id === id)) throw new Error("Workspace not found");
    const next = {
      ...current,
      workspaces: current.workspaces.map((workspace) =>
        workspace.id === id ? { ...workspace, name: normalizeName(name) } : workspace
      ),
    };
    await persist(next, scope);
    install(next);
  });
}

/** Delete a workspace from the graph registry. Unknown IDs reject. Deleting
 * the active workspace activates the first survivor; deleting the last
 * creates a new empty-named default workspace instead. Persist the registry
 * before applying replacement UI. Backend errors reject. Cost follows session
 * and registry bytes. */
export function deleteWorkspace(id: string): Promise<void> {
  return enqueue(async (scope) => {
    const current = registry();
    const removed = current.workspaces.find((workspace) => workspace.id === id);
    if (!removed) throw new Error("Workspace not found");
    const deletingActive = id === current.activeId;
    if (deletingActive) await scope.after(writeOwned(scope.owner, flushSession()));
    let remaining = current.workspaces.filter((workspace) => workspace.id !== id);
    if (!remaining.length) {
      remaining = [{ id: workspaceId(), name: "", blob: defaultWorkspaceSession() }];
    }
    const next: WorkspaceRegistry = {
      version: 1,
      activeId: deletingActive ? remaining[0].id : current.activeId,
      workspaces: remaining,
    };
    await persist(next, scope);
    install(next);
    if (deletingActive) applyWorkspace(remaining[0]);
  });
}

export function workspaceDisplayName(workspace: Pick<Workspace, "name">): string {
  return workspace.name || "Default";
}

/** Clear only the reactive workspace list and active ID. Persisted records and
 * the live session remain. Registry operations fail until reinitialization.
 * O(1) signal writes; synchronous, with no disk I/O. */
export function clearWorkspaces() {
  setWorkspaceList([]);
  setActiveId("");
  setSessionWorkspaceId(null);
  clearRestoredSessionWorkspaceId();
}

export function resetWorkspacesForTest() {
  clearWorkspaces();
  operationQueue = {};
}
