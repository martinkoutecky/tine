import { afterEach, describe, expect, it, vi } from "vitest";
import { pdfProbe } from "./pdfProbe";

describe("pdfProbe", () => {
  afterEach(() => { delete (globalThis as { __tinePdfProbe?: unknown }).__tinePdfProbe; });

  it("does nothing unless a perf harness installs the hook", () => {
    expect(() => pdfProbe({ type: "render-start", page: 1 })).not.toThrow();
  });

  it("hands installed hooks the event with a timestamp", () => {
    const hook = vi.fn();
    (globalThis as { __tinePdfProbe?: unknown }).__tinePdfProbe = hook;
    pdfProbe({ type: "render-end", page: 3, scale: 2 });
    expect(hook).toHaveBeenCalledWith(expect.objectContaining({ type: "render-end", page: 3, scale: 2, t: expect.any(Number) }));
  });
});
