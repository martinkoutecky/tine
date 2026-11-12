import { afterAll, beforeAll, expect, it, vi } from "vitest";
import { backend, type GraphConfigChange } from "./backend";
import { cycleMarkerSmart } from "./editor/repeat";
import { setWorkflow, workflow } from "./ui";
import { pushToast, toasts } from "./toasts";
import { msg } from "./i18n";
import type { GraphMeta } from "./types";

const h = vi.hoisted(() => ({
  listeners: new Map<string, (e: { payload: unknown }) => void>(),
  config: undefined as undefined | ((change: GraphConfigChange) => void),
  disposeCapture: undefined as undefined | (() => void),
}));
// capture.tsx mounts at import and discards the dispose handle; keep it so the
// file can own the mount and end it before jsdom is torn down.
vi.mock("solid-js/web", async (original) => {
  const web = await original<typeof import("solid-js/web")>();
  return { ...web, render: ((code, element, init) => {
    const dispose = web.render(code, element, init);
    if ((element as Element).id === "capture-root") h.disposeCapture = dispose;
    return dispose;
  }) as typeof web.render };
});
vi.mock("./components/Block", () => ({ Block: () => <textarea />,
  CaptureCtx: { Provider: (p: { children: unknown }) => p.children },
}));
vi.mock("./editorController", async (original) => ({ ...await original<typeof import("./editorController")>(), startEditing: () => {} }));
vi.mock("./keybindings", () => ({ installKeybindings: () => () => {}, eventToBindingString: () => "" }));
vi.mock("@tauri-apps/api/event", () => ({
  listen: async (name: string, cb: (e: { payload: unknown }) => void) => {
    h.listeners.set(name, cb); return () => h.listeners.delete(name);
  },
  emitTo: async () => {},
}));
vi.mock("@tauri-apps/api/window", () => ({ getCurrentWindow: () => ({
  label: "capture", isVisible: async () => true, onFocusChanged: async () => () => {},
  setSize: async () => {}, setFocus: async () => {},
}) }));

const meta = (wf: string) => ({ root: "/capture-graph", preferred_workflow: wf } as GraphMeta);
beforeAll(async () => {
  setWorkflow("now"); // A separate WebView starts with OG's default.
  vi.spyOn(backend(), "bindCaptureGraph").mockResolvedValue(meta("todo"));
  vi.spyOn(backend(), "graphBindingGeneration").mockReturnValue(17);
  vi.spyOn(backend(), "onGraphConfigChanged").mockImplementation(async cb => { h.config = cb; return () => {}; });
  vi.spyOn(backend(), "getCaptureEnterFiles").mockResolvedValue(false);
  vi.spyOn(backend(), "captureTarget").mockResolvedValue("main");
  const root = document.createElement("div"); root.id = "capture-root"; document.body.append(root);
  await import("./capture");
  await vi.waitFor(() => expect(h.listeners.has("capture-shown")).toBe(true));
  await vi.waitFor(() => expect(backend().bindCaptureGraph).toHaveBeenCalled());
});

// The capture app mounts at import and is never disposed, so its re-fit timers
// (resettle: 80 and 220 ms) can fire after jsdom is torn down and throw
// "document is not defined" as an unhandled error that fails the whole run.
// Let them drain, then dispose the app: with its <Toasts/> renderer gone, a
// toast pushed by any async work still in flight (a settings read, a retry)
// only changes the toast store and cannot touch a document that no longer exists.
afterAll(async () => {
  await new Promise<void>((resolve) => setTimeout(resolve, 300));
  h.disposeCapture!();
  pushToast(msg("toast-copied"), "success");
  expect(document.querySelector(".toast")).toBeNull();
  expect(toasts()).toHaveLength(1);
});

function sequence() {
  let raw = "buy milk";
  return Array.from({ length: 4 }, () => {
    raw = cycleMarkerSmart(raw, workflow(), "md").raw;
    return raw;
  });
}

it("cold Quick Capture uses TODO -> DOING -> DONE -> plain text from its bound graph", async () => {
  await vi.waitFor(() => expect(sequence()).toEqual(["TODO buy milk", "DOING buy milk", "DONE buy milk", "buy milk"]));
});

it("live changes use the same cycle and ignore another graph's config", async () => {
  expect(h.config).toBeTypeOf("function");
  h.config!({ binding_generation: 17, meta: meta("now") });
  expect(sequence()).toEqual(["LATER buy milk", "NOW buy milk", "DONE buy milk", "buy milk"]);
  h.config!({ binding_generation: 18, meta: meta("todo") });
  expect(sequence()[0]).toBe("LATER buy milk");
  h.config!({ binding_generation: 17, meta: meta("todo") });
  expect(sequence()).toEqual(["TODO buy milk", "DOING buy milk", "DONE buy milk", "buy milk"]);
});

it("reopening refreshes the workflow changed while hidden", async () => {
  vi.mocked(backend().bindCaptureGraph).mockResolvedValue(meta("now"));
  h.listeners.get("capture-shown")!({ payload: null });
  await vi.waitFor(() => expect(sequence()).toEqual(["LATER buy milk", "NOW buy milk", "DONE buy milk", "buy milk"]));
});

it("a delayed show snapshot cannot overwrite a newer live preference", async () => {
  let resolve!: (value: GraphMeta) => void;
  vi.mocked(backend().bindCaptureGraph).mockImplementationOnce(() => new Promise(done => { resolve = done; }));
  h.listeners.get("capture-shown")!({ payload: null });
  await vi.waitFor(() => expect(resolve).toBeTypeOf("function"));
  h.config!({ binding_generation: 17, meta: meta("todo") });
  resolve(meta("now"));
  await new Promise(done => setTimeout(done, 0));
  expect(sequence()).toEqual(["TODO buy milk", "DOING buy milk", "DONE buy milk", "buy milk"]);
});
