import { afterEach, expect, it, vi } from "vitest";
import { render } from "solid-js/web";
import { App } from "./App";
import { commandDefaults, installKeybindings } from "./keybindings";
import { setLeftSidebarOpen } from "./ui";
import { initializeWorkspaces, resetWorkspacesForTest } from "./workspaces";

const ipc = vi.hoisted(() => ({ invoke: vi.fn().mockResolvedValue(undefined) }));
const chrome = vi.hoisted(() => ({ osControls: false }));
vi.mock("./nativeChrome", async (original) => ({
  ...await original<typeof import("./nativeChrome")>(),
  osDrawsWindowControls: () => chrome.osControls,
}));
// Only the shortcut needs a native host here; App keeps its mock backend.
vi.mock("./appQuit", () => ({ requestAppQuit: () => ipc.invoke("request_app_quit") }));

let dispose = () => {};
let uninstall = () => {};
afterEach(() => {
  uninstall(); dispose(); ipc.invoke.mockClear();
  resetWorkspacesForTest();
  chrome.osControls = false;
  document.body.innerHTML = "";
});

it("offers remappable Quit Tine and dispatches Ctrl+Q while editing", async () => {
  expect(commandDefaults().find(c => c.id === "app/quit")).toMatchObject({ label: "Quit Tine", binding: "mod+q" });
  const editor = document.body.appendChild(document.createElement("textarea"));
  editor.focus();
  uninstall = installKeybindings();
  editor.dispatchEvent(new KeyboardEvent("keydown", { key: "q", code: "KeyQ", ctrlKey: true, bubbles: true }));
  expect(ipc.invoke).toHaveBeenCalledOnce();
  expect(ipc.invoke).toHaveBeenCalledWith("request_app_quit");
  uninstall(); ipc.invoke.mockClear();
  uninstall = installKeybindings({ "app/quit": "mod+shift+q" });
  editor.dispatchEvent(new KeyboardEvent("keydown", { key: "q", ctrlKey: true, bubbles: true }));
  expect(ipc.invoke).not.toHaveBeenCalled();
  editor.dispatchEvent(new KeyboardEvent("keydown", { key: "Q", ctrlKey: true, shiftKey: true, bubbles: true }));
  expect(ipc.invoke).toHaveBeenCalledOnce();
  uninstall(); ipc.invoke.mockClear();
  uninstall = installKeybindings({ "app/quit": "false" });
  editor.dispatchEvent(new KeyboardEvent("keydown", { key: "q", ctrlKey: true, bubbles: true }));
  expect(ipc.invoke).not.toHaveBeenCalled();
});

it("marks only the sidebar header background draggable and keeps its switcher clickable", async () => {
  setLeftSidebarOpen(true);
  await initializeWorkspaces();
  const host = document.body.appendChild(document.createElement("div"));
  dispose = render(() => <App />, host);
  const header = host.querySelector<HTMLElement>("[data-workspace-switcher-sidebar]")!;
  expect(header.getAttribute("data-tauri-drag-region")).toBe("");
  const button = await vi.waitFor(() => {
    const found = header.querySelector<HTMLButtonElement>(".workspace-switcher-btn");
    expect(found).not.toBeNull();
    expect(found!.disabled).toBe(false);
    return found!;
  });
  expect(button.hasAttribute("data-tauri-drag-region")).toBe(false);
  button.click();
  await vi.waitFor(() => expect(button.getAttribute("aria-expanded")).toBe("true"));
});

it("does not add the drag region where the OS draws the window controls", () => {
  chrome.osControls = true;
  setLeftSidebarOpen(true);
  const host = document.body.appendChild(document.createElement("div"));
  dispose = render(() => <App />, host);
  expect(host.querySelector("[data-workspace-switcher-sidebar]")!.hasAttribute("data-tauri-drag-region")).toBe(false);
});
