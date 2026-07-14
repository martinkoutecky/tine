import { describe, expect, it } from "vitest";
import { createCaptureScratchPage } from "./captureSeed";

describe("Quick Capture scratch seed", () => {
  it("gives the first block a real id so automatic editor activation can target it", () => {
    const first = createCaptureScratchPage();
    const second = createCaptureScratchPage();

    expect(first.blocks[0].id).not.toBe("");
    expect(second.blocks[0].id).not.toBe(first.blocks[0].id);
  });

  it("rejects the empty sentinel that made the first visible bullet non-editable", () => {
    expect(() => createCaptureScratchPage("")).toThrow(/must not be empty/);
  });
});
