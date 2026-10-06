// OG-MULTIWINDOW P6: workspace windows in the saved session. A one-window
// session is byte-identical to before; saved windows round-trip with their own
// pane trees and geometry; a malformed entry is dropped while the rest restore.
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { readFileSync } from "node:fs";
import path from "node:path";
import {
  applyParsedSession,
  buildPersistedSession,
  installWorkspaceWindowSession,
  parsePersistedSession,
  type ParsedWindow,
  type PersistedSession,
  type WindowGeometry,
} from "./session";
import { applySidebarSession } from "./ui";
import { createWindowLayout, dropWindowLayout, layoutPaneIds, layoutRoot, layoutWindowIds, paneRouter, resetPaneLayoutToSingle, splitPane } from "./panes";
import type { PaneSnapshot } from "./router";
import { MAX_WORKSPACE_WINDOWS } from "./windowRealm";

const journals = (): PaneSnapshot => ({ tabs: [{ history: [{ kind: "journals" }], pos: 0, pinned: false }], activeIndex: 0 });
const page = (name: string): PaneSnapshot => ({
  tabs: [{ history: [{ kind: "page", name, pageKind: "page" }], pos: 0, pinned: false }],
  activeIndex: 0,
});

let open: { id: string; geometry: WindowGeometry | null }[] = [];
const closeAll = vi.fn(() => { for (const w of open) dropWindowLayout(w.id); open = []; });
const reopen = vi.fn((_windows: ParsedWindow[]) => {});
let uninstall: () => void = () => {};

beforeEach(() => {
  resetPaneLayoutToSingle(journals());
  applySidebarSession({});
  open = [];
  closeAll.mockClear();
  reopen.mockClear();
  uninstall = installWorkspaceWindowSession({ list: () => open, closeAll, open: reopen });
});
afterEach(() => {
  closeAll();
  uninstall();
});

function openWindow(id: string, snapshot: PaneSnapshot, geometry: WindowGeometry | null = null): string {
  const paneId = createWindowLayout(id, snapshot);
  open.push({ id, geometry });
  return paneId;
}

describe("workspace windows in the session (P6)", () => {
  it("adds zero bytes when no workspace window is open", () => {
    const withHandlers = JSON.stringify(buildPersistedSession());
    uninstall();
    expect(JSON.stringify(buildPersistedSession())).toBe(withHandlers);
    expect(withHandlers).not.toContain("windows");
  });

  it("round-trips each window's pane tree, focused pane and geometry", () => {
    const first = openWindow("ws-1", page("Alpha"), { x: 40, y: 50, width: 900, height: 700 });
    const second = splitPane(first, "row")!;
    paneRouter(second).restoreSnapshot(page("Beta"));
    openWindow("ws-2", page("Gamma"));
    const saved = buildPersistedSession();
    expect(saved.windows).toHaveLength(2);
    expect(saved.windows![0].geometry).toEqual({ x: 40, y: 50, width: 900, height: 700 });
    expect(saved.windows![1].geometry).toBeUndefined();

    const parsed = parsePersistedSession(JSON.stringify(saved))!;
    expect(parsed.windows).toHaveLength(2);
    expect(layoutPaneIds(parsed.windows[0].layout)).toEqual([first, second]);
    expect(parsed.windows[0].snapshots.get(second)?.tabs[0].history[0]).toMatchObject({ kind: "page", name: "Beta" });
    expect(parsed.windows[0].geometry).toEqual({ x: 40, y: 50, width: 900, height: 700 });
    expect(parsed.windows[1].geometry).toBeNull();
    expect(parsed.windows[1].snapshots.size).toBe(1);

    applyParsedSession(parsed);
    // The open windows close BEFORE main's tree is restored, then the saved ones reopen.
    expect(closeAll).toHaveBeenCalledOnce();
    expect(closeAll.mock.invocationCallOrder[0]).toBeLessThan(reopen.mock.invocationCallOrder[0]);
    expect(reopen).toHaveBeenCalledWith(parsed.windows);
    expect(layoutWindowIds()).toEqual(["main"]);
  });

  it("drops a malformed or colliding window and restores the rest", () => {
    const paneId = openWindow("ws-1", page("Alpha"), { x: 1, y: 2, width: 800, height: 600 });
    const saved = buildPersistedSession();
    const good = saved.windows![0];
    const raw: PersistedSession = {
      ...saved,
      windows: [
        "garbage" as never,
        { layout: { kind: "split" } as never },
        { layout: { ...good.layout, paneId: "main" } as never },
        { ...good, geometry: { x: "a", y: 0, width: 10, height: 10 } as never },
        { ...good }, // repeats the previous window's pane id: dropped
        { layout: { kind: "pane", paneId: "pane-x", tabs: "nope" } as never },
      ],
    };
    const parsed = parsePersistedSession(JSON.stringify(raw))!;
    expect(parsed).not.toBeNull();
    expect(parsed.windows).toHaveLength(1);
    expect(layoutPaneIds(parsed.windows[0].layout)).toEqual([paneId]);
    expect(parsed.windows[0].geometry).toBeNull();
    expect(layoutPaneIds(parsed.layout)).toEqual(["main"]);
  });

  it("keeps at most MAX_WORKSPACE_WINDOWS saved windows", () => {
    const saved = buildPersistedSession();
    const windows = Array.from({ length: MAX_WORKSPACE_WINDOWS + 4 }, (_, i) => ({
      layout: { kind: "pane" as const, paneId: `pane-w${i}`, ...page(`P${i}`) },
    }));
    const parsed = parsePersistedSession(JSON.stringify({ ...saved, windows }))!;
    expect(parsed.windows).toHaveLength(MAX_WORKSPACE_WINDOWS);
  });

  it("does not save a listed window that owns no pane tree", () => {
    open.push({ id: "ws-ghost", geometry: null });
    expect(buildPersistedSession().windows).toBeUndefined();
    expect(layoutRoot("ws-ghost")).toEqual(layoutRoot("main"));
  });

  it("names the same window bound as the native side", () => {
    const rust = readFileSync(path.join(__dirname, "..", "src-tauri", "src", "workspace_windows.rs"), "utf8");
    expect(rust).toMatch(new RegExp(`pub\\(crate\\) const MAX_WORKSPACE_WINDOWS: usize = ${MAX_WORKSPACE_WINDOWS};`));
  });

  it("listens for the event names the native side emits", () => {
    const rust = readFileSync(path.join(__dirname, "..", "src-tauri", "src", "workspace_windows.rs"), "utf8");
    const js = readFileSync(path.join(__dirname, "workspaceWindows.ts"), "utf8");
    for (const constant of ["CLOSE_REQUESTED_EVENT", "DESTROYED_EVENT"]) {
      const name = new RegExp(`pub\\(crate\\) const ${constant}: &str = "([^"]+)";`).exec(rust)?.[1];
      expect(name, constant).toBeTruthy();
      expect(js).toContain(`listen<string>("${name}"`);
    }
  });
});
