import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { render } from "solid-js/web";
import type { Backend } from "../backend";
import { closeSettings, openSettings } from "../ui";
import { resetStore } from "../document";
import { setToasts, toasts } from "../toasts";

const backupBackend = vi.hoisted(() => ({
  getBackupKeep: vi.fn(),
  listBackups: vi.fn(),
  setBackupKeep: vi.fn(),
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

beforeEach(() => {
  backupBackend.getBackupKeep.mockReset();
  backupBackend.listBackups.mockReset();
  backupBackend.setBackupKeep.mockReset();
});

afterEach(() => {
  closeSettings();
  document.body.innerHTML = "";
  localStorage.clear();
  vi.clearAllMocks();
});

describe("Backups settings", () => {
  it("reports a retention write failure after the graph changes", async () => {
    backupBackend.getBackupKeep.mockResolvedValue(12);
    backupBackend.listBackups.mockResolvedValue([]);
    let rejectWrite!: (error: Error) => void;
    backupBackend.setBackupKeep.mockImplementation(() => new Promise((_, reject) => { rejectWrite = reject; }));
    setToasts([]);
    const root = document.createElement("div");
    document.body.append(root);
    const dispose = render(() => <Settings />, root);
    openSettings("backups");
    await tick();
    await tick();
    const keep = root.querySelector('input[type="number"]') as HTMLInputElement;
    keep.value = "7";
    keep.dispatchEvent(new Event("change", { bubbles: true }));
    expect(backupBackend.setBackupKeep).toHaveBeenCalledWith(7);
    resetStore();
    rejectWrite(new Error("retention disk failed"));
    await tick();
    expect(toasts().some((toast) => toast.message.includes("retention disk failed"))).toBe(true);
    dispose();
  });
  it("shows loading and error states, then enables backup controls after data loads", async () => {
    let rejectList!: (error: Error) => void;
    backupBackend.getBackupKeep.mockResolvedValue(12);
    backupBackend.listBackups
      .mockImplementationOnce(() => new Promise((_, reject) => { rejectList = reject; }))
      .mockResolvedValueOnce([{ id: "zip:2026-07-22_12-00-00", stamp: "2026-07-22_12-00-00", files: 1, status: "ok" }]);

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
  it("labels damaged and unsupported backups and offers no restore for them", async () => {
    backupBackend.getBackupKeep.mockResolvedValue(12);
    backupBackend.listBackups.mockResolvedValue([
      { id: "zip:2026-07-22_12-00-02", stamp: "2026-07-22_12-00-02", files: 3, status: "unsupported" },
      { id: "zip:2026-07-22_12-00-01", stamp: "2026-07-22_12-00-01", files: 2, status: "damaged" },
      { id: "dir:2026-07-22_12-00-00", stamp: "2026-07-22_12-00-00", files: 1, status: "ok" },
    ]);
    const root = document.createElement("div");
    document.body.append(root);
    const dispose = render(() => <Settings />, root);
    openSettings("backups");
    await tick();
    await tick();

    const rows = [...root.querySelectorAll(".settings-backup-row")];
    expect(rows.map((row) => row.querySelector(".settings-backup-files")?.textContent?.trim())).toEqual([
      "3 files · Made by a newer Tine",
      "2 files · Damaged",
      "1 files",
    ]);
    expect(rows.map((row) => (row.querySelector("button") as HTMLButtonElement).disabled)).toEqual([true, true, false]);
    dispose();
  });
});
