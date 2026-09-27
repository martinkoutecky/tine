import { expect, it } from "vitest";
import { isBlockMoving, withBlockMoving } from "./edits/moves";

it("clears the move flag after a rejected move", async () => {
  await expect(withBlockMoving("A", async () => { throw new Error("failed"); })).rejects.toThrow("failed");
  expect(isBlockMoving()).toBe(false);
});
