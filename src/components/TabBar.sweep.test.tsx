import { afterEach, expect, it } from "vitest";
import { tabDropTargetAt } from "./TabBar";

afterEach(() => { document.body.innerHTML = ""; });

it.each([
  ["left", 1, 150], ["right", 299, 150],
  ["top", 150, 1], ["bottom", 150, 299],
] as const)("GH #499: outer %s edge previews and targets only the hovered pane", (side, x, y) => {
  document.body.innerHTML = '<div class="content-row"><div data-pane-id="A"></div><div data-pane-id="B"></div></div>';
  const row = document.querySelector<HTMLElement>(".content-row")!;
  const pane = document.querySelector<HTMLElement>('[data-pane-id="A"]')!;
  const rect = new DOMRect(0, 0, 300, 300);
  row.getBoundingClientRect = () => rect;
  pane.getBoundingClientRect = () => rect;
  expect(tabDropTargetAt(pane, x, y, "B")).toEqual({ kind: "split", paneId: "A", side });
});

it("GH #499: below A and right of B remain local in mixed split layouts", () => {
  document.body.innerHTML = '<div class="content-row"><div data-pane-id="A"></div><div data-pane-id="B"></div><div data-pane-id="C"></div></div>';
  const row = document.querySelector<HTMLElement>(".content-row")!;
  const a = document.querySelector<HTMLElement>('[data-pane-id="A"]')!;
  const b = document.querySelector<HTMLElement>('[data-pane-id="B"]')!;
  row.getBoundingClientRect = () => new DOMRect(0, 0, 600, 300);
  a.getBoundingClientRect = () => new DOMRect(0, 0, 300, 300);
  b.getBoundingClientRect = () => new DOMRect(300, 0, 300, 300);
  expect(tabDropTargetAt(a, 150, 299, "C")).toEqual({ kind: "split", paneId: "A", side: "bottom" });
  a.getBoundingClientRect = () => new DOMRect(0, 0, 300, 150);
  expect(tabDropTargetAt(b, 599, 150, "C")).toEqual({ kind: "split", paneId: "B", side: "right" });
});
