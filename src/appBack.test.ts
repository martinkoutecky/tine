import { describe, expect, it, vi } from "vitest";
import { appBackAvailable, dispatchAppBack, type AppBackDeps } from "./appBack";

function deps(over: Partial<{ transient: boolean; drawer: boolean; moved: boolean }> = {}) {
  const s = { transient: false, drawer: false, moved: false, ...over };
  const d = {
    dismissTransient: vi.fn(() => s.transient),
    dismissDrawer: vi.fn(() => s.drawer),
    restoreDrawerFocus: vi.fn(),
    historyBack: vi.fn(() => s.moved),
    closeRoot: vi.fn(),
  } satisfies AppBackDeps;
  return d;
}

describe("the one Back ladder (GH #492, #501)", () => {
  it("takes exactly one rung, highest first", () => {
    const t = deps({ transient: true, drawer: true, moved: true });
    expect(dispatchAppBack(t)).toBe("transient");
    expect(t.dismissDrawer).not.toHaveBeenCalled();
    expect(t.historyBack).not.toHaveBeenCalled();

    const d = deps({ drawer: true, moved: true });
    expect(dispatchAppBack(d)).toBe("drawer");
    expect(d.restoreDrawerFocus).toHaveBeenCalledOnce();
    expect(d.historyBack).not.toHaveBeenCalled();

    const h = deps({ moved: true });
    expect(dispatchAppBack(h)).toBe("history");
    expect(h.closeRoot).not.toHaveBeenCalled();

    const r = deps();
    expect(dispatchAppBack(r)).toBe("root");
    expect(r.closeRoot).toHaveBeenCalledOnce();
  });

  it("answers 'would Back do anything' without performing it", () => {
    const probe = (t: boolean, d: boolean, g: boolean) => appBackAvailable({
      hasTransient: () => t, hasDrawer: () => d, canGoBack: () => g,
    });
    expect(probe(false, false, false)).toBe(false);
    expect(probe(true, false, false)).toBe(true);
    expect(probe(false, true, false)).toBe(true);
    expect(probe(false, false, true)).toBe(true);
  });
});
