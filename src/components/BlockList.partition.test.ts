import { createSignal } from "solid-js";
import { expect, it } from "vitest";
import { stablePartition } from "./BlockList";

const handle = (ids: string[]) => { const [get, set] = createSignal<readonly string[]>(ids); return { ids: get, set }; };
const range = (from: number, to: number) => Array.from({ length: to - from }, (_, i) => `b${from + i}`);
const ones = (n: number) => Array.from({ length: n }, () => 1);

it("GH #745: with no prior windows it packs greedily into 24-weight groups", () => {
  const out = stablePartition(range(0, 100), ones(100), []);
  expect(out.map(g => g.ids.length)).toEqual([24, 24, 24, 24, 4]);
  expect(out.every(g => g.handle === null)).toBe(true);
});

it("GH #745: a row inserted into a full window keeps every window boundary", () => {
  const prior = [handle(range(0, 24)), handle(range(24, 48)), handle(range(48, 72))];
  const ids = [...range(0, 10), "new", ...range(10, 72)];
  const out = stablePartition(ids, ones(ids.length), prior);
  expect(out.map(g => g.handle)).toEqual(prior);
  expect(out.map(g => g.ids.length)).toEqual([25, 24, 24]);
});

it("GH #745: a window that more than doubles splits and keeps its handle on the first piece", () => {
  const prior = [handle(range(0, 24)), handle(range(24, 48))];
  const extra = Array.from({ length: 30 }, (_, i) => `n${i}`);
  const ids = [...range(0, 12), ...extra, ...range(12, 48)];
  const out = stablePartition(ids, ones(ids.length), prior);
  expect(out[0].handle).toBe(prior[0]);
  expect(out.at(-1)!.handle).toBe(prior[1]);
  expect(out.every(g => g.ids.reduce(sum => sum + 1, 0) <= 24)).toBe(true);
  expect(out.flatMap(g => g.ids)).toEqual(ids);
});

it("GH #745: a deleted window disappears and a shrunken one merges into its neighbour", () => {
  const prior = [handle(range(0, 24)), handle(range(24, 48)), handle(range(48, 72))];
  const gone = stablePartition([...range(0, 24), ...range(48, 72)], ones(48), prior);
  expect(gone.map(g => g.handle)).toEqual([prior[0], prior[2]]);
  const shrunk = stablePartition([...range(0, 24), ...range(24, 30), ...range(48, 60)], ones(42), prior);
  expect(shrunk.map(g => g.handle)).toEqual([prior[0], prior[1]]);
  expect(shrunk.map(g => g.ids.length)).toEqual([24, 18]);
});

it("GH #745: one block moved across a window boundary does not split its destination", () => {
  const prior = [handle(range(0, 24)), handle(range(24, 48))];
  const ids = [...range(0, 10), "b30", ...range(10, 24), ...range(24, 30), ...range(31, 48)];
  const out = stablePartition(ids, ones(ids.length), prior);
  expect(out.map(g => g.handle)).toEqual(prior);
  expect(out.flatMap(g => g.ids)).toEqual(ids);
});

it("heavy siblings (one expanded subtree each) stay one window each", () => {
  const ids = range(0, 10);
  const out = stablePartition(ids, ids.map(() => 81), []);
  expect(out.map(g => g.ids.length)).toEqual(Array(10).fill(1));
});
