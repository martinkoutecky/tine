import { describe, expect, it } from "vitest";
import { orderedLane } from "./orderedWrites";

const tick = () => new Promise((resolve) => setTimeout(resolve, 0));

describe("ordered lane", () => {
  // PERF-FIXES D3: back-to-back opens share one call; a call between them
  // closes the batch, so no open moves past another ordered request.
  it("joins back-to-back batch items and never reorders them past another call", async () => {
    const lane = orderedLane();
    const issued: string[] = [];
    const batch = (items: number[]) => {
      issued.push(`open ${items.join(",")}`);
      return Promise.resolve(items.map((id) => `opened ${id}`));
    };
    const answers = [
      lane.joined("page_open", 1, batch),
      lane.joined("page_open", 2, batch),
      lane("page_submit", () => { issued.push("submit 3"); return Promise.resolve("submitted 3"); }),
      lane.joined("page_open", 4, batch),
    ];
    expect(await Promise.all(answers)).toEqual(["opened 1", "opened 2", "submitted 3", "opened 4"]);
    expect(issued).toEqual(["open 1,2", "submit 3", "open 4"]);
  });

  it("starts a new batch once the previous one has started", async () => {
    const lane = orderedLane();
    const issued: number[][] = [];
    let release: () => void = () => {};
    const first = lane.joined("page_open", 1, (items: number[]) => {
      issued.push([...items]);
      return new Promise<number[]>((resolve) => { release = () => resolve(items); });
    });
    await tick();
    const second = lane.joined("page_open", 2, (items: number[]) => { issued.push([...items]); return Promise.resolve(items); });
    release();
    expect(await Promise.all([first, second])).toEqual([1, 2]);
    expect(issued).toEqual([[1], [2]]);
  });

  it("a failed batch rejects its own calls and never jams the lane", async () => {
    const lane = orderedLane();
    const failing = (): Promise<number[]> => Promise.reject(new Error("host gone"));
    const a = lane.joined("page_open", 1, failing);
    const b = lane.joined("page_open", 2, failing);
    const after = lane("page_close", () => Promise.resolve("closed"));
    await expect(a).rejects.toThrow("host gone");
    await expect(b).rejects.toThrow("host gone");
    expect(await after).toBe("closed");
  });
});
