import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { backend } from "./backend";
import { resetStore, setRaw, isDirty } from "./document";
import { loadSingle } from "./document/workingSet";
import { installBackgroundFlush } from "./backgroundFlush";

let hidden = false;
const handlers = new Map<string, () => void>();

beforeEach(() => { resetStore(); hidden = false; handlers.clear(); });
afterEach(() => { vi.restoreAllMocks(); });

function install(closeInFlight = () => false) {
  return installBackgroundFlush({
    endEdit: () => {},
    flushAll: () => import("./document").then((m) => m.flushAll()),
    closeInFlight,
    isHidden: () => hidden,
    addEventListener: ((name: string, fn: () => void) => { handlers.set(name, fn); }) as typeof document.addEventListener,
    removeEventListener: ((name: string) => { handlers.delete(name); }) as typeof document.removeEventListener,
  });
}

describe("background durability", () => {
  it("writes a dirty edit on hide before the debounce fires", async () => {
    const write = vi.spyOn(backend(), "savePages").mockResolvedValue({ ok: ["rev"] });
    loadSingle({ name: "Hide", kind: "page", title: "Hide", pre_block: null,
      blocks: [{ id: "leaf", raw: "old", collapsed: false, children: [] }] });
    setRaw("leaf", "new");
    expect(isDirty("Hide")).toBe(true);
    const dispose = install();
    hidden = true;
    handlers.get("visibilitychange")!();
    await vi.waitFor(() => expect(write).toHaveBeenCalledOnce());
    expect(write.mock.calls[0][0][0].page.blocks[0].raw).toBe("new");
    dispose();
  });

  it("deduplicates hide/pagehide/freeze, yields to close, and unregisters", async () => {
    let finish!: (value: boolean) => void;
    const flushAll = vi.fn(() => new Promise<boolean>((resolve) => { finish = resolve; }));
    const closeInFlight = vi.fn(() => false);
    const dispose = installBackgroundFlush({
      endEdit: () => {}, flushAll, closeInFlight, isHidden: () => hidden,
      addEventListener: ((name: string, fn: () => void) => { handlers.set(name, fn); }) as typeof document.addEventListener,
      removeEventListener: ((name: string) => { handlers.delete(name); }) as typeof document.removeEventListener,
    });
    handlers.get("visibilitychange")!();
    expect(flushAll).not.toHaveBeenCalled();
    hidden = true;
    for (const name of ["visibilitychange", "pagehide", "freeze"]) handlers.get(name)!();
    expect(flushAll).toHaveBeenCalledOnce();
    finish(true);
    await Promise.resolve();
    await Promise.resolve();
    handlers.get("pagehide")!();
    expect(flushAll).toHaveBeenCalledTimes(2);
    finish(true);
    await Promise.resolve();
    await Promise.resolve();
    closeInFlight.mockReturnValue(true);
    handlers.get("pagehide")!();
    expect(flushAll).toHaveBeenCalledTimes(2);
    dispose();
    expect(handlers.size).toBe(0);
  });
});
