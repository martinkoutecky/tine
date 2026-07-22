import { afterEach, describe, expect, it, vi } from "vitest";
import { render } from "solid-js/web";
import type { Backend } from "../backend";
import { closeSettings, openSettings } from "../ui";

const backupBackend = vi.hoisted(() => ({
  getBackupKeep: vi.fn(),
  listBackups: vi.fn(),
}));

vi.mock("../backend", async (importOriginal) => {
  const actual = await importOriginal<typeof import("../backend")>();
  return {
    ...actual,
    backend: () => backupBackend as unknown as Backend,
    isTauri: () => false,
  };
});

import { Settings } from "./Settings";

const tick = () => new Promise((resolve) => setTimeout(resolve, 0));

afterEach(() => {
  closeSettings();
  document.body.innerHTML = "";
  localStorage.clear();
  vi.clearAllMocks();
});

describe("Backups settings", () => {
  it("shows loading and error states, then enables backup controls after data loads", async () => {
    let rejectList!: (error: Error) => void;
    backupBackend.getBackupKeep.mockResolvedValue(12);
    backupBackend.listBackups
      .mockImplementationOnce(() => new Promise((_, reject) => { rejectList = reject; }))
      .mockResolvedValueOnce([{ stamp: "2026-07-22_12-00-00", files: 1 }]);

    const root = document.createElement("div");
    document.body.append(root);
    const dispose = render(() => <Settings />, root);
    openSettings("backups");
    await tick();

    const keep = root.querySelector('input[type="number"]') as HTMLInputElement;
    const refresh = [...root.querySelectorAll("button")].find((button) => button.textContent?.trim() === "Refresh") as HTMLButtonElement;
    expect(root.querySelector('[role="status"]')?.textContent).toContain("Loading snapshot settings");
    expect(keep.disabled).toBe(true);
    expect(refresh.disabled).toBe(true);

    rejectList(new Error("offline"));
    await tick();
    await tick();

    expect(root.querySelector('[role="alert"]')?.textContent).toContain("Couldn't load backup settings: Error: offline");
    expect(keep.disabled).toBe(true);
    expect(refresh.disabled).toBe(false);

    const retry = [...root.querySelectorAll("button")].find((button) => button.textContent?.trim() === "Retry") as HTMLButtonElement;
    retry.click();
    await tick();
    await tick();

    expect(root.querySelector('[role="status"]')).toBeNull();
    expect(root.querySelector('[role="alert"]')).toBeNull();
    expect(root.textContent).toContain("1 files");
    expect(keep.disabled).toBe(false);
    const restore = [...root.querySelectorAll("button")].find((button) => button.textContent?.trim() === "Restore") as HTMLButtonElement;
    expect(restore.disabled).toBe(false);
    dispose();
  });
});
