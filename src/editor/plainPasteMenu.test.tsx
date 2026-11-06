import { afterEach, expect, it, vi } from "vitest";
import { installPlainPasteMenu } from "./plainPasteMenu";
import { registerFocusedEditorCommandBridge } from "../editorCommandBridge";

const native = vi.hoisted(() => ({ listen: vi.fn() }));
vi.mock("@tauri-apps/api/webviewWindow", () => ({
  getCurrentWebviewWindow: () => ({ label: "this-graph", listen: native.listen }),
}));
afterEach(() => { Reflect.deleteProperty(window, "__TAURI_INTERNALS__"); vi.clearAllMocks(); });

it("delivers a window-scoped native menu action to the focused editor and removes the listener", async () => {
  Object.defineProperty(window, "__TAURI_INTERNALS__", { configurable: true, value: {} });
  const stop = vi.fn(), dispatch = vi.fn(() => true);
  native.listen.mockResolvedValue(stop);
  const unregister = registerFocusedEditorCommandBridge({ blockId: "focused", dispatch, blur() {} });
  const uninstall = installPlainPasteMenu();
  try {
    await vi.waitFor(() => expect(native.listen).toHaveBeenCalledWith("editor-paste-plain", expect.any(Function)));
    native.listen.mock.calls[0][1]();
    expect(dispatch).toHaveBeenCalledExactlyOnceWith("editor/paste-plain");
    uninstall();
    expect(stop).toHaveBeenCalledOnce();
  } finally { uninstall(); unregister(); }
});

it("removes a late listener when its owner has already closed", async () => {
  Object.defineProperty(window, "__TAURI_INTERNALS__", { configurable: true, value: {} });
  const stop = vi.fn();
  let finish!: (unlisten: () => void) => void;
  native.listen.mockImplementation(() => new Promise(resolve => { finish = resolve; }));
  const uninstall = installPlainPasteMenu();
  await vi.waitFor(() => expect(native.listen).toHaveBeenCalled());
  uninstall();
  finish(stop);
  await vi.waitFor(() => expect(stop).toHaveBeenCalledOnce());
});
