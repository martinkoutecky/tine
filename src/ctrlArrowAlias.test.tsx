import { describe, expect, it } from "vitest";
import { ctrlArrowAlias } from "./ctrlArrowAlias";

const ev = (init: KeyboardEventInit) => new KeyboardEvent("keydown", { cancelable: true, ...init });

describe("ctrlArrowAlias (GH #986)", () => {
  it("reads Ctrl+P / Ctrl+N as ArrowUp / ArrowDown and acts on the real event", () => {
    const up = ev({ key: "p", code: "KeyP", ctrlKey: true });
    const alias = ctrlArrowAlias(up);
    expect([alias.key, alias.code, alias.ctrlKey, alias.shiftKey]).toEqual(["ArrowUp", "ArrowUp", false, false]);
    alias.preventDefault();
    expect(up.defaultPrevented).toBe(true);
    expect(ctrlArrowAlias(ev({ key: "n", ctrlKey: true })).key).toBe("ArrowDown");
  });
  it("reads a letter typed on a non-Latin layout by its physical key", () => {
    expect(ctrlArrowAlias(ev({ key: "з", code: "KeyP", ctrlKey: true })).key).toBe("ArrowUp");
  });
  it("leaves every other chord alone", () => {
    for (const init of [{ key: "p" }, { key: "p", ctrlKey: true, shiftKey: true }, { key: "p", ctrlKey: true, altKey: true },
      { key: "p", metaKey: true }, { key: "p", ctrlKey: true, metaKey: true }, { key: "k", ctrlKey: true }, { key: "ArrowUp" }]) {
      const e = ev(init);
      expect(ctrlArrowAlias(e)).toBe(e);
    }
  });
});
