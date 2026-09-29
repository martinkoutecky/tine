import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { createResource, createRoot } from "solid-js";
import { render } from "solid-js/web";
import { readLatestOr, readOr, resetResourceReportsForTests } from "./resourceRead";

const dbgLines = vi.hoisted(() => [] as string[]);
vi.mock("./debug", () => ({ dbg: (line: string) => dbgLines.push(line) }));

beforeEach(() => {
  resetResourceReportsForTests();
  dbgLines.length = 0;
});

afterEach(() => {
  document.body.innerHTML = "";
  vi.restoreAllMocks();
});

/** A resource that has already settled, so the reads below are synchronous. */
async function settled<T>(load: () => Promise<T>) {
  return createRoot(async () => {
    const [resource] = createResource(load);
    await Promise.allSettled([load().catch(() => undefined)]);
    // One more microtask turn for Solid to record the rejection.
    await Promise.resolve();
    await Promise.resolve();
    return resource;
  });
}

describe("readOr (GH #490/#332: a rejected resource must not throw into render)", () => {
  it("control: reading a rejected resource throws, which is the whole defect", async () => {
    const resource = await settled(() => Promise.reject(new Error("getBacklinks failed")));
    expect(() => resource()).toThrow("getBacklinks failed");
    expect(() => resource.latest).toThrow("getBacklinks failed");
  });

  it("returns the fallback instead, so the branch the site already wrote can run", async () => {
    const resource = await settled(() => Promise.reject(new Error("getBacklinks failed")));
    expect(readOr(resource, undefined, "linked references")).toBeUndefined();
    expect(readOr(resource, [], "linked references")).toEqual([]);
    expect(readLatestOr(resource, "(couldn’t read)", "journal conflict file")).toBe("(couldn’t read)");
  });

  it("returns the value untouched when the resource succeeded", async () => {
    const resource = await settled(async () => ["a", "b"]);
    expect(readOr(resource, [], "linked references")).toEqual(["a", "b"]);
    expect(readLatestOr(resource, [], "linked references")).toEqual(["a", "b"]);
  });

  it("records the failure once, in the opt-in debug log only, never the always-on console", async () => {
    const warn = vi.spyOn(console, "warn").mockImplementation(() => {});
    const resource = await settled(() => Promise.reject(new Error("page unreadable")));
    readOr(resource, undefined, "page inventory");
    readOr(resource, undefined, "page inventory");
    readLatestOr(resource, undefined, "page inventory");

    expect(dbgLines).toEqual(["resource failed: page inventory: Error: page unreadable"]);
    expect(warn).not.toHaveBeenCalled();
  });

  it("keeps a sibling rendering, where the bare read took the whole tree down", async () => {
    const failing = await settled(() => Promise.reject(new Error("listKnownGraphs failed")));

    const bare = document.createElement("div");
    document.body.appendChild(bare);
    expect(() =>
      render(() => (
        <>
          <div class="value">{String(failing())}</div>
          <div class="sibling">still here</div>
        </>
      ), bare),
    ).toThrow("listKnownGraphs failed");
    expect(bare.querySelector(".sibling")).toBeNull();

    const guarded = document.createElement("div");
    document.body.appendChild(guarded);
    render(() => (
      <>
        <div class="value">{String(readOr(failing, "none", "known graphs"))}</div>
        <div class="sibling">still here</div>
      </>
    ), guarded);
    expect(guarded.querySelector(".value")?.textContent).toBe("none");
    expect(guarded.querySelector(".sibling")?.textContent).toBe("still here");
  });
});
