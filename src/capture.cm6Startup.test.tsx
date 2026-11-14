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

it("waits for a stored classic choice before preloading hidden Quick Capture, then preloads on toggle-on", async () => {
  const { cm6EditorEnabled, cm6EditorLoading, setCm6EditorEnabled } = await import("./editor/cm6Setting");
  // Begin with the production default, even in the explicit-classic render suite.
  setCm6EditorEnabled(true);
  await new Promise(resolve => setTimeout(resolve, 0));
  let finish!: (value: boolean) => void;
  const stored = new Promise<boolean>(resolve => { finish = resolve; });
  vi.spyOn(backend(), "getAppBool").mockImplementation(async (key, fallback) => key === "experimental_cm6_editor" ? stored : fallback);
  vi.spyOn(backend(), "setAppBool").mockResolvedValue();
  vi.spyOn(backend(), "getCaptureEnterFiles").mockResolvedValue(false);
  vi.spyOn(backend(), "captureTarget").mockResolvedValue("main");
  vi.spyOn(backend(), "onGraphConfigChanged").mockResolvedValue(() => {});
  const root = document.createElement("div"); root.id = "capture-root"; document.body.append(root);
  await import("./capture");
  await vi.waitFor(() => expect(h.listeners.has("capture-shown")).toBe(true));
  expect(cm6EditorLoading()).toBe(true);
  expect(h.preload).not.toHaveBeenCalled();
  finish(false);
  await vi.waitFor(() => expect(cm6EditorLoading()).toBe(false));
  expect(cm6EditorEnabled()).toBe(false);
  expect(h.preload).not.toHaveBeenCalled();
  expect(backend().getAppBool).toHaveBeenCalledWith("experimental_cm6_editor", true);
  setCm6EditorEnabled(true);
  expect(h.preload).toHaveBeenCalledTimes(1);
  setCm6EditorEnabled(false);
  expect(h.preload).toHaveBeenCalledTimes(1);
  setCm6EditorEnabled(true);
  expect(h.preload).toHaveBeenCalledTimes(2);
});
