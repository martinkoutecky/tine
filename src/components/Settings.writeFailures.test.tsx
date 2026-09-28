// @vitest-environment jsdom
import { afterEach, expect, it, vi } from "vitest";
import { render } from "solid-js/web";
import { backend } from "../backend";
import { toasts, setToasts } from "../toasts";
import { closeSettings, openSettings } from "../ui";
import { Settings } from "./Settings";

afterEach(() => { closeSettings(); document.body.innerHTML = ""; setToasts([]); vi.restoreAllMocks(); });

it("reverts the Files watch mode when saving it fails", async () => {
  vi.spyOn(backend(), "getWatchMode").mockResolvedValue("inotify");
  vi.spyOn(backend(), "setWatchMode").mockRejectedValue(new Error("disk full"));
  const root = document.createElement("div");
  document.body.append(root);
  const dispose = render(() => <Settings />, root);
  openSettings("files");
  await vi.waitFor(() => expect(root.textContent).toContain("Watch for external edits"));
  const poll = [...root.querySelectorAll<HTMLButtonElement>("button")].find((button) => button.textContent?.includes("Poll (3s)"));
  expect(poll).toBeDefined();
  poll!.click();
  await vi.waitFor(() => expect(toasts().some((toast) => toast.kind === "error")).toBe(true));
  expect(poll!.classList.contains("active")).toBe(false);
  dispose();
});

it("reports a failed Advanced disclosure preference write", async () => {
  const root = document.createElement("div");
  document.body.append(root);
  const dispose = render(() => <Settings />, root);
  openSettings("editor");
  await vi.waitFor(() => expect(root.querySelector(".settings-advanced-toggle")).not.toBeNull());
  vi.spyOn(Storage.prototype, "setItem").mockImplementation(() => { throw new Error("quota"); });
  root.querySelector<HTMLButtonElement>(".settings-advanced-toggle")!.click();
  expect(toasts().some((toast) => toast.kind === "error")).toBe(true);
  dispose();
});
