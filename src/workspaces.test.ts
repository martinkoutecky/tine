import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { backend } from "./backend";
import { layoutPaneIds, layoutRoot, paneRouter, resetPaneLayoutToSingle, restorePaneLayout } from "./panes";
import type { PaneSnapshot } from "./router";
import { buildPersistedSession } from "./session";
import { applySidebarSession, pdfTarget, rightSidebar, setPdfTarget } from "./ui";
import {
  activatePdfOwnership,
  registerPdfParticipant,
  resetPdfOwnershipForTest,
  type PdfOwnership,
} from "./pdfOwnership";
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

let pdfOwner: PdfOwnership;

beforeEach(() => {
  resetPaneLayoutToSingle(journals());
  applySidebarSession({ right: false, items: [] });
  resetWorkspacesForTest();
  resetPdfOwnershipForTest();
  pdfOwner = activatePdfOwnership("/test/workspace-graph");
  vi.restoreAllMocks();
});

afterEach(() => {
  setPdfTarget(null);
  resetPdfOwnershipForTest();
});

describe("named workspace switching", () => {
  it("drains PDF work before replacing a layout", async () => {
    const current = buildPersistedSession();
    const target = { ...current, focusedPaneId: "main" };
    const events: string[] = [];
    vi.spyOn(backend(), "loadWorkspaces").mockResolvedValue(JSON.stringify({
      version: 1,
      activeId: "default",
      workspaces: [
        { id: "default", name: "", blob: current },
        { id: "target", name: "Target", blob: target },
      ],
    }));
    vi.spyOn(backend(), "saveSession").mockImplementation(async () => { events.push("session"); });
    vi.spyOn(backend(), "saveWorkspaces").mockImplementation(async () => { events.push("registry"); });
    registerPdfParticipant(pdfOwner, {
      flush: async () => { events.push("pdf"); return true; },
      cancel: vi.fn(),
    });

    await initializeWorkspaces();
    await switchWorkspace("target");

    expect(events).toEqual(["session", "pdf", "registry"]);
  });

  it("aborts switch, create, and active deletion before registry or layout mutation when PDF drain fails", async () => {
    const current = buildPersistedSession();
    const target = { ...current };
    const saveRegistry = vi.spyOn(backend(), "saveWorkspaces").mockResolvedValue();
    vi.spyOn(backend(), "saveSession").mockResolvedValue();
    vi.spyOn(backend(), "loadWorkspaces").mockResolvedValue(JSON.stringify({
      version: 1,
      activeId: "default",
      workspaces: [
        { id: "default", name: "", blob: current },
        { id: "target", name: "Target", blob: target },
      ],
    }));
    await initializeWorkspaces();
    const beforeRegistry = workspaces();
    const beforeLayout = layoutRoot();
    registerPdfParticipant(pdfOwner, {
      flush: async () => false,
      cancel: vi.fn(),
    });

    await expect(switchWorkspace("target")).rejects.toThrow("pending PDF changes");
    await expect(createWorkspace("Blocked")).rejects.toThrow("pending PDF changes");
    await expect(deleteWorkspace("default")).rejects.toThrow("pending PDF changes");

    expect(saveRegistry).not.toHaveBeenCalled();
    expect(activeWorkspaceId()).toBe("default");
    expect(workspaces()).toEqual(beforeRegistry);
    expect(layoutRoot()).toEqual(beforeLayout);
  });

  it("keeps independent PDF open/closed identity in each named workspace", async () => {
    setPdfTarget({ filename: "assets/alpha.pdf", label: "Alpha", owner: pdfOwner, page: 8 });
    vi.spyOn(backend(), "loadWorkspaces").mockResolvedValue(registryFromCurrent());
    vi.spyOn(backend(), "saveWorkspaces").mockResolvedValue();
    vi.spyOn(backend(), "saveSession").mockResolvedValue();

    await initializeWorkspaces();
    const beta = await createWorkspace("Beta");
    expect(pdfTarget()).toBeNull();

    setPdfTarget({ filename: "assets/beta.pdf", label: "Beta", owner: pdfOwner, highlightId: "hl" });
    await switchWorkspace("default");
    expect(pdfTarget()).toEqual({ filename: "assets/alpha.pdf", label: "Alpha", owner: pdfOwner });

    await switchWorkspace(beta);
    expect(pdfTarget()).toEqual({ filename: "assets/beta.pdf", label: "Beta", owner: pdfOwner });
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
    vi.spyOn(backend(), "saveWorkspaces").mockResolvedValue();
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
    vi.spyOn(backend(), "saveWorkspaces").mockResolvedValue();
    vi.spyOn(backend(), "saveSession").mockResolvedValue();
    const savePage = vi.spyOn(backend(), "savePage");

    await initializeWorkspaces();
    await saveActiveWorkspace();
    await renameWorkspace("default", "One");
    const secondId = await createWorkspace("Two");
    await switchWorkspace("default");
    await switchWorkspace(secondId);
    await deleteWorkspace(secondId);

    expect(savePage).not.toHaveBeenCalled();
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
    vi.spyOn(backend(), "saveWorkspaces").mockResolvedValue();
    vi.spyOn(backend(), "saveSession").mockResolvedValue();
    const savePage = vi.spyOn(backend(), "savePage");

    await initializeWorkspaces();
    await switchWorkspace("parked");

    expect(rightSidebar()).toEqual(parked.rightSidebarItems);
    expect(savePage).not.toHaveBeenCalled();
  });
});
