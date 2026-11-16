import { afterEach, describe, expect, it, vi } from "vitest";

// D-20261018-W1 (review r1, blocker): a narrow or touch start PRESENTS one
// saved pane but RETAINS the others, their tabs, the saved focus and the saved
// windows; the next session write must carry all of them.
afterEach(() => {
  vi.unstubAllGlobals();
  delete globalThis.__TINE_PLATFORM__;
  vi.resetModules();
});

const leaf = (paneId: string, name: string) => ({
  kind: "pane", paneId,
  tabs: [{ history: [{ kind: "page", name, pageKind: "page" }], pos: 0, pinned: false }],
  activeIndex: 0,
});
const split = (a: unknown, b: unknown) => ({ kind: "split", dir: "row", ratio: 0.5, children: [a, b] });
const names = (node: any): string[] => node.kind === "pane"
  ? node.tabs.map((t: any) => (t.history[t.pos].name ?? t.history[t.pos].filename))
  : node.children.flatMap(names);

async function start(coarse: boolean, width: number, height: number) {
  vi.resetModules();
  globalThis.__TINE_PLATFORM__ = "desktop"; // identity is NOT the variable here
  vi.stubGlobal("innerWidth", width);
  vi.stubGlobal("innerHeight", height);
  vi.stubGlobal("matchMedia", (q: string) => ({
    matches: coarse && (q === "(pointer: coarse)" || q === "(any-pointer: coarse)"),
    addEventListener() {}, removeEventListener() {},
  }));
  return import("./session");
}

