import { describe, expect, it, vi } from "vitest";
import { createPdfPinch } from "./pdfPinch";

const points = (distance: number, x = 150, y = 100) => [
  { identifier: 1, clientX: x - distance / 2, clientY: y },
  { identifier: 2, clientX: x + distance / 2, clientY: y },
];
function setup(scale = 1) {
  const start = vi.fn(), preview = vi.fn(), finish = vi.fn();
  return { start, preview, finish, pinch: createPdfPinch({ scale: () => scale, start, preview, finish }) };
}
describe("PDF pinch state", () => {
  it("tracks finger identities, live scale and a moving midpoint, committing once after either finger lifts", () => {
    const { pinch, preview, finish } = setup(1.5);
    expect(pinch.update(points(100).slice(0, 1), "start")).toBe(false);
    expect(pinch.update(points(100), "start")).toBe(true);
    pinch.update(points(150, 170, 120).reverse(), "move");
    expect(preview).toHaveBeenLastCalledWith({ scale: 2.25, factor: 1.5,
      origin: { x: 150, y: 100 }, midpoint: { x: 170, y: 120 } });
    pinch.update(points(150).slice(1), "end");
    pinch.update(points(100), "start"); // a replacement finger cannot restart
    pinch.update([], "end");
    expect(finish).toHaveBeenCalledTimes(1);
    expect(finish.mock.calls[0][0].scale).toBe(2.25);
  });
  it.each([[1000, 4], [1, 0.4]])("clamps a %spx separation to %s", (distance, scale) => {
    const { pinch, preview } = setup();
    pinch.update(points(100), "start");
    pinch.update(points(distance), "move");
    expect(preview.mock.calls[0][0].scale).toBe(scale);
    expect(preview.mock.calls[0][0].factor).toBe(scale);
  });
  it.each(["cancel", "third finger", "teardown"])("restores preview without committing on %s", (reason) => {
    const { pinch, finish } = setup();
    pinch.update(points(100), "start");
    pinch.update(points(150), "move");
    if (reason === "cancel") pinch.update([], "cancel");
    else if (reason === "third finger") pinch.update([...points(150), { identifier: 3, clientX: 1, clientY: 1 }], "start");
    else pinch.cancel();
    pinch.update([], "end");
    expect(finish).toHaveBeenCalledExactlyOnceWith(null);
  });
  it("ignores a zero-distance start and leaves one-finger moves native", () => {
    const { pinch, start } = setup();
    expect(pinch.update(points(0), "start")).toBe(false);
    expect(pinch.update(points(100).slice(0, 1), "move")).toBe(false);
    expect(start).not.toHaveBeenCalled();
  });
});
