import { beforeAll, expect, it } from "vitest";
import { initParser } from "../render/parse";
import { swipeRefNames } from "./blockSwipeWiring";

beforeAll(() => initParser());

// L3-11: OG's `:block/refs` for this opt-out holds headline tags, references, the marker and the priority.
it("counts an Org headline tag", () => {
  expect(swipeRefNames("Task :private:", "org")).toContain("private");
  expect(swipeRefNames("Task [[x]] :private:", "org")).toEqual(expect.arrayContaining(["x", "private"]));
});
it("counts the marker and the priority", () => {
  const refs = swipeRefNames("TODO [#A] buy [[milk]] #shop", "md");
  expect(refs).toEqual(expect.arrayContaining(["todo", "a", "milk", "shop"]));
});
it("has nothing to say about plain text", () => {
  expect(swipeRefNames("plain", "md")).toEqual([]);
});
