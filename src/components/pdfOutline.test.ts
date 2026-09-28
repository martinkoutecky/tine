import { describe, expect, it } from "vitest";
import { PDF_OUTLINE_MAX_NODES, PDF_OUTLINE_MAX_SCANNED_SLOTS, sanitizeOutlineItems } from "./pdfOutline";

/** A proxy over `items` that counts element reads, so the test measures the
 *  work done rather than wall-clock time. */
function counted(items: unknown[]) {
  const reads = { count: 0 };
  const proxy = new Proxy(items, {
    get(target, key, receiver) {
      if (typeof key === "string" && /^\d+$/.test(key)) reads.count++;
      return Reflect.get(target, key, receiver);
    },
  });
  return { proxy, reads };
}

describe("sanitizeOutlineItems (og 15b, I-22)", () => {
  it("stops scanning a huge outline of invalid entries at the slot cap", () => {
    const { proxy, reads } = counted(new Array(1_000_000).fill(null));
    expect(sanitizeOutlineItems(proxy)).toMatchObject({ items: [], truncated: true });
    expect(reads.count).toBeLessThanOrEqual(PDF_OUTLINE_MAX_SCANNED_SLOTS);
  });

  it("bounds scanning across many nested arrays of invalid entries", () => {
    const children = Array.from({ length: 1000 }, () => counted(new Array(1000).fill(0)));
    const outline = children.map(({ proxy }) => ({ title: "x", items: proxy }));
    sanitizeOutlineItems(outline);
    const total = children.reduce((sum, { reads }) => sum + reads.count, 0);
    expect(total).toBeLessThanOrEqual(PDF_OUTLINE_MAX_SCANNED_SLOTS);
  });

  it("still emits a benign outline of the maximum node count", () => {
    const outline = Array.from({ length: PDF_OUTLINE_MAX_NODES }, (_, i) => ({ title: `Chapter ${i}`, dest: `d${i}` }));
    const result = sanitizeOutlineItems(outline);
    expect(result.items).toHaveLength(PDF_OUTLINE_MAX_NODES);
    expect(result.truncated).toBe(false);
    expect(result.items[PDF_OUTLINE_MAX_NODES - 1]).toMatchObject({ label: `Chapter ${PDF_OUTLINE_MAX_NODES - 1}`, destination: `d${PDF_OUTLINE_MAX_NODES - 1}` });
  });

  it("reports truncation when a valid bookmark follows the slot boundary", () => {
    const outline = new Array(PDF_OUTLINE_MAX_SCANNED_SLOTS).fill(null);
    outline.push({ title: "Late bookmark", dest: "late" });
    const result = sanitizeOutlineItems(outline);
    expect(result.items).toHaveLength(0);
    expect(result.truncated).toBe(true);
  });
});
