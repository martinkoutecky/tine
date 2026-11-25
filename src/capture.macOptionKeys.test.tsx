import { expect, it, vi } from "vitest";

// The keymap reads the platform once, when it loads: be a Mac from the start.
vi.hoisted(() => Object.defineProperty(navigator, "platform", { configurable: true, value: "MacIntel" }));

import { CAPTURE_SCRATCH_NAME } from "./captureSeed";
import { pageByName } from "./document";
import { editorIn } from "./tests/editorDriver";

vi.mock("@tauri-apps/api/event", () => ({ listen: async () => () => {}, emitTo: async () => {} }));
vi.mock("@tauri-apps/api/window", () => ({
  getCurrentWindow: () => ({ label: "capture", isVisible: async () => false, onFocusChanged: async () => () => {} }),
}));

// GH #1040: Quick Capture installs the main window's key handler, so on macOS
// Option+Left/Right in its editor and title field are word motion too (the
// window has no history to go Back in).
it("macOS Quick Capture: Option+Left/Right reach the editor and the title field unprevented", async () => {
  const host = document.createElement("div");
  host.id = "capture-root";
  document.body.append(host);
  await import("./capture");
  await vi.waitFor(() => expect(pageByName(CAPTURE_SCRATCH_NAME)?.roots[0]).toBeDefined());
  const editor = await vi.waitFor(() => editorIn(host));
  editor.focus();
  const arrow = (key: "ArrowLeft" | "ArrowRight") => ({ key, code: key, altKey: true });
  // CodeMirror consumes the arrow for its own word motion; the textarea leaves it to the system.
  if (editor.kind === "textarea") {
    expect(editor.keydown(arrow("ArrowLeft")).defaultPrevented).toBe(false);
    expect(editor.keydown(arrow("ArrowRight")).defaultPrevented).toBe(false);
  }
  const title = host.querySelector<HTMLInputElement>("input.capture-title")!;
  title.focus();
  for (const key of ["ArrowLeft", "ArrowRight"] as const) {
    const event = new KeyboardEvent("keydown", { bubbles: true, cancelable: true, ...arrow(key) });
    title.dispatchEvent(event);
    expect(event.defaultPrevented).toBe(false);
  }
});
