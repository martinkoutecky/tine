import { expect, it, vi } from "vitest";
import { graphRowMenuActions } from "./graphRowMenu";

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
  expect(failure.action?.label).toBe("Retry");
  failure.action!.run();
  await vi.waitFor(() => expect(openKnown).toHaveBeenCalledTimes(2));
  expect(openKnown).toHaveBeenLastCalledWith("/graphs/notes", false);
  setToasts([]);
});
