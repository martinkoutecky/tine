import { describe, it, expect } from "vitest";
import { cycleMarker, nextMarker, leadingMarker, setMarker } from "./marker";

describe("nextMarker", () => {
  it("todo workflow cycle", () => {
    expect(nextMarker(null, "todo")).toBe("TODO");
    expect(nextMarker("TODO", "todo")).toBe("DOING");
    expect(nextMarker("DOING", "todo")).toBe("DONE");
    expect(nextMarker("DONE", "todo")).toBe(null);
  });
  it("now workflow cycle", () => {
    expect(nextMarker(null, "now")).toBe("LATER");
    expect(nextMarker("LATER", "now")).toBe("NOW");
    expect(nextMarker("NOW", "now")).toBe("DONE");
    expect(nextMarker("DONE", "now")).toBe(null);
  });
});

describe("cycleMarker", () => {
  it("adds a marker to a plain block (caret shifts by prefix length)", () => {
    expect(cycleMarker("buy milk", "todo")).toEqual({ raw: "TODO buy milk", delta: 5 });
  });
  it("advances an existing marker", () => {
    expect(cycleMarker("TODO buy milk", "todo")).toEqual({ raw: "DOING buy milk", delta: 1 });
  });
  it("removes the marker after DONE", () => {
    expect(cycleMarker("DONE buy milk", "todo")).toEqual({ raw: "buy milk", delta: -5 });
  });
  it("preserves continuation lines and props", () => {
    const r = cycleMarker("buy milk\nid:: abc", "todo");
    expect(r.raw).toBe("TODO buy milk\nid:: abc");
  });
  it("detects leading marker", () => {
    expect(leadingMarker("DOING x")).toBe("DOING");
    expect(leadingMarker("plain")).toBe(null);
  });
});

describe("setMarker", () => {
  it("replaces an existing marker while preserving priority, body and properties", () => {
    expect(setMarker("TODO [#A] Ship it\nowner:: Martin", "DONE"))
      .toBe("DONE [#A] Ship it\nowner:: Martin");
  });

  it("adds any supported marker to an unmarked block", () => {
    for (const marker of ["TODO", "DOING", "LATER", "NOW", "DONE", "WAITING", "WAIT", "IN-PROGRESS", "CANCELED"]) {
      expect(setMarker("Ship it", marker)).toBe(`${marker} Ship it`);
    }
  });
});
