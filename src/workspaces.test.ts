import { beforeEach, describe, expect, it, vi } from "vitest";
import { backend } from "./backend";
import { layoutPaneIds, layoutRoot, paneRouter, resetPaneLayoutToSingle, restorePaneLayout } from "./panes";
import type { PaneSnapshot } from "./router";
import { buildPersistedSession } from "./session";
import { resetStore } from "./document";
import { applySidebarSession, rightSidebar } from "./ui";
import {
  activeWorkspaceId,
  createWorkspace,
  deleteWorkspace,
  initializeWorkspaces,
  renameWorkspace,
  resetWorkspacesForTest,
  saveActiveWorkspace,
  switchWorkspace,
  workspaces,
} from "./workspaces";

const journals = (): PaneSnapshot => ({
  tabs: [{ history: [{ kind: "journals" }], pos: 0, pinned: false }],
  activeIndex: 0,
});

const pages = (names: string[], activeIndex = 0): PaneSnapshot => ({
  tabs: names.map((name) => ({
    history: [{ kind: "page", name, pageKind: "page" }],
    pos: 0,
    pinned: false,
  })),
  activeIndex,
});

function registryFromCurrent() {
  return JSON.stringify({
    version: 1,
    activeId: "default",
    workspaces: [{ id: "default", name: "", blob: buildPersistedSession() }],
  });
}

beforeEach(() => {
  resetPaneLayoutToSingle(journals());
  applySidebarSession({ right: false, items: [] });
  resetWorkspacesForTest();
  vi.restoreAllMocks();
});

