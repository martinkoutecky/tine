// UI-CAPTURE-NO-PAGE-INVENTORY: the Quick Capture WebView is granted no
// graph-wide read (src-tauri state.rs: capture has no graph slot, only narrow
// capture-scoped commands). Typing a page reference there asks the shared page
// index for a name (completion row, link rendering). That must not issue the
// refused `page_inventory` read and raise "Couldn't refresh the page list" in
// the capture window.
import { beforeAll, expect, it, vi } from "vitest";
import { backend } from "./backend";
import { allPageNames, navigationName, resolvedTarget } from "./pageIndex";
import { toasts } from "./toasts";
import { initParser } from "./render/parse";

vi.mock("./components/Block", () => ({ Block: () => null,
  CaptureCtx: { Provider: (p: { children: unknown }) => p.children },
}));
vi.mock("./editorController", async (original) => ({ ...await original<typeof import("./editorController")>(), startEditing: () => {} }));
vi.mock("./keybindings", () => ({ installKeybindings: () => () => {}, eventToBindingString: () => "" }));
vi.mock("@tauri-apps/api/event", () => ({ listen: async () => () => {}, emitTo: async () => {} }));
vi.mock("@tauri-apps/api/window", () => ({ getCurrentWindow: () => ({
  label: "capture", isVisible: async () => false, onFocusChanged: async () => () => {}, hide: vi.fn(),
}) }));

const inventory = vi.fn(async () => { throw new Error("no graph loaded for window capture"); });

beforeAll(async () => {
  await initParser();
  vi.spyOn(backend(), "bindCaptureGraph").mockResolvedValue(undefined);
  vi.spyOn(backend(), "getCaptureEnterFiles").mockResolvedValue(false);
  vi.spyOn(backend(), "captureTarget").mockResolvedValue("main");
  vi.spyOn(backend(), "graphBindingGeneration").mockReturnValue(17);
  vi.spyOn(backend(), "pageInventory").mockImplementation(inventory);
  const root = document.createElement("div"); root.id = "capture-root"; document.body.append(root);
  await import("./capture");
  await vi.waitFor(() => expect(document.querySelector(".capture-shell")).not.toBeNull());
});

it("UI-CAPTURE-NO-PAGE-INVENTORY: page-name lookups in Quick Capture never read the refused inventory", async () => {
  // What a `[[Fz` completion row and a rendered `[[Fuzzy Existing]]` link ask.
  expect(navigationName("Fuzzy Existing")).toBe("Fuzzy Existing");
  expect(resolvedTarget("Fuzzy Existing")).toBeUndefined();
  expect(allPageNames()).toEqual([]);
  for (let i = 0; i < 5; i++) await new Promise((resolve) => setTimeout(resolve, 0));
  expect(inventory).not.toHaveBeenCalled();
  expect(toasts().filter((toast) => toast.kind === "error").map((toast) => toast.message)).toEqual([]);
});
