import { describe, expect, it } from "vitest";
import { largestFirst, parseFrameExtents, parseXwininfo } from "../scripts/lib/e2e-x11.mjs";

const XWININFO = `
xwininfo: Window id: 0x400003 "Tine"

  Absolute upper-left X:  12
  Absolute upper-left Y:  -4
  Relative upper-left X:  0
  Width: 1280
  Height: 800
`;

describe("shared X11 window helper for the native Linux journeys", () => {
  it("reads the client origin and size from xwininfo, naming the window", () => {
    expect(parseXwininfo(XWININFO, "4194307")).toEqual({
      WINDOW: 4194307, X: 12, Y: -4, WIDTH: 1280, HEIGHT: 800,
    });
    expect(() => parseXwininfo("Width: 3", "1")).toThrow(/omitted Absolute upper-left X/);
  });

  it("reads frame extents, and treats a missing property as zero only when the journey asks", () => {
    expect(parseFrameExtents("_NET_FRAME_EXTENTS(CARDINAL) = 1, 2, 28, 3")).toEqual({
      left: 1, right: 2, top: 28, bottom: 3,
    });
    const absent = "_NET_FRAME_EXTENTS:  not found.\n_GTK_FRAME_EXTENTS:  not found.";
    expect(parseFrameExtents(absent, { missingAsZero: true })).toEqual({ left: 0, right: 0, top: 0, bottom: 0 });
    expect(() => parseFrameExtents(absent)).toThrow(/malformed frame extents/);
    expect(() => parseFrameExtents("garbage", { missingAsZero: true })).toThrow(/malformed frame extents/);
  });

  it("orders windows largest first so the graph window beats a tiny helper surface", () => {
    const sizes: Record<string, { WIDTH: number; HEIGHT: number }> = {
      small: { WIDTH: 10, HEIGHT: 10 }, big: { WIDTH: 800, HEIGHT: 600 }, mid: { WIDTH: 100, HEIGHT: 100 },
    };
    expect(largestFirst(["small", "big", "mid"], (id) => sizes[id])).toEqual(["big", "mid", "small"]);
    expect(largestFirst(["a", "b"], () => { throw new Error("gone"); })).toEqual(["a", "b"]);
  });
});
