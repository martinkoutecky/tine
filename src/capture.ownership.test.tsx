import { describe, expect, it, vi } from "vitest";
import { backend } from "./backend";

const h = vi.hoisted(() => ({
  listeners: new Map<string, (event: { payload?: unknown }) => void>(),
  captureApi: null as null | { enterFiles: () => boolean },
}));

vi.mock("./render/parse", () => ({ initParser: async () => {} }));
vi.mock("./captureSeed", () => ({ CAPTURE_SCRATCH_NAME: "___Capture Scratch___", createCaptureScratchPage: () => ({}) }));
vi.mock("./components/Block", () => ({
  Block: () => null,
  CaptureCtx: { Provider: (props: { value: { enterFiles: () => boolean }; children: unknown }) => {
    h.captureApi = props.value;
    return props.children;
  } },
}));
vi.mock("./components/DatePicker", () => ({ DatePicker: () => null }));
vi.mock("./document", () => ({
  ensurePageLoaded: () => {}, pageByName: () => ({ roots: [] }), blockSubtreeMarkdown: () => "",
  deleteBlock: () => {}, setRaw: () => {}, node: () => null,
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
  emitTo: async () => {},
}));
vi.mock("@tauri-apps/api/window", () => ({
  getCurrentWindow: () => ({ label: "capture", isVisible: async () => false,
    onFocusChanged: async () => () => {}, hide: async () => {} }),
}));

describe("capture preference request ownership", () => {
  it("keeps the newer enter-files setting when an older read finishes last", async () => {
    vi.spyOn(backend(), "bindCaptureGraph").mockImplementation(() => new Promise(() => {}));
    let first!: (value: boolean) => void;
    let second!: (value: boolean) => void;
    const read = vi.spyOn(backend(), "getCaptureEnterFiles")
      .mockImplementationOnce(() => new Promise((resolve) => { first = resolve; }))
      .mockImplementationOnce(() => new Promise((resolve) => { second = resolve; }));
    const root = document.createElement("div");
    root.id = "capture-root";
    document.body.append(root);
    await import("./capture");
    await vi.waitFor(() => expect(read).toHaveBeenCalledTimes(1));
    await vi.waitFor(() => expect(h.listeners.has("capture-shown")).toBe(true));
    h.listeners.get("capture-shown")!({});
    await vi.waitFor(() => expect(read).toHaveBeenCalledTimes(2));
    second(true);
    await vi.waitFor(() => expect(h.captureApi?.enterFiles()).toBe(true));
    first(false);
    await Promise.resolve();
    await Promise.resolve();
    expect(h.captureApi?.enterFiles()).toBe(true);
    read.mockRestore();
  });
});