describe("named workspace switching", () => {
  it("reloads a published registry after a post-rename sync failure before the next write", async () => {
    let disk = registryFromCurrent();
    let failOnce = true;
    vi.spyOn(backend(), "loadWorkspaces").mockImplementation(async () => disk);
    vi.spyOn(backend(), "saveWorkspaces").mockImplementation(async (raw) => {
      disk = raw;
      if (failOnce && JSON.parse(raw).workspaces[0].name === "First") {
        failOnce = false;
        throw new Error("injected directory sync I/O failure");
      }
      return "durable";
    });
    await initializeWorkspaces();
    await expect(renameWorkspace("default", "First")).rejects.toThrow("injected directory sync");
    await createWorkspace("Second");
    expect(JSON.parse(disk).workspaces.map((workspace: { name: string }) => workspace.name)).toEqual(["First", "Second"]);
  });
  it("installs a visible registry even when its directory sync failed", async () => {
    vi.spyOn(backend(), "loadWorkspaces").mockResolvedValue(registryFromCurrent());
    const save = vi.spyOn(backend(), "saveWorkspaces")
      .mockResolvedValueOnce("published-unsynced")
      .mockResolvedValue("durable");
    await initializeWorkspaces();
    await renameWorkspace("default", "First");
    expect(workspaces()[0].name).toBe("First");
    await createWorkspace("Second");
    expect(JSON.parse(save.mock.calls[1][0]).workspaces.map((workspace: { name: string }) => workspace.name)).toEqual(["First", "Second"]);
  });
  it("does not install an old graph registry after its read finishes on another graph", async () => {
    let finish!: (raw: string) => void;
    vi.spyOn(backend(), "loadWorkspaces").mockImplementationOnce(() => new Promise((resolve) => { finish = resolve; }));
    const pending = initializeWorkspaces();
    await Promise.resolve();
    resetStore();
    finish(registryFromCurrent());
    await expect(pending).rejects.toThrow(/graph/i);
    expect(workspaces()).toEqual([]);
  });

  it("clears the old registry when the new graph registry cannot be read", async () => {
    vi.spyOn(backend(), "loadWorkspaces").mockResolvedValueOnce(registryFromCurrent()).mockRejectedValueOnce(new Error("unreadable"));
    await initializeWorkspaces();
    resetStore();
    await expect(initializeWorkspaces()).rejects.toThrow("unreadable");
    expect(workspaces()).toEqual([]);
    expect(activeWorkspaceId()).toBe("");
  });

  it("does not persist a workspace switch queued before a graph reset", async () => {
    vi.spyOn(backend(), "loadWorkspaces").mockResolvedValue(registryFromCurrent());
    const save = vi.spyOn(backend(), "saveWorkspaces").mockResolvedValue("durable");
    await initializeWorkspaces();
    const pending = switchWorkspace("default");
    resetStore();
    await expect(pending).rejects.toThrow(/graph/i);
    expect(save).not.toHaveBeenCalled();
  });

  it("restores the first workspace's routed tabs and split layout after creating and using a second", async () => {
    restorePaneLayout(
      {
        kind: "split",
        dir: "row",
        ratio: 0.4,
        children: [
          { kind: "pane", paneId: "main" },
          { kind: "pane", paneId: "research" },
        ],
      },
      new Map([
        ["main", pages(["Project alpha", "Decisions"], 1)],
        ["research", pages(["Source notes"])],
      ]),
      "research"
    );
    vi.spyOn(backend(), "loadWorkspaces").mockResolvedValue(registryFromCurrent());
    vi.spyOn(backend(), "saveWorkspaces").mockResolvedValue("durable");
    vi.spyOn(backend(), "saveSession").mockResolvedValue();

    await initializeWorkspaces();
    await renameWorkspace("default", "Alpha");
    const secondId = await createWorkspace("Beta");
    expect(layoutPaneIds()).toEqual(["main"]);
    expect(paneRouter("main").snapshot().tabs[0].history[0]).toEqual({ kind: "journals" });

    resetPaneLayoutToSingle(pages(["Project beta"]));
    await switchWorkspace("default");

    expect(activeWorkspaceId()).toBe("default");
    expect(layoutRoot()).toEqual({
      kind: "split",
      dir: "row",
      ratio: 0.4,
      children: [
        { kind: "pane", paneId: "main" },
        { kind: "pane", paneId: "research" },
      ],
    });
    expect(paneRouter("main").snapshot().tabs.map((tab) => tab.history[tab.pos])).toEqual([
      { kind: "page", name: "Project alpha", pageKind: "page" },
      { kind: "page", name: "Decisions", pageKind: "page" },
    ]);
    expect(paneRouter("research").snapshot().tabs[0].history[0]).toEqual({
      kind: "page",
      name: "Source notes",
      pageKind: "page",
    });
    expect(workspaces().map(({ id, name }) => ({ id, name }))).toEqual([
      { id: "default", name: "Alpha" },
      { id: secondId, name: "Beta" },
    ]);
  });

  it("never calls a graph writer across save, switch, new, rename, and delete", async () => {
    resetPaneLayoutToSingle(pages(["Byte-identical page"]));
    vi.spyOn(backend(), "loadWorkspaces").mockResolvedValue(registryFromCurrent());
    vi.spyOn(backend(), "saveWorkspaces").mockResolvedValue("durable");
    vi.spyOn(backend(), "saveSession").mockResolvedValue();
    const savePages = vi.spyOn(backend(), "savePages");

    await initializeWorkspaces();
    await saveActiveWorkspace();
    await renameWorkspace("default", "One");
    const secondId = await createWorkspace("Two");
    await switchWorkspace("default");
    await switchWorkspace(secondId);
    await deleteWorkspace(secondId);

    expect(savePages).not.toHaveBeenCalled();
    expect(workspaces()).toHaveLength(1);
    expect(activeWorkspaceId()).toBe("default");

    await deleteWorkspace("default");
    expect(workspaces()).toHaveLength(1);
    expect(activeWorkspaceId()).toBe(workspaces()[0].id);
    expect(activeWorkspaceId()).not.toBe("default");
  });

  it("restores parked sidebar references directly without stamping an id into the graph", async () => {
    const current = buildPersistedSession();
    const parked = {
      ...current,
      rightSidebar: true,
      rightSidebarItems: [{
        kind: "block" as const,
        uuid: "11111111-1111-4111-8111-111111111111",
        page: "Referenced page",
        pageKind: "page" as const,
      }],
    };
    vi.spyOn(backend(), "loadWorkspaces").mockResolvedValue(JSON.stringify({
      version: 1,
      activeId: "default",
      workspaces: [
        { id: "default", name: "One", blob: current },
        { id: "parked", name: "Parked", blob: parked },
      ],
    }));
    vi.spyOn(backend(), "saveWorkspaces").mockResolvedValue("durable");
    vi.spyOn(backend(), "saveSession").mockResolvedValue();
    const savePages = vi.spyOn(backend(), "savePages");

    await initializeWorkspaces();
    await switchWorkspace("parked");

    expect(rightSidebar()).toEqual(parked.rightSidebarItems);
    expect(savePages).not.toHaveBeenCalled();
  });
});
