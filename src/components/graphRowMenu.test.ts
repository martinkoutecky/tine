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
    "Show in folder", "Copy path", "Remove from this list",
  ]);
  expect(actions.slice(0, 2).every((action) => action.disabled)).toBe(true);
  const mobile = graphRowMenuActions(graph, {
    openKnown: vi.fn(), reveal: vi.fn(), copyPath: vi.fn(), forget: vi.fn(),
    desktop: false, isCurrent: false,
  });
  expect(mobile.map((action) => action.label)).toEqual(["Open here", "Copy path", "Remove from this list"]);
});
