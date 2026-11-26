import { expect, it, vi } from "vitest";
import { disposeCapture, mountCapture } from "./captureApp";
import { pushToast, toasts } from "../toasts";
import { msg } from "../i18n";

vi.mock("@tauri-apps/api/event", () => ({ listen: async () => () => {}, emitTo: async () => {} }));
vi.mock("@tauri-apps/api/window", () => ({
  getCurrentWindow: () => ({ label: "capture", isVisible: async () => false, onFocusChanged: async () => () => {} }),
}));

it("a toast pushed after a Capture test file ends does not render into the document", async () => {
  const root = await mountCapture();
  await vi.waitFor(() => expect(root.querySelector(".toast-stack")).not.toBeNull());
  pushToast(msg("toast-copied"), "success");
  expect(root.querySelectorAll(".toast")).toHaveLength(1);
  disposeCapture();
  // What late work does once the file is over: the store changes, the DOM does not.
  pushToast(msg("toast-copy-failed"), "warn");
  expect(toasts()).toHaveLength(2);
  expect(document.querySelector(".toast")).toBeNull();
});
