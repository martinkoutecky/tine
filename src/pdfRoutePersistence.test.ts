import { expect, it } from "vitest";
import { parsePersistedSession } from "./session";

it("restores a PDF as an ordinary pane tab with its own page and zoom", () => {
  const pdf = { kind: "pdf", viewId: "pdf-1", filename: "paper.pdf", label: "Paper", page: 7, scale: 1.75 };
  const raw = JSON.stringify({
    tabs: [{ history: [{ kind: "journals" }], pos: 0, pinned: false }],
    activeIndex: 0,
    layout: { kind: "split", dir: "row", ratio: 0.6, children: [
      { kind: "pane", paneId: "main", tabs: [{ history: [{ kind: "journals" }], pos: 0, pinned: false }], activeIndex: 0 },
      { kind: "pane", paneId: "reader", tabs: [{ history: [pdf], pos: 0, pinned: false }], activeIndex: 0 },
    ] },
    focusedPaneId: "reader",
  });
  const restored = parsePersistedSession(raw);
  expect(restored?.snapshots.get("reader")?.tabs[0].history[0]).toEqual(pdf);
  expect(restored?.focusedPaneId).toBe("reader");
});

it("keeps a malformed saved PDF visible as an error tab", () => {
  const raw = JSON.stringify({ tabs: [{ history: [{ kind: "pdf", filename: "../bad.pdf", page: -4 }],
    pos: 0, pinned: false }], activeIndex: 0 });
  const route = parsePersistedSession(raw)?.snapshots.get("main")?.tabs[0].history[0];
  expect(route).toMatchObject({ kind: "invalid", title: "Unavailable PDF" });
});

it("gives restored duplicate PDF tabs separate view identities", () => {
  const pdf = { kind: "pdf", viewId: "same", filename: "paper.pdf", label: "Paper" };
  const raw = JSON.stringify({ tabs: [{ history: [pdf], pos: 0, pinned: false },
    { history: [pdf], pos: 0, pinned: false }], activeIndex: 0 });
  const tabs = parsePersistedSession(raw)?.snapshots.get("main")?.tabs;
  const first = tabs?.[0].history[0];
  const second = tabs?.[1].history[0];
  expect(first?.kind).toBe("pdf");
  expect(second?.kind).toBe("pdf");
  if (first?.kind === "pdf" && second?.kind === "pdf") expect(first.viewId).not.toBe(second.viewId);
});
