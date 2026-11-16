// GH #704 v2: the tab tree in the session file. `parent` (an earlier index in
// the same snapshot) and `collapsed` are optional and omitted when unused, so a
// session without nesting is byte-identical to v1 and an older build reading a
// v2 session simply restores flat tabs. A bad link restores as a root; the
// session is never refused over it.
import { beforeEach, describe, expect, it } from "vitest";
import { applyParsedSession, buildPersistedSession, parsePersistedSession } from "./session";
import { mainRouter, resetPaneLayoutToSingle } from "./panes";
import type { PaneSnapshot, SerializedTab } from "./router";
import { tabDepths } from "./tabTree";
import { applySidebarSession } from "./ui";
import { readFileSync } from "node:fs";

const st = (name: string, extra: Partial<SerializedTab> = {}): SerializedTab =>
  ({ history: [{ kind: "page", name, pageKind: "page" }], pos: 0, pinned: false, ...extra });
const journalsTab = (extra: Partial<SerializedTab> = {}): SerializedTab =>
  ({ history: [{ kind: "journals" }], pos: 0, pinned: false, ...extra });
const snap = (tabs: SerializedTab[]): PaneSnapshot => ({ tabs, activeIndex: 0 });

const shape = () => {
  const tabs = mainRouter().tabs();
  const d = tabDepths(tabs);
  return tabs.map((t, i) => {
    const r = mainRouter().tabRoute(t);
    return `${"-".repeat(d[i])}${r.kind === "page" ? r.name : r.kind}${t.collapsed ? "+" : ""}`;
  }).join(" ");
};

function restoreFrom(raw: string) {
  const parsed = parsePersistedSession(raw);
  expect(parsed).not.toBeNull();
  applyParsedSession(parsed!);
}

beforeEach(() => {
  resetPaneLayoutToSingle(snap([journalsTab()]));
  applySidebarSession({});
});

describe("session tab tree (GH #704)", () => {
  it("round-trips nesting and collapse through the session file", () => {
    resetPaneLayoutToSingle(snap([journalsTab(), st("A"), st("B", { parent: 1, collapsed: true }), st("C", { parent: 2 }), st("D")]));
    expect(shape()).toBe("journals A -B+ --C D");
    const raw = JSON.stringify(buildPersistedSession());
    resetPaneLayoutToSingle(snap([journalsTab()]));
    restoreFrom(raw);
    expect(shape()).toBe("journals A -B+ --C D");
  });

  it("a session without nesting is byte-identical and carries no tree fields", () => {
    resetPaneLayoutToSingle(snap([journalsTab(), st("A"), st("B")]));
    const raw = JSON.stringify(buildPersistedSession());
    expect(raw).not.toMatch(/"parent"|"collapsed"/);
    resetPaneLayoutToSingle(snap([journalsTab()]));
    restoreFrom(raw);
    expect(JSON.stringify(buildPersistedSession())).toBe(raw);
  });

  it("a flat session is byte-identical to the previous build's writer (frozen at 99923a87b)", () => {
    // Bytes written by buildPersistedSession() at base 99923a87b for this layout
    // (a pinned tab, Journals, a two-entry history, the active tab third).
    const previous = readFileSync(new URL("./fixtures/session-flat-99923a87b.json", import.meta.url), "utf8");
    resetPaneLayoutToSingle({
      tabs: [
        st("Pinned note", { pinned: true }),
        journalsTab(),
        { history: [{ kind: "page", name: "Alpha", pageKind: "page" }, { kind: "page", name: "Beta", pageKind: "page" }], pos: 1, pinned: false },
        st("Gamma"),
      ],
      activeIndex: 2,
    });
    expect(JSON.stringify(buildPersistedSession())).toBe(previous);
    // And the previous writer's file restores to the same bytes.
    resetPaneLayoutToSingle(snap([journalsTab()]));
    restoreFrom(previous);
    expect(JSON.stringify(buildPersistedSession())).toBe(previous);
  });

  it("an old (v1, no-field) session restores flat", () => {
    const v1 = JSON.stringify({ tabs: [journalsTab(), st("A"), st("B")], activeIndex: 1 });
    restoreFrom(v1);
    expect(shape()).toBe("journals A B");
    expect(mainRouter().tabs().every((t) => t.parent === null && !t.collapsed)).toBe(true);
  });

  it("a corrupt parent index restores as a root, never refusing the session", () => {
    const raw = JSON.stringify({
      layout: {
        kind: "pane", paneId: "main", activeIndex: 0,
        tabs: [journalsTab(), st("A", { parent: 1 }), st("B", { parent: 99 }), st("C", { parent: "1" as unknown as number }), st("D", { parent: 1, collapsed: "yes" as unknown as true })],
      },
    });
    restoreFrom(raw);
    expect(shape()).toBe("journals A B C D");
  });

  it("a dropped tab's children move up to its parent; later links follow the shifted indices", () => {
    const raw = JSON.stringify({
      layout: {
        kind: "pane", paneId: "main", activeIndex: 0,
        tabs: [
          journalsTab(),
          st("A"),
          { history: [{ kind: "nonsense" }], pos: 0, pinned: false, parent: 1 }, // invalid route: dropped
          st("B", { parent: 2 }), // child of the dropped tab -> child of A
          st("C", { parent: 3 }),
          st("D"),
          st("E", { parent: 5 }),
          journalsTab({ parent: 5 }), // a second Journals with no page history: dropped
          st("F", { parent: 7 }), // -> child of D
        ],
      },
    });
    restoreFrom(raw);
    expect(shape()).toBe("journals A -B --C D -E -F");
  });
});
