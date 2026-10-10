// Q-P2b-3 (GH #538, #590): the host's save-failure notice carries the failed
// platform step and OS error, and the window keeps showing them in the sticky,
// copyable error toast (Martin 9/29). A notice without them names no step.

import { afterEach, expect, it, vi } from "vitest";
import { backend } from "../../backend";
import { startEditing } from "../../editorController";
import { setToasts, toasts } from "../../toasts";
import { resetStore, setRaw } from "../index";
import { loadRoutedPage } from "../workingSet";
import { bindTestHost } from "./wiring.test.support";

const PATH = "pages/Failing.md";

afterEach(() => { resetStore(); setToasts([]); vi.restoreAllMocks(); });

/** A page the host holds for this window (edited, sent, still being edited). */
async function heldPage() {
  resetStore();
  loadRoutedPage({ name: "Failing", kind: "page", title: "Failing", id: PATH, rev: "r1", pre_block: null,
    blocks: [{ id: "failing-block", raw: "before", collapsed: false, children: [] }] });
  const host = await bindTestHost();
  const submit = vi.spyOn(backend(), "pageSubmit");
  setRaw("failing-block", "after", { timetracking: false });
  startEditing("failing-block");
  await vi.waitFor(() => expect(submit).toHaveBeenCalledOnce(), { timeout: 1000 });
  return host;
}

function saveToasts() {
  return toasts().filter((toast) => toast.message.includes("Couldn't save “Failing”"));
}

it("shows the failed save's platform step and OS error in a sticky error toast", async () => {
  const host = await heldPage();
  expect(saveToasts()).toEqual([]);
  host.notice(PATH, { failures: 3, saveError: true, operation: "renameat2(RENAME_NOREPLACE)", osError: 22 });
  await vi.waitFor(() => expect(saveToasts()).toHaveLength(1));
  const [toast] = saveToasts();
  expect(toast.kind).toBe("error");
  expect(toast.sticky).toBe(true);
  expect(toast.message).toContain("renameat2(RENAME_NOREPLACE), os error 22");
});

it("names no platform step when the notice carries none", async () => {
  const host = await heldPage();
  host.notice(PATH, { failures: 3, saveError: true, operation: null, osError: null });
  await vi.waitFor(() => expect(saveToasts()).toHaveLength(1));
  const [toast] = saveToasts();
  expect(toast.sticky).toBe(true);
  expect(toast.message).not.toContain("(");
  expect(toast.message).not.toContain("os error");
});
