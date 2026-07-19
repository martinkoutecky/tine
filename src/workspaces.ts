import { createSignal } from "solid-js";
import { backend } from "./backend";
import {
  applyParsedSession,
  buildPersistedSession,
  flushSession,
  parsePersistedSession,
  scheduleSessionSave,
  type PersistedSession,
} from "./session";

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

let operationTail: Promise<void> = Promise.resolve();

function enqueue<T>(operation: () => Promise<T>): Promise<T> {
  const result = operationTail.then(operation, operation);
  operationTail = result.then(() => undefined, () => undefined);
  return result;
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

async function persist(next: WorkspaceRegistry): Promise<void> {
  await backend().saveWorkspaces(JSON.stringify(next));
}

function install(next: WorkspaceRegistry) {
  setWorkspaceList(next.workspaces);
  setActiveId(next.activeId);
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

export function initializeWorkspaces(): Promise<void> {
  return enqueue(async () => {
    const loaded = parseRegistry(await backend().loadWorkspaces());
    if (!loaded) throw new Error("The named-workspace registry is invalid");
    // The unchanged live session file is authoritative for the active workspace
    // on launch. Keep its freshest state in memory without rewriting either file.
    const current = buildPersistedSession();
    loaded.workspaces = loaded.workspaces.map((workspace) =>
      workspace.id === loaded.activeId ? { ...workspace, blob: current } : workspace
    );
    install(loaded);
  });
}

export function saveActiveWorkspace(): Promise<void> {
  return enqueue(async () => {
    await flushSession();
    const current = registry();
    const next: WorkspaceRegistry = {
      ...current,
      workspaces: current.workspaces.map((workspace) =>
        workspace.id === current.activeId
          ? { ...workspace, blob: buildPersistedSession() }
          : workspace
      ),
    };
    await persist(next);
    install(next);
  });
}

export function switchWorkspace(targetId: string): Promise<void> {
  return enqueue(async () => {
    await flushSession();
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
    await persist(next);
    install(next);
    if (targetId !== current.activeId) {
      applyWorkspace(next.workspaces.find((workspace) => workspace.id === targetId)!);
    }
  });
}

export function createWorkspace(name: string): Promise<string> {
  return enqueue(async () => {
    await flushSession();
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
    await persist(next);
    install(next);
    applyWorkspace(fresh);
    return id;
  });
}

export function renameWorkspace(id: string, name: string): Promise<void> {
  return enqueue(async () => {
    const current = registry();
    if (!current.workspaces.some((workspace) => workspace.id === id)) throw new Error("Workspace not found");
    const next = {
      ...current,
      workspaces: current.workspaces.map((workspace) =>
        workspace.id === id ? { ...workspace, name: normalizeName(name) } : workspace
      ),
    };
    await persist(next);
    install(next);
  });
}

export function deleteWorkspace(id: string): Promise<void> {
  return enqueue(async () => {
    const current = registry();
    const removed = current.workspaces.find((workspace) => workspace.id === id);
    if (!removed) throw new Error("Workspace not found");
    const deletingActive = id === current.activeId;
    if (deletingActive) await flushSession();
    let remaining = current.workspaces.filter((workspace) => workspace.id !== id);
    if (!remaining.length) {
      remaining = [{ id: workspaceId(), name: "", blob: defaultWorkspaceSession() }];
    }
    const next: WorkspaceRegistry = {
      version: 1,
      activeId: deletingActive ? remaining[0].id : current.activeId,
      workspaces: remaining,
    };
    await persist(next);
    install(next);
    if (deletingActive) applyWorkspace(remaining[0]);
  });
}

export function workspaceDisplayName(workspace: Pick<Workspace, "name">): string {
  return workspace.name || "Default";
}

export function resetWorkspacesForTest() {
  setWorkspaceList([]);
  setActiveId("");
  operationTail = Promise.resolve();
}
