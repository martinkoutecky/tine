import { afterEach, expect, it, vi } from "vitest";
import { render } from "solid-js/web";
import { backend } from "../backend";
import { closeSettings, openSettings } from "../ui";
import { bumpGraphEpoch } from "../graphSession";
import { setToasts, toasts } from "../toasts";

const flush = vi.hoisted(() => vi.fn());
vi.mock("../document", async (importOriginal) => ({
  ...await importOriginal<typeof import("../document")>(), flushAll: flush,
}));

import { Settings } from "./Settings";

afterEach(() => { closeSettings(); vi.restoreAllMocks(); document.body.innerHTML = ""; });

it("does not scan orphan assets when pending edits fail to flush", async () => {
  flush.mockResolvedValue(false);
  const scan = vi.spyOn(backend(), "listOrphanAssets").mockResolvedValue([]);
  const root = document.createElement("div");
  document.body.append(root);
  const dispose = render(() => <Settings />, root);
  try {
    openSettings("files");
    const button = [...root.querySelectorAll("button")].find((node) => node.textContent?.includes("Scan for orphans"));
    expect(button).toBeDefined();
    button!.click();
    await vi.waitFor(() => expect(flush).toHaveBeenCalled());
    expect(scan).not.toHaveBeenCalled();
  } finally { dispose(); }
});

async function scanOne(root: HTMLElement) {
  flush.mockResolvedValue(true);
  vi.spyOn(backend(), "listOrphanAssets").mockResolvedValue([{ name: "shared.png", size: 12, modified: null }]);
  vi.spyOn(backend(), "assetTrashStats").mockResolvedValue({ count: 0, bytes: 0, pages: 0, journals: 0, conflicts: 0, other: 0 });
  openSettings("files");
  const scan = () => [...root.querySelectorAll("button")].find((node) => node.textContent?.includes("Scan for orphans"));
  await vi.waitFor(() => expect(scan()).toBeDefined());
  scan()!.click();
  await vi.waitFor(() => expect(root.textContent).toContain("shared.png"));
  return () => [...root.querySelectorAll("button")].find((node) => node.textContent === "Trash");
}

it("trashes a scanned orphan in the graph it was scanned in", async () => {
  const trash = vi.spyOn(backend(), "trashAsset").mockResolvedValue("trashed");
  const root = document.createElement("div");
  document.body.append(root);
  const dispose = render(() => <Settings />, root);
  try {
    const trashButton = await scanOne(root);
    trashButton()!.click();
    await vi.waitFor(() => expect(trash).toHaveBeenCalledWith("shared.png", expect.anything()));
  } finally { dispose(); }
});

it("keeps a scanned orphan that a page started using, and says so instead of reporting a failure (GH #623)", async () => {
  setToasts([]);
  const trash = vi.spyOn(backend(), "trashAsset").mockResolvedValue("referenced");
  const root = document.createElement("div");
  document.body.append(root);
  const dispose = render(() => <Settings />, root);
  try {
    const trashButton = await scanOne(root);
    trashButton()!.click();
    await vi.waitFor(() => expect(trash).toHaveBeenCalled());
    await vi.waitFor(() => expect(toasts().some((toast) => toast.kind === "info" && toast.message.includes("was kept"))).toBe(true));
    expect(toasts().some((toast) => toast.kind === "error")).toBe(false);
  } finally { dispose(); }
});

it("hides a scan once its graph is gone, and refuses a Trash click that outlived the binding (I-20)", async () => {
  setToasts([]);
  let generation = 1;
  const api = backend() as unknown as { graphBindingGeneration?: () => number };
  const previous = api.graphBindingGeneration;
  api.graphBindingGeneration = () => generation;
  const trash = vi.spyOn(backend(), "trashAsset").mockResolvedValue("trashed");
  const root = document.createElement("div");
  document.body.append(root);
  const dispose = render(() => <Settings />, root);
  try {
    const trashButton = await scanOne(root);
    // A native binding change that has not yet re-rendered the list: the click is refused.
    generation = 2;
    trashButton()!.click();
    await vi.waitFor(() => expect(toasts().some((toast) => toast.kind === "error" && toast.message.includes("no longer open"))).toBe(true));
    expect(trash).not.toHaveBeenCalled();
    // A graph switch retires the list from the screen.
    bumpGraphEpoch();
    await vi.waitFor(() => expect(root.textContent).not.toContain("shared.png"));
  } finally { dispose(); api.graphBindingGeneration = previous; }
});
