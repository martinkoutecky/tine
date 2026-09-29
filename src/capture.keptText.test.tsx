// GH #254 family (og J1, manager decision Q1): a capture the main window refuses
// (another file holding today's name has unsaved input) is acknowledged as not
// saved, and the capture window keeps the text in its editor and says so, so it
// can be submitted again once the name is free.
import { expect, it, vi } from "vitest";
import { backend } from "./backend";

const h = vi.hoisted(() => ({
  listeners: new Map<string, (event: { payload?: unknown }) => void>(),
  captureApi: null as null | { submit: () => void },
  emitted: [] as Array<{ target: string; event: string; payload: unknown }>,
  setRaw: [] as unknown[],
  deleted: [] as unknown[],
  markdown: "- captured thought",
  hide: vi.fn(),
}));

vi.mock("./render/parse", () => ({ initParser: async () => {} }));
vi.mock("./captureSeed", () => ({ CAPTURE_SCRATCH_NAME: "___Capture Scratch___", createCaptureScratchPage: () => ({}) }));
vi.mock("./components/Block", () => ({
  Block: () => null,
  CaptureCtx: { Provider: (props: { value: { submit: () => void }; children: unknown }) => {
    h.captureApi = props.value;
    return props.children;
  } },
}));
vi.mock("./components/DatePicker", () => ({ DatePicker: () => null }));
vi.mock("./document", () => ({
  ensurePageLoaded: () => null, pageByName: () => ({ roots: ["scratch-root"] }),
  blockSubtreeMarkdown: () => h.markdown,
  deleteBlock: (id: unknown) => { h.deleted.push(id); }, setRaw: (id: unknown) => { h.setRaw.push(id); }, node: () => null,
}));
vi.mock("./editorController", () => ({ startEditing: () => {} }));
vi.mock("./keybindings", () => ({ installKeybindings: () => () => {}, eventToBindingString: () => "" }));
vi.mock("./ui", () => ({ datePicker: () => null }));
vi.mock("./spellcheckSettings", () => ({ initSpellcheckSettings: async () => {} }));
vi.mock("./refCompletionSettings", () => ({ initRefCompletionSettings: async () => {} }));
vi.mock("./lsShimInstall", () => ({}));
vi.mock("@tauri-apps/api/event", () => ({
  listen: async (name: string, listener: (event: { payload?: unknown }) => void) => {
    h.listeners.set(name, listener);
    return () => h.listeners.delete(name);
  },
  emitTo: async (target: string, event: string, payload: unknown) => { h.emitted.push({ target, event, payload }); },
}));
vi.mock("@tauri-apps/api/window", () => ({
  getCurrentWindow: () => ({ label: "capture", isVisible: async () => false,
    onFocusChanged: async () => () => {}, hide: h.hide }),
}));

it("keeps the captured text in the capture window when the main window could not save it", async () => {
  vi.spyOn(backend(), "bindCaptureGraph").mockImplementation(() => new Promise(() => {}));
  vi.spyOn(backend(), "getCaptureEnterFiles").mockResolvedValue(false);
  vi.spyOn(backend(), "captureTarget").mockResolvedValue("main");
  vi.spyOn(backend(), "graphBindingGeneration").mockReturnValue(17);
  const root = document.createElement("div");
  root.id = "capture-root";
  document.body.append(root);
  await import("./capture");
  await vi.waitFor(() => expect(h.captureApi).not.toBeNull());
  h.captureApi!.submit();
  await vi.waitFor(() => expect(h.emitted.some((e) => e.event === "quick-capture")).toBe(true));
  const request = h.emitted.find((e) => e.event === "quick-capture")!.payload as { id: string; text: string };
  expect(request.text).toBe("- captured thought");
  await vi.waitFor(() => expect(h.listeners.has("quick-capture-ack")).toBe(true));
  h.listeners.get("quick-capture-ack")!({ payload: { id: request.id, ok: false } });
  await new Promise((resolve) => setTimeout(resolve, 20));
  // The scratch editor was neither cleared nor emptied, so the text is still there to resubmit.
  expect(h.setRaw).toEqual([]);
  expect(h.deleted).toEqual([]);
  expect(document.body.textContent ?? "").toContain("text kept");
});

it.each(["text", "title", "IME"])("acknowledges only the submitted snapshot, retaining later %s edits", async (change) => {
  h.setRaw.length = 0;
  h.deleted.length = 0;
  h.hide.mockClear();
  h.markdown = "- capture A";
  const title = document.querySelector<HTMLInputElement>(".capture-title")!;
  title.value = "Destination";
  title.dispatchEvent(new Event("input", { bubbles: true }));
  const emittedBefore = h.emitted.length;
  h.captureApi!.submit();
  await vi.waitFor(() => expect(h.emitted.length).toBeGreaterThan(emittedBefore));
  await vi.waitFor(() => expect(h.listeners.has("quick-capture-ack")).toBe(true));
  const request = h.emitted.filter((e) => e.event === "quick-capture").at(-1)!.payload as { id: string; text: string; bindingGeneration: number };
  expect(request.text).toBe("- capture A");
  expect(request.bindingGeneration).toBe(17);
  if (change === "text") h.markdown = "- capture AB\n- new child";
  if (change === "title") { title.value = "Later title"; title.dispatchEvent(new Event("input", { bubbles: true })); }
  if (change === "IME") document.querySelector(".capture-shell")!.dispatchEvent(new CompositionEvent("compositionstart", { bubbles: true }));
  h.listeners.get("quick-capture-ack")!({ payload: { id: request.id, ok: true } });
  await Promise.resolve();
  expect(h.setRaw).toEqual([]);
  expect(h.deleted).toEqual([]);
  expect(title.value).toBe(change === "title" ? "Later title" : "Destination");
  expect(h.hide).not.toHaveBeenCalled();
});

it("clears and hides after a successful acknowledgement when the snapshot is unchanged", async () => {
  h.setRaw.length = 0; h.hide.mockClear(); h.markdown = "- final capture";
  const emittedBefore = h.emitted.length;
  h.captureApi!.submit();
  await vi.waitFor(() => expect(h.emitted.length).toBeGreaterThan(emittedBefore));
  const request = h.emitted.filter((e) => e.event === "quick-capture").at(-1)!.payload as { id: string };
  h.listeners.get("quick-capture-ack")!({ payload: { id: request.id, ok: true } });
  await vi.waitFor(() => expect(h.hide).toHaveBeenCalledOnce());
  expect(h.setRaw).toEqual(["scratch-root"]);
  expect(document.querySelector<HTMLInputElement>(".capture-title")!.value).toBe("");
});
