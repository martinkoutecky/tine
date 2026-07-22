import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { render } from "solid-js/web";
import { backend } from "../backend";
import { resetPaneLayoutToSingle } from "../panes";
import { buildPersistedSession } from "../session";
import {
  activeWorkspaceId,
  createWorkspace,
  initializeWorkspaces,
  resetWorkspacesForTest,
  workspaces,
} from "../workspaces";
import { WorkspaceSwitcher } from "./WorkspaceSwitcher";

let dispose = () => {};

beforeEach(async () => {
  resetPaneLayoutToSingle({
    tabs: [{ history: [{ kind: "page", name: "Alpha page", pageKind: "page" }], pos: 0, pinned: false }],
    activeIndex: 0,
  });
  resetWorkspacesForTest();
  vi.spyOn(backend(), "loadWorkspaces").mockResolvedValue(JSON.stringify({
    version: 1,
    activeId: "default",
    workspaces: [{ id: "default", name: "Alpha", blob: buildPersistedSession() }],
  }));
  vi.spyOn(backend(), "saveWorkspaces").mockResolvedValue();
  vi.spyOn(backend(), "saveSession").mockResolvedValue();
  await initializeWorkspaces();
  await createWorkspace("Beta");
});

afterEach(() => {
  dispose();
  dispose = () => {};
  document.body.innerHTML = "";
  resetWorkspacesForTest();
  vi.restoreAllMocks();
});

describe("WorkspaceSwitcher", () => {
  it("renders the collapsed-sidebar fallback as the compact W control without a workspace label", () => {
    const host = document.createElement("div");
    document.body.appendChild(host);
    dispose = render(() => <WorkspaceSwitcher compact />, host);

    const root = host.querySelector<HTMLElement>('[data-workspace-switcher-compact="true"]')!;
    expect(root.querySelector(".workspace-switcher-name")).toBeNull();
    expect(root.querySelector(".workspace-switcher-mark")?.textContent).toBe("W");
    expect(root.querySelector(".workspace-switcher-caret")?.textContent).toBe("▾");
  });

  it("offers hover quick-switch and a click menu with new, rename, and confirmed delete actions", async () => {
    const host = document.createElement("div");
    document.body.appendChild(host);
    dispose = render(() => <WorkspaceSwitcher />, host);
    const root = host.querySelector<HTMLElement>("[data-workspace-switcher]")!;
    const trigger = root.querySelector<HTMLButtonElement>(".workspace-switcher-btn")!;

    root.dispatchEvent(new MouseEvent("mouseenter", { bubbles: false }));
    expect(root.querySelector(".workspace-quick-menu")?.textContent).toContain("Alpha");
    expect(root.querySelector(".workspace-quick-menu")?.textContent).toContain("Beta");

    trigger.click();
    expect(trigger.getAttribute("aria-expanded")).toBe("true");
    expect(root.querySelector(".workspace-quick-menu")).toBeNull();
    const menu = root.querySelector<HTMLElement>(".workspace-menu")!;
    expect(menu.textContent).toContain("+ New workspace");
    expect(menu.textContent).toContain("Rename");
    expect(menu.textContent).toContain("Delete");

    menu.querySelector<HTMLButtonElement>('[aria-label="Rename Alpha"]')!.click();
    expect(menu.querySelector<HTMLInputElement>('[aria-label="Workspace name"]')?.value).toBe("Alpha");

    const confirm = vi.spyOn(backend(), "confirm").mockResolvedValue(true);
    menu.querySelector<HTMLButtonElement>('[aria-label="Delete Alpha"]')!.click();
    await vi.waitFor(() => expect(workspaces()).toHaveLength(1));
    expect(confirm).toHaveBeenCalledWith("Delete workspace “Alpha”?", "Delete workspace");
    expect(activeWorkspaceId()).not.toBe("default");
  });
});
