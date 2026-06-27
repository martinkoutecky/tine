import { describe, it, expect } from "vitest";
import { emojiSplit, twemojiName } from "./emoji";

describe("emojiSplit", () => {
  it("plain text → single text part", () => {
    expect(emojiSplit("hello world")).toEqual([{ t: "text", v: "hello world" }]);
  });

  it("splits emoji out of surrounding text", () => {
    expect(emojiSplit("a 🏁 b")).toEqual([
      { t: "text", v: "a " },
      { t: "emoji", v: "🏁" },
      { t: "text", v: " b" },
    ]);
  });

  it("a lone emoji", () => {
    expect(emojiSplit("🏎️")).toEqual([{ t: "emoji", v: "🏎️" }]);
  });

  it("empty / no-emoji strings pass through untouched", () => {
    expect(emojiSplit("")).toEqual([{ t: "text", v: "" }]);
    expect(emojiSplit("café — n-fold IP")).toEqual([{ t: "text", v: "café — n-fold IP" }]);
  });
});

describe("twemojiName (Twemoji filename rule)", () => {
  it("single-codepoint emoji", () => {
    expect(twemojiName("🏁")).toBe("1f3c1");
    expect(twemojiName("📅")).toBe("1f4c5");
  });

  it("strips U+FE0F for a non-ZWJ presentation sequence", () => {
    expect(twemojiName("🏎️")).toBe("1f3ce"); // U+1F3CE U+FE0F → fe0f dropped
  });

  it("keeps U+FE0F and joins with '-' inside a ZWJ sequence", () => {
    // 👨‍👩‍👧 family — ZWJ sequence keeps every codepoint.
    expect(twemojiName("👨‍👩‍👧")).toBe("1f468-200d-1f469-200d-1f467");
  });
});
