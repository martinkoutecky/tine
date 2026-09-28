import { afterEach, expect, it, vi } from "vitest";
import type { JSX } from "solid-js";
import { render } from "solid-js/web";
import { backend } from "../backend";
import { invalidateBinding } from "../binding";
import { setToasts, toasts } from "../toasts";
import { JournalFilenamePanel } from "./JournalFilenamePanel";

function mount(node: () => JSX.Element): { root: HTMLDivElement; dispose: () => void } {
  const root = document.createElement("div");
  document.body.appendChild(root);
  const dispose = render(node, root);
  return { root, dispose: () => { dispose(); root.remove(); } };
}

afterEach(() => {
  vi.restoreAllMocks();
  setToasts([]);
});

// Master e6f9b6e1ceae: opening a graph proposes journal renames; only this
// button performs them.
it("lists proposed journal renames and applies them only after confirmation", async () => {
  const list = vi.spyOn(backend(), "listJournalFilenameMigrations")
    .mockResolvedValueOnce([{ from: "Thursday, 25-06-2026.org", to: "2026_06_25.org" }])
    .mockResolvedValue([]);
  const confirm = vi.spyOn(backend(), "confirm").mockResolvedValueOnce(false).mockResolvedValueOnce(true);
  const apply = vi.spyOn(backend(), "applyJournalFilenameMigrations").mockResolvedValue({ migrated: 1, skipped: [] });
  vi.spyOn(backend(), "listJournalConflicts").mockResolvedValue([]);
  const { root, dispose } = mount(() => <JournalFilenamePanel />);
  try {
    await vi.waitFor(() => expect(root.textContent).toContain("Thursday, 25-06-2026.org"));
    expect(root.textContent).toContain("→ 2026_06_25.org");
    const button = () => root.querySelector<HTMLButtonElement>("button")!;
    button().click();
    await vi.waitFor(() => expect(confirm).toHaveBeenCalledTimes(1));
    expect(apply).not.toHaveBeenCalled();
    button().click();
    await vi.waitFor(() => expect(apply).toHaveBeenCalledOnce());
    // G3 finding 3: the backend gets exactly the list the confirmation named.
    expect(apply).toHaveBeenCalledWith([{ from: "Thursday, 25-06-2026.org", to: "2026_06_25.org" }]);
    await vi.waitFor(() => expect(toasts().at(-1)?.message).toBe("Renamed 1 journal file"));
    await vi.waitFor(() => expect(root.textContent).not.toContain("Thursday"));
    expect(list).toHaveBeenCalledTimes(2);
  } finally {
    dispose();
  }
});

// G3 finding 4: a durable failure from a graph the user already left must not
// toast into the graph they are now on.
it("does not toast a stale apply failure into the next graph", async () => {
  vi.spyOn(backend(), "listJournalFilenameMigrations")
    .mockResolvedValue([{ from: "Thursday, 25-06-2026.org", to: "2026_06_25.org" }]);
  vi.spyOn(backend(), "confirm").mockResolvedValue(true);
  let reject!: (error: unknown) => void;
  const apply = vi.spyOn(backend(), "applyJournalFilenameMigrations")
    .mockReturnValue(new Promise((_, no) => { reject = no; }));
  const { root, dispose } = mount(() => <JournalFilenamePanel />);
  try {
    await vi.waitFor(() => expect(root.textContent).toContain("Thursday"));
    root.querySelector<HTMLButtonElement>("button")!.click();
    await vi.waitFor(() => expect(apply).toHaveBeenCalledOnce());
    invalidateBinding();
    reject(new Error("could not take a snapshot first"));
    await vi.waitFor(() => expect(root.querySelector("button")?.textContent).toBe("Rename to date names"));
    expect(toasts().map((toast) => toast.message)).toEqual([]);
  } finally {
    dispose();
  }
});
