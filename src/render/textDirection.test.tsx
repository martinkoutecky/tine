import { beforeAll, describe, expect, it } from "vitest";
import { initParser } from "./parse";
import { arrowLeave, blockDirAttr, blockTextDirection, firstStrongDirection, isRtlContext } from "./textDirection";
import { beyondFinalGlyph } from "./spans";

beforeAll(() => initParser());

describe("what the reader sees decides a block's direction (GH #823)", () => {
  const rtl = "یادداشت"; // an invented Persian word
  it.each([
    ["plain Persian", rtl, "rtl"],
    ["Hebrew", "הערה", "rtl"],
    ["Arabic", "ملاحظة", "rtl"],
    ["a task whose source starts with Latin", `TODO ${rtl}`, "rtl"],
    ["a task with a priority", `TODO [#A] ${rtl}`, "rtl"],
    ["a heading", `## ${rtl}`, "rtl"],
    ["a leading page reference in a Persian sentence", `[[Tine]] ${rtl}`, "rtl"],
    ["a leading Latin tag in a Persian sentence", `#inbox ${rtl}`, "rtl"],
    ["a leading property", `key:: value\n${rtl}`, "rtl"],
    ["leading inline code", `\`code\` ${rtl}`, "rtl"],
    ["bold", `**${rtl}**`, "rtl"],
    ["only a reference", `[[${rtl}]]`, "rtl"],
    ["digits before the text", `42 ${rtl}`, "rtl"],
    ["an English sentence with a Persian reference", `See [[${rtl}]] here`, "ltr"],
    ["a sentence starting in English", `Tine ${rtl}`, "ltr"],
    ["a mark overruling the guess", `‏Tine ${rtl}`, "rtl"],
    ["English", "plain words", "ltr"],
    ["only digits", "12345", null],
    ["Persian digits", "۱۲۳", null],
    ["empty", "", null],
    ["only code", `\`\`\`js\n// ${rtl}\n\`\`\``, null],
  ])("%s", (_name, raw, want) => {
    expect(blockTextDirection(raw, "md")).toBe(want);
  });

  it("reads Org syntax with the Org parser", () => {
    expect(blockTextDirection(`* TODO ${rtl}`.slice(2), "org")).toBe("rtl");
  });

  it("gives a left-to-right block no attribute at all, under the default layout", () => {
    expect(blockDirAttr("TODO plain words #tag", "md")).toBeUndefined();
    expect(blockDirAttr("12345", "md")).toBeUndefined();
    expect(blockDirAttr(`TODO ${rtl}`, "md")).toBe("rtl");
    expect(blockDirAttr("plain words", "md", true)).toBe("ltr");
  });

  it("first strong character skips digits, punctuation and emoji", () => {
    expect(firstStrongDirection("1. \u{1F600} (ا")).toBe("rtl");
    expect(firstStrongDirection("1. \u{1F600} (a")).toBe("ltr");
    expect(firstStrongDirection("1. \u{1F600}")).toBeNull();
  });
});

describe("arrow keys keep their on-screen meaning at a block's edges (GH #823)", () => {
  const inside = (dir?: string) => {
    const host = document.createElement("div");
    if (dir) host.setAttribute("dir", dir);
    return host.appendChild(document.createElement("textarea"));
  };
  it("left to right: unchanged", () => {
    const el = inside();
    expect(isRtlContext(el)).toBe(false);
    expect(arrowLeave("ArrowLeft", el, 0, 0, 5)).toBe("backward");
    expect(arrowLeave("ArrowRight", el, 5, 5, 5)).toBe("forward");
    expect(arrowLeave("ArrowRight", el, 0, 0, 5)).toBeNull();
    expect(arrowLeave("ArrowLeft", el, 5, 5, 5)).toBeNull();
  });
  it("right to left: offset 0 is the right edge", () => {
    const el = inside("rtl");
    expect(arrowLeave("ArrowRight", el, 0, 0, 5)).toBe("backward");
    expect(arrowLeave("ArrowLeft", el, 5, 5, 5)).toBe("forward");
    expect(arrowLeave("ArrowLeft", el, 0, 0, 5)).toBeNull();
    expect(arrowLeave("ArrowRight", el, 5, 5, 5)).toBeNull();
  });
  it("an empty block leaves both ways, a range or an interior caret neither", () => {
    const el = inside("rtl");
    expect(arrowLeave("ArrowRight", el, 0, 0, 0)).toBe("backward");
    expect(arrowLeave("ArrowLeft", el, 0, 0, 0)).toBe("forward");
    expect(arrowLeave("ArrowLeft", el, 0, 5, 5)).toBeNull();
    expect(arrowLeave("ArrowLeft", el, 2, 2, 5)).toBeNull();
  });
});

describe("past the end of a block (GH #823)", () => {
  const boxes = [{ top: 0, bottom: 20, left: 100, right: 300 }];
  it("is on the left of a right-to-left line", () => {
    expect(beyondFinalGlyph(boxes, 50, 10, true)).toBe(true);
    expect(beyondFinalGlyph(boxes, 350, 10, true)).toBe(false);
  });
  it("stays on the right of a left-to-right line", () => {
    expect(beyondFinalGlyph(boxes, 350, 10)).toBe(true);
    expect(beyondFinalGlyph(boxes, 50, 10)).toBe(false);
  });
});
