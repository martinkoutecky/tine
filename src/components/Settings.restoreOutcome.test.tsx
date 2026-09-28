import { afterEach, expect, it, vi } from "vitest";
import { render } from "solid-js/web";
import { backend } from "../backend";
import { closeSettings, graphTransitioning, openSettings, setGraphTransitioning } from "../ui";
import { setToasts, toasts } from "../toasts";
import { resetStore } from "../document";
import { setJournalConflicts, setSyncConflicts } from "../ui";

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

it("does not restore a backup after its confirmation outlives the graph", async () => {
  vi.spyOn(backend(), "getBackupKeep").mockResolvedValue(12);
  vi.spyOn(backend(), "listBackups").mockResolvedValue([{ stamp: "2026-07-22_12-00-00", files: 1 }]);
  let finish!: (confirmed: boolean) => void;
  vi.spyOn(backend(), "confirm").mockImplementationOnce(() => new Promise((resolve) => { finish = resolve; }));
  const restoreBackup = vi.spyOn(backend(), "restoreBackup").mockResolvedValue();
  controls.flush.mockResolvedValue(true);
  const root = document.createElement("div");
  document.body.append(root);
  const dispose = render(() => <Settings />, root);
  try {
    openSettings("backups");
    const restore = () => [...root.querySelectorAll<HTMLButtonElement>("button")]
      .find((button) => button.textContent?.trim() === "Restore");
    await vi.waitFor(() => expect(restore()?.disabled).toBe(false));
    restore()!.click();
    resetStore();
    finish(true);
    await new Promise((resolve) => setTimeout(resolve, 0));
    expect(restoreBackup).not.toHaveBeenCalled();
  } finally { dispose(); }
});

it("does not clear a newer graph transition when an old restore finishes", async () => {
  vi.spyOn(backend(), "getBackupKeep").mockResolvedValue(12);
  vi.spyOn(backend(), "listBackups").mockResolvedValue([{ stamp: "2026-07-22_12-00-00", files: 1 }]);
  vi.spyOn(backend(), "confirm").mockResolvedValue(true);
  let finish!: () => void;
  vi.spyOn(backend(), "restoreBackup").mockImplementationOnce(() => new Promise((resolve) => { finish = resolve; }));
  controls.flush.mockResolvedValue(true);
  const root = document.createElement("div");
  document.body.append(root);
  const dispose = render(() => <Settings />, root);
  try {
    openSettings("backups");
    const restore = () => [...root.querySelectorAll<HTMLButtonElement>("button")]
      .find((button) => button.textContent?.trim() === "Restore");
    await vi.waitFor(() => expect(restore()?.disabled).toBe(false));
    restore()!.click();
    await vi.waitFor(() => expect(backend().restoreBackup).toHaveBeenCalled());
    resetStore();
    setGraphTransitioning(true);
    finish();
    await new Promise((resolve) => setTimeout(resolve, 0));
    expect(graphTransitioning()).toBe(true);
  } finally { dispose(); setGraphTransitioning(false); }
});

it("does not trash a duplicate journal after its confirmation outlives the graph", async () => {
  vi.spyOn(backend(), "getBackupKeep").mockResolvedValue(12);
  vi.spyOn(backend(), "listBackups").mockResolvedValue([]);
  const conflicts = [{ title: "Day", files: [
    { name: "2026_07_22.md", path: "journals/2026_07_22.md", preview: "main", canonical: true },
    { name: "Day.md", path: "journals/Day.md", preview: "stray", canonical: false },
  ] }];
  vi.spyOn(backend(), "listJournalConflicts").mockResolvedValue(conflicts);
  setJournalConflicts(conflicts);
  let finish!: (confirmed: boolean) => void;
  vi.spyOn(backend(), "confirm").mockImplementationOnce(() => new Promise((resolve) => { finish = resolve; }));
  const trash = vi.spyOn(backend(), "trashJournalFile").mockResolvedValue();
  const root = document.createElement("div");
  document.body.append(root);
  const dispose = render(() => <Settings />, root);
  try {
    openSettings("backups");
    const button = () => root.querySelector<HTMLButtonElement>('[data-journal-conflict="journals/Day.md"] .settings-btn-danger');
    await vi.waitFor(() => expect(button()).not.toBeNull());
    button()!.click();
    resetStore();
    finish(true);
    await new Promise((resolve) => setTimeout(resolve, 0));
    expect(trash).not.toHaveBeenCalled();
  } finally { dispose(); setJournalConflicts([]); }
});

it("does not discard a sync conflict after its confirmation outlives the graph", async () => {
  vi.spyOn(backend(), "getBackupKeep").mockResolvedValue(12);
  vi.spyOn(backend(), "listBackups").mockResolvedValue([]);
  const conflicts = [{ path: "pages/old.sync-conflict.md", base_name: "Page", base_path: "pages/Page.md",
    kind: "page" as const, tag: "old", preview: "copy" }];
  vi.spyOn(backend(), "listSyncConflicts").mockResolvedValue(conflicts);
  setSyncConflicts(conflicts);
  let finish!: (confirmed: boolean) => void;
  vi.spyOn(backend(), "confirm").mockImplementationOnce(() => new Promise((resolve) => { finish = resolve; }));
  const trash = vi.spyOn(backend(), "trashSyncConflict").mockResolvedValue();
  const root = document.createElement("div");
  document.body.append(root);
  const dispose = render(() => <Settings />, root);
  try {
    openSettings("backups");
    const button = () => root.querySelector<HTMLButtonElement>(".sync-conflict-row .settings-btn-danger");
    await vi.waitFor(() => expect(button()).not.toBeNull());
    button()!.click();
    resetStore();
    finish(true);
    await new Promise((resolve) => setTimeout(resolve, 0));
    expect(trash).not.toHaveBeenCalled();
  } finally { dispose(); setSyncConflicts([]); }
});
