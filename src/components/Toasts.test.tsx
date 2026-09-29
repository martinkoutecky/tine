// OG-TOAST T2 (Martin, 2026-09-29): a red toast means something actually went
// wrong and is worth reporting. It stays until dismissed, offers Copy of the
// full message, and is recorded for later recovery. Warn/info/success keep
// their 3.2 s auto-dismiss.
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { render } from "solid-js/web";
import { backend } from "../backend";
import { initDebug, resetDebugForTests } from "../debug";
import { pushToast, setToasts, toasts } from "../toasts";
import { Toasts } from "./Toasts";

let dispose: () => void = () => {};
beforeEach(() => {
  setToasts([]);
  const host = document.createElement("div");
  document.body.appendChild(host);
  dispose = render(() => <Toasts />, host);
});
afterEach(() => {
  dispose();
  vi.useRealTimers();
  vi.restoreAllMocks();
  document.body.innerHTML = "";
  setToasts([]);
});

describe("error toasts", () => {
  it("an error toast does not auto-dismiss and its Copy button copies the full message", async () => {
    vi.useFakeTimers();
    const message = "Tine couldn't finish checking for external changes. (Error: disk said no)";
    pushToast(message, "error");
    pushToast("Saved", "info");
    pushToast("Heads up", "warn");
    vi.advanceTimersByTime(60_000);
    expect(toasts().map((t) => t.kind)).toEqual(["error"]);
    const node = document.querySelector(".toast.toast-error")!;
    expect(node.querySelector(".toast-msg")!.textContent).toBe(message);
    // A click on the body of an error toast does not dismiss it either.
    (node as HTMLElement).click();
    expect(toasts()).toHaveLength(1);
    vi.useRealTimers();
    const write = vi.spyOn(backend(), "writeText").mockResolvedValue();
    const copy = [...node.querySelectorAll("button")].find((b) => b.textContent?.trim() === "Copy")!;
    expect(copy).toBeTruthy();
    copy.click();
    await vi.waitFor(() => expect(write).toHaveBeenCalledWith(message));
    await vi.waitFor(() => expect(toasts().some((t) => t.message === "Error message copied")).toBe(true));
    expect(toasts().some((t) => t.kind === "error" && t.message === message)).toBe(true);
    (node.querySelector(".toast-close") as HTMLElement).click();
    expect(toasts().some((t) => t.kind === "error")).toBe(false);
  });

  it("only error toasts offer Copy", () => {
    pushToast("Heads up", "warn", { sticky: true });
    expect(document.querySelector(".toast-copy")).toBeNull();
  });

  it("every error toast is recorded: its occurrence in the recorder, its text in the debug log", async () => {
    resetDebugForTests();
    const api = backend();
    const recorded = vi.spyOn(api, "diagnosticFrontendEvent").mockResolvedValue();
    const logged = vi.spyOn(api, "debugLog").mockResolvedValue();
    vi.spyOn(api, "debugInfo").mockResolvedValue({ enabled: true, path: "/tmp/tine-debug.log", recorderActive: true, previousExitUnclean: false } as never);
    await initDebug();
    pushToast("Could not save session: disk full", "error");
    pushToast("Saved", "info");
    await vi.waitFor(() => expect(recorded).toHaveBeenCalledWith("error_toast", undefined));
    expect(recorded.mock.calls.filter(([kind]) => kind === "error_toast")).toHaveLength(1);
    expect(logged).toHaveBeenCalledWith("error toast: Could not save session: disk full");
  });
});