describe("single-pane start retains the saved layout", () => {
  it("a touch desktop at 480x600 restores one pane and still writes both panes and the saved focus", async () => {
    const { parsePersistedSession, applyParsedSession, buildPersistedSession } = await start(true, 480, 600);
    const saved = { layout: split(leaf("main", "Alpha"), leaf("second", "Beta")), focusedPaneId: "second" };
    const parsed = parsePersistedSession(JSON.stringify(saved))!;
    applyParsedSession(parsed);
    const written = buildPersistedSession();
    expect(names(written.layout)).toEqual(["Alpha", "Beta"]);
    expect(written.focusedPaneId).toBe("second");
  });

  it("tabs opened in the presented pane land in that pane's saved leaf only", async () => {
    const { parsePersistedSession, applyParsedSession, buildPersistedSession } = await start(true, 480, 600);
    const { mainRouter } = await import("./panes");
    const saved = { layout: split(leaf("main", "Alpha"), leaf("second", "Beta")), focusedPaneId: "main" };
    applyParsedSession(parsePersistedSession(JSON.stringify(saved))!);
    mainRouter().openInNewTab({ kind: "page", name: "Gamma", pageKind: "page" }, true);
    const written = buildPersistedSession();
    expect(names(written.layout)).toEqual(["Alpha", "Gamma", "Beta"]);
  });

  it("a wide window is unchanged: both panes are live and written", async () => {
    const { parsePersistedSession, applyParsedSession, buildPersistedSession } = await start(false, 1280, 800);
    const saved = { layout: split(leaf("main", "Alpha"), leaf("second", "Beta")), focusedPaneId: "second" };
    const parsed = parsePersistedSession(JSON.stringify(saved))!;
    expect(parsed.retained).toBeUndefined();
    applyParsedSession(parsed);
    expect(names(buildPersistedSession().layout)).toEqual(["Alpha", "Beta"]);
  });

  it("once the user changes the pane tree the retained layout is retired", async () => {
    const { parsePersistedSession, applyParsedSession, buildPersistedSession } = await start(true, 480, 600);
    const { resetPaneLayoutToSingle } = await import("./panes");
    const saved = { layout: split(leaf("main", "Alpha"), leaf("second", "Beta")), focusedPaneId: "main" };
    applyParsedSession(parsePersistedSession(JSON.stringify(saved))!);
    resetPaneLayoutToSingle(); // e.g. a graph switch
    expect(names(buildPersistedSession().layout)).toEqual(["Alpha"]);
  });

  it("saved workspace windows survive a single-pane start", async () => {
    const { parsePersistedSession, applyParsedSession, buildPersistedSession } = await start(true, 480, 600);
    const saved = {
      layout: leaf("main", "Alpha"), focusedPaneId: "main",
      windows: [{ layout: leaf("w1", "Elsewhere"), focusedPaneId: "w1" }],
    };
    applyParsedSession(parsePersistedSession(JSON.stringify(saved))!);
    expect(buildPersistedSession().windows?.map((w) => names(w.layout))).toEqual([["Elsewhere"]]);
  });

  // Review r2 (blocker): only the USER acting on the pane tree releases the kept
  // panes. A PDF companion the app creates itself must not.
  const widen = () => {
    vi.stubGlobal("innerWidth", 1280);
    vi.stubGlobal("innerHeight", 900);
    window.dispatchEvent(new Event("resize"));
  };
  const savedTwoPanes = { layout: split(leaf("main", "Alpha"), leaf("second", "Beta")), focusedPaneId: "second" };

  it("widening and opening a PDF keeps the hidden pane, now and on the next launch", async () => {
    const { parsePersistedSession, applyParsedSession, buildPersistedSession } = await start(true, 480, 600);
    const panes = await import("./panes");
    applyParsedSession(parsePersistedSession(JSON.stringify(savedTwoPanes))!);
    widen();
    expect(panes.openPdf("sample.pdf", "sample")).not.toBeNull();
    expect(panes.layoutPaneIds(panes.layoutRoot("main")).length).toBe(2); // the companion exists
    const written = buildPersistedSession();
    expect(names(written.layout).sort()).toEqual(["Alpha", "Beta", "sample.pdf"]);
    expect(written.focusedPaneId).toBe("second");
    const ids: string[] = [];
    const collect = (n: any) => (n.kind === "pane" ? ids.push(n.paneId) : n.children.forEach(collect));
    collect(written.layout);
    expect(new Set(ids).size).toBe(ids.length);
    applyParsedSession(parsePersistedSession(JSON.stringify(written))!);
    expect(names(buildPersistedSession().layout).sort()).toEqual(["Alpha", "Beta", "sample.pdf"]);
  });

  it("an app-made 'open in other pane' split keeps the hidden pane too", async () => {
    const { parsePersistedSession, applyParsedSession, buildPersistedSession } = await start(true, 480, 600);
    const panes = await import("./panes");
    applyParsedSession(parsePersistedSession(JSON.stringify(savedTwoPanes))!);
    widen();
    expect(panes.openRouteInOtherPane({ kind: "page", name: "Gamma", pageKind: "page" }, "main")).not.toBeNull();
    expect(names(buildPersistedSession().layout).sort()).toEqual(["Alpha", "Beta", "Gamma"]);
  });

  it("a divider drag on the companion keeps the hidden pane", async () => {
    const { parsePersistedSession, applyParsedSession, buildPersistedSession } = await start(true, 480, 600);
    const panes = await import("./panes");
    applyParsedSession(parsePersistedSession(JSON.stringify(savedTwoPanes))!);
    widen();
    panes.openPdf("sample.pdf", "sample");
    panes.setSplitRatio([], 0.7, "main");
    expect(names(buildPersistedSession().layout).sort()).toEqual(["Alpha", "Beta", "sample.pdf"]);
  });

  it("a user split, a close or a graph reset after the companion releases the kept panes", async () => {
    for (const act of ["split", "close", "reset"] as const) {
      const { parsePersistedSession, applyParsedSession, buildPersistedSession } = await start(true, 480, 600);
      const panes = await import("./panes");
      applyParsedSession(parsePersistedSession(JSON.stringify(savedTwoPanes))!);
      widen();
      const pdfPane = panes.openPdf("sample.pdf", "sample") && panes.layoutPaneIds(panes.layoutRoot("main")).find((id) => id !== "main")!;
      if (act === "split") panes.splitPane("main", "col");
      else if (act === "close") panes.closePane(pdfPane as string);
      else panes.resetPaneLayoutToSingle();
      const got = names(buildPersistedSession().layout);
      expect(got, act).not.toContain("Beta");
      expect(got, act).toContain("Alpha");
    }
  });
});
