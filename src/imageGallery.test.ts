import { describe, expect, it } from "vitest";
import { orderGallery, stepIndex } from "./imageGallery";

const it_ = (src: string, x: number, y: number) => ({ src, x, y, key: src });

describe("orderGallery (OG open-lightbox: reading order, rotated to the clicked image)", () => {
  const items = [it_("c", 0, 300), it_("a", 0, 0), it_("b2", 200, 100), it_("b1", 0, 100)];
  it("sorts by y then x and starts at the clicked image, wrapping the earlier ones to the end", () => {
    expect(orderGallery(items, "b1").map((i) => i.src)).toEqual(["b1", "b2", "c", "a"]);
    expect(orderGallery(items, "a").map((i) => i.src)).toEqual(["a", "b1", "b2", "c"]);
  });
  it("an unknown clicked image keeps plain reading order; one image is itself", () => {
    expect(orderGallery(items, "zzz").map((i) => i.src)).toEqual(["a", "b1", "b2", "c"]);
    expect(orderGallery([it_("only", 1, 1)], "only").map((i) => i.src)).toEqual(["only"]);
  });
});

describe("stepIndex", () => {
  it("one image: nowhere to go", () => { expect(stepIndex(0, 1, 1)).toBeNull(); expect(stepIndex(0, 1, -1)).toBeNull(); });
  it("two images: stops at the ends", () => {
    expect(stepIndex(0, 2, 1)).toBe(1);
    expect(stepIndex(1, 2, 1)).toBeNull();
    expect(stepIndex(0, 2, -1)).toBeNull();
  });
  it("three or more: wraps (PhotoSwipe loop)", () => {
    expect(stepIndex(2, 3, 1)).toBe(0);
    expect(stepIndex(0, 3, -1)).toBe(2);
    expect(stepIndex(1, 5, 1)).toBe(2);
  });
});
