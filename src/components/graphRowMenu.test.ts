import { expect, it, vi } from "vitest";
import { graphRowMenuActions } from "./graphRowMenu";
import { renderEnglish } from "../i18n";
import { toUiText } from "../toasts";

const graph = { name: "Notes", path: "/graphs/notes" };

it("keeps current graph actions visible but disabled and omits desktop actions on mobile", () => {
  const actions = graphRowMenuActions(graph, {
    openKnown: vi.fn(), reveal: vi.fn(), copyPath: vi.fn(), forget: vi.fn(),
    desktop: true, isCurrent: true,
  });
  expect(actions.map((action) => action.label)).toEqual([
    "Open in a new window (already open here)", "Open here (current graph)",
    "Show in folder", "Copy link", "Copy path", "Remove from this list",
  ]);
  expect(actions.slice(0, 2).every((action) => action.disabled)).toBe(true);
  const mobile = graphRowMenuActions(graph, {
    openKnown: vi.fn(), reveal: vi.fn(), copyPath: vi.fn(), forget: vi.fn(),
    desktop: false, isCurrent: false,
  });
  expect(mobile.map((action) => action.label)).toEqual(["Open here", "Copy link", "Copy path", "Remove from this list"]);
});

it("keeps a context-menu open failure sticky and retries the same graph (master 9a9122b1544d)", async () => {
  const { setToasts, toasts } = await import("../toasts");
  setToasts([]);
  const openKnown = vi.fn(async () => { throw new Error("graph root is not readable"); });
  const actions = graphRowMenuActions(graph, {
    openKnown, reveal: vi.fn(), copyPath: vi.fn(), forget: vi.fn(),
    desktop: true, isCurrent: false,
  });
  const openHere = actions.find((action) => action.label === "Open here")!;
  expect(openHere.run).toBeDefined();
  openHere.run!();
  await vi.waitFor(() => expect(toasts()).toHaveLength(1));
  const failure = toasts()[0]!;
  expect(failure.kind).toBe("error");
  expect(failure.message).toContain("graph root is not readable");
  expect(failure.sticky).toBe(true);
  expect(renderEnglish(toUiText(failure.action!.label))).toBe("Retry");
  failure.action!.run();
  await vi.waitFor(() => expect(openKnown).toHaveBeenCalledTimes(2));
  expect(openKnown).toHaveBeenLastCalledWith("/graphs/notes", false);
  setToasts([]);
});

// GH #926: the row shows the presentable path (display_path); Copy path copies
// the canonical one, which is what pasting into a file manager or a link needs.
it("copies the full canonical path even when the row shows a shorter spelling", () => {
  const full = "/private/var/mobile/Containers/Data/Application/0F8B2C1E-7A3D-4C55-9B21-1D2E3F4A5B6C/Documents/Notes";
  const copyPath = vi.fn(async () => {});
  const actions = graphRowMenuActions({ name: "Notes", path: full, display_path: "Documents/Notes" }, {
    openKnown: vi.fn(), reveal: vi.fn(), copyPath, forget: vi.fn(), desktop: false, isCurrent: false,
  });
  actions.find((action) => action.label === "Copy path")!.run!();
  expect(copyPath).toHaveBeenCalledExactlyOnceWith(full);
});
