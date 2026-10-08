import { expect, it } from "vitest";
import { resetStore } from "./workingSet";
import { isConflicted, markConflict } from "./save/engine";

it("clears a page conflict when the document store resets", () => {
  markConflict("Old page");
  resetStore();
  expect(isConflicted("Old page")).toBe(false);
});
