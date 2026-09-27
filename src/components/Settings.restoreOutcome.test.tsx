import { afterEach, expect, it, vi } from "vitest";
import { render } from "solid-js/web";
import { backend } from "../backend";
import { closeSettings, openSettings } from "../ui";
import { setToasts, toasts } from "../toasts";

const controls = vi.hoisted(() => ({ flush: vi.fn(), load: vi.fn() }));
vi.mock("../document", async (importOriginal) => ({
  ...await importOriginal<typeof import("../document")>(), flushAll: controls.flush,
}));
vi.mock("../graph", async (importOriginal) => ({
  ...await importOriginal<typeof import("../graph")>(), loadGraphPath: controls.load,
}));
import { Settings } from "./Settings";

afterEach(() => { closeSettings(); setToasts([]); vi.restoreAllMocks(); document.body.innerHTML = ""; });

it("reports an aborted graph reload after restore without claiming success", async () => {
  vi.spyOn(backend(), "getBackupKeep").mockResolvedValue(12);
  vi.spyOn(backend(), "listBackups").mockResolvedValue([{ stamp: "2026-07-22_12-00-00", files: 1 }]);
  vi.spyOn(backend(), "confirm").mockResolvedValue(true);
  vi.spyOn(backend(), "restoreBackup").mockResolvedValue();
  controls.flush.mockResolvedValue(true);
  controls.load.mockResolvedValue({ kind: "aborted" });
  const root = document.createElement("div");
  document.body.append(root);
  const dispose = render(() => <Settings />, root);
  try {
    openSettings("backups");
    const restore = () => [...root.querySelectorAll<HTMLButtonElement>("button")]
      .find((button) => button.textContent?.trim() === "Restore");
    await vi.waitFor(() => expect(restore()?.disabled).toBe(false));
    restore()!.click();
    await vi.waitFor(() => expect(controls.load).toHaveBeenCalled());
    expect(toasts().some((toast) => toast.kind === "success" && toast.message.includes("Restored snapshot"))).toBe(false);
    expect(toasts().some((toast) => toast.kind === "error" && toast.message.includes("couldn't be reloaded"))).toBe(true);
  } finally { dispose(); }
});
