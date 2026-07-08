import { describe, expect, it } from "vitest";
import { decodeNavIntent } from "./navProtocol";

function ev(init: Partial<KeyboardEvent>): KeyboardEvent {
  return {
    key: "",
    code: "",
    shiftKey: false,
    ctrlKey: false,
    metaKey: false,
    altKey: false,
    isComposing: false,
    ...init,
  } as KeyboardEvent;
}

describe("decodeNavIntent — the one nav key table (ADR 0034)", () => {
  it("decodes plain arrows as steps and shifted arrows as extends", () => {
    expect(decodeNavIntent(ev({ key: "ArrowUp" }))).toEqual({ kind: "step", dir: "up" });
    expect(decodeNavIntent(ev({ key: "ArrowRight" }))).toEqual({ kind: "step", dir: "right" });
    expect(decodeNavIntent(ev({ key: "ArrowDown", shiftKey: true }))).toEqual({ kind: "extend", dir: "down" });
    expect(decodeNavIntent(ev({ key: "ArrowLeft", shiftKey: true }))).toEqual({ kind: "extend", dir: "left" });
  });

  it("declines every mod-chord — those are surface commands, not nav", () => {
    expect(decodeNavIntent(ev({ key: "ArrowRight", ctrlKey: true }))).toBeNull();
    expect(decodeNavIntent(ev({ key: "Enter", metaKey: true }))).toBeNull();
    expect(decodeNavIntent(ev({ key: "k", ctrlKey: true }))).toBeNull();
    expect(decodeNavIntent(ev({ key: "Escape", altKey: true }))).toBeNull();
  });

  it("decodes Escape as dismiss (shift tolerated), Enter as activate, F2 only on opt-in", () => {
    expect(decodeNavIntent(ev({ key: "Escape" }))).toEqual({ kind: "dismiss" });
    expect(decodeNavIntent(ev({ key: "Escape", shiftKey: true }))).toEqual({ kind: "dismiss" });
    expect(decodeNavIntent(ev({ key: "Enter" }))).toEqual({ kind: "activate" });
    expect(decodeNavIntent(ev({ key: "F2" }))).toBeNull();
    expect(decodeNavIntent(ev({ key: "F2" }), { acceptF2: true })).toEqual({ kind: "activate" });
  });

  it("shifted non-character keys are not nav; shifted printables are", () => {
    expect(decodeNavIntent(ev({ key: "Enter", shiftKey: true }))).toBeNull();
    expect(decodeNavIntent(ev({ key: "Delete", shiftKey: true }))).toBeNull();
    expect(decodeNavIntent(ev({ key: "Z", shiftKey: true }))).toEqual({ kind: "overtype", char: "Z" });
  });

  it("decodes Backspace/Delete as sided removes and printables as overtype", () => {
    expect(decodeNavIntent(ev({ key: "Backspace" }))).toEqual({ kind: "remove", side: "before" });
    expect(decodeNavIntent(ev({ key: "Delete" }))).toEqual({ kind: "remove", side: "after" });
    expect(decodeNavIntent(ev({ key: "z" }))).toEqual({ kind: "overtype", char: "z" });
    expect(decodeNavIntent(ev({ key: " " }))).toEqual({ kind: "overtype", char: " " });
  });

  it("ignores non-nav keys and IME composition", () => {
    expect(decodeNavIntent(ev({ key: "Tab" }))).toBeNull();
    expect(decodeNavIntent(ev({ key: "Home" }))).toBeNull();
    expect(decodeNavIntent(ev({ key: "z", isComposing: true }))).toBeNull();
  });
});
