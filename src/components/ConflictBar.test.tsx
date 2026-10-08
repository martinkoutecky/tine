import { afterEach, beforeEach, expect, it } from "vitest";
import { render } from "solid-js/web";
import { ConflictBar } from "./ConflictBar";
import { loadFeed, resetStore } from "../document";
import { installRouterBridge } from "../routerBridge";
import type { PageTarget } from "../routeTypes";
import { markConflict } from "../document/save/engine";

let host: HTMLDivElement;
let dispose: (() => void) | undefined;
beforeEach(() => { resetStore(); host = document.createElement("div"); document.body.append(host); });
afterEach(() => { dispose?.(); host.remove(); });

it("N4: a released page explains both outcomes and offers Use disk first", () => {
  markConflict("Source", { kind: "released", partner: "Destination" });
  dispose = render(() => <ConflictBar />, host);
  const message = host.querySelector(".conflict-msg")?.textContent ?? "";
  expect(message).toContain("disk version of “Destination”");
  expect(message).toContain("restore what this page gave");
  expect(message).toContain("moved content is on no page");
  expect([...host.querySelectorAll("button")].map((button) => button.textContent?.trim())).toEqual([
    "Use disk version", "Keep mine (overwrite)",
  ]);
});

it("22a: a live-draft conflict routes to the in-page review and never offers Keep mine (overwrite)", () => {
  loadFeed([{ id: "pages/Live.md", name: "Live", title: "Live", kind: "page", pre_block: null, rev: "r1",
    blocks: [{ id: "live-1", raw: "mine", collapsed: false, children: [] }] }]);
  markConflict("Live", { kind: "disk-changed" });
  const opened: PageTarget[] = [];
  installRouterBridge({ route: () => ({ kind: "journals" }), focusBlock: () => {}, scheduleSessionSave: () => {}, openPageTarget: (t) => opened.push(t) });
  dispose = render(() => <ConflictBar />, host);
  const buttons = [...host.querySelectorAll("button")];
  expect(buttons.map((button) => button.textContent?.trim())).toEqual(["Review"]);
  buttons[0].click();
  expect(opened).toEqual([{ name: "Live", pageKind: "page", path: "pages/Live.md" }]);
});

it("22a: a pathless draft's conflict keeps both choices on the bar", () => {
  markConflict("Nowhere", { kind: "disk-changed" });
  dispose = render(() => <ConflictBar />, host);
  expect([...host.querySelectorAll("button")].map((button) => button.textContent?.trim())).toEqual([
    "Use disk version", "Keep mine (overwrite)",
  ]);
});
