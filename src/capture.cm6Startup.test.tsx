import { afterAll, expect, it, vi } from "vitest";
import { backend } from "./backend";

const h = vi.hoisted(() => ({ preload: vi.fn(async () => {}), listeners: new Set<string>() }));
vi.mock("./appIdentity", () => ({ APP_UPDATE_CHANNEL: "beta", APP_PRODUCT_NAME: "Tine" }));
vi.unmock("./editor/cm6Setting");
vi.mock("./components/EditorGate", () => ({ preloadCm6Editor: h.preload }));
vi.mock("./components/Block", () => ({ Block: () => null,
  CaptureCtx: { Provider: (props: { children: unknown }) => props.children },
}));
vi.mock("./editorController", async original => ({ ...await original<typeof import("./editorController")>(), startEditing: () => {} }));
vi.mock("./keybindings", () => ({ installKeybindings: () => () => {}, eventToBindingString: () => "" }));
vi.mock("@tauri-apps/api/event", () => ({
  listen: async (name: string) => { h.listeners.add(name); return () => h.listeners.delete(name); },
  emitTo: async () => {}, emit: async () => {},
}));
vi.mock("@tauri-apps/api/window", () => ({ getCurrentWindow: () => ({
  label: "capture", isVisible: async () => false, onFocusChanged: async () => () => {},
  setSize: async () => {},
}) }));

// Capture owns its root for the life of this WebView; drain its layout timers.
afterAll(() => new Promise<void>(resolve => setTimeout(resolve, 300)));

it("preloads the stored CM6 choice while Quick Capture is still hidden and starts loading on toggle-on", async () => {
  vi.spyOn(backend(), "getAppBool").mockImplementation(async (key, fallback) => key === "experimental_cm6_editor" || fallback);
  vi.spyOn(backend(), "setAppBool").mockResolvedValue();
  vi.spyOn(backend(), "getCaptureEnterFiles").mockResolvedValue(false);
  vi.spyOn(backend(), "captureTarget").mockResolvedValue("main");
  vi.spyOn(backend(), "onGraphConfigChanged").mockResolvedValue(() => {});
  const root = document.createElement("div"); root.id = "capture-root"; document.body.append(root);
  await import("./capture");
  await vi.waitFor(() => expect(h.listeners.has("capture-shown")).toBe(true));
  await vi.waitFor(() => expect(h.preload).toHaveBeenCalledTimes(1));
  expect(backend().getAppBool).toHaveBeenCalledWith("experimental_cm6_editor", false);
  const { setCm6EditorEnabled } = await import("./editor/cm6Setting");
  setCm6EditorEnabled(false);
  expect(h.preload).toHaveBeenCalledTimes(1);
  setCm6EditorEnabled(true);
  expect(h.preload).toHaveBeenCalledTimes(2);
});
