// Previous / Next change in the seen notice (ADR 0079, D-20261029-SS): the marked
// items in page order, and which one a step lands on.
import { expect, it } from "vitest";
import { emptyMarks, SEEN_PAGE_ROOT, type SeenMarks } from "./marks";
import { orderedChangeItems, stepTarget, type SeenItem } from "./steps";

// a (changed)
//   a1 (changed, paired with the marker before it)
//   a2
// b
// c (changed)
const children: Record<string, string[]> = { a: ["a1", "a2"], a1: [], a2: [], b: [], c: [] };
const marks = (over: Partial<SeenMarks>): SeenMarks => ({ ...emptyMarks(), ...over });
const raw = (id: string) => `raw-${id}`;

it("lists changed blocks and removed markers in the order the page draws them", () => {
  const m = marks({
    blocks: { a: "raw-a", a1: "raw-a1", c: "raw-c" },
    paired: { a1: true },
    first: { [SEEN_PAGE_ROOT]: [{ text: "gone at top" }], a: [{ text: "first in a" }] },
    after: { a1: [{ text: "after a1" }], a: [{ text: "after a" }, { text: "after a again" }] },
    removedCount: 5,
  });
  const keys = orderedChangeItems(m, ["a", "b", "c"], (id) => children[id] ?? [], raw).map((item) => item.key);
  expect(keys).toEqual(["f::0", "b:a", "f:a:0", "a:a1:0", "a:a:0", "a:a:1", "b:c"]);
});

it("a block edited locally since the snapshot is no longer an item", () => {
  const m = marks({ blocks: { a: "old", c: "raw-c" } });
  expect(orderedChangeItems(m, ["a", "b", "c"], (id) => children[id] ?? [], raw).map((item) => item.key)).toEqual(["b:c"]);
});

const item = (key: string): SeenItem => ({ key, kind: "block", reveal: key });
const items = ["x", "y", "z"].map(item);

it("steps from nowhere to the first (next) or last (previous) item", () => {
  expect(stepTarget(items, null, 1)).toEqual({ item: items[0], index: 0, wrapped: false });
  expect(stepTarget(items, null, -1)).toEqual({ item: items[2], index: 2, wrapped: false });
});

it("steps from the current item and wraps at both ends, saying so", () => {
  expect(stepTarget(items, { key: "x", index: 0 }, 1)).toEqual({ item: items[1], index: 1, wrapped: false });
  expect(stepTarget(items, { key: "z", index: 2 }, 1)).toEqual({ item: items[0], index: 0, wrapped: true });
  expect(stepTarget(items, { key: "x", index: 0 }, -1)).toEqual({ item: items[2], index: 2, wrapped: true });
  expect(stepTarget(items, { key: "y", index: 1 }, -1)).toEqual({ item: items[0], index: 0, wrapped: false });
});

it("a current item that vanished steps from the place it held", () => {
  const rest = [item("x"), item("z")]; // y was edited away; z now sits where y was
  expect(stepTarget(rest, { key: "y", index: 1 }, 1)).toEqual({ item: rest[1], index: 1, wrapped: false });
  expect(stepTarget(rest, { key: "y", index: 1 }, -1)).toEqual({ item: rest[0], index: 0, wrapped: false });
  expect(stepTarget(rest, { key: "z-gone", index: 5 }, 1)).toEqual({ item: rest[0], index: 0, wrapped: true });
});

it("nothing to step to when no item is marked", () => {
  expect(stepTarget([], null, 1)).toBeNull();
});
