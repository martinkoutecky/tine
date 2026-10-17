import { afterEach, expect, it } from "vitest";
import { tabDropTargetAt } from "./TabBar";

afterEach(() => { document.body.innerHTML = ""; });

it.each([
  [300, 300, 50, 150, "left"], [300, 300, 250, 150, "right"],
  [300, 300, 150, 85, "top"], [300, 300, 150, 250, "bottom"],
  [1000, 800, 95, 400, "left"], [1000, 800, 97, 400, null],
  [1000, 800, 500, 127, "top"], [1000, 800, 500, 129, null],
  [200, 600, 100, 120, "top"], [600, 200, 85, 120, "left"],
  [100, 100, 23, 65, "left"], [100, 100, 25, 65, null],
  [300, 300, 30, 40, "top"], [300, 300, 5, 60, "left"],
  [300, 300, 150, 160, null],
] as const)("GH #499: %ix%i pane at (%i,%i) chooses %s", (width, height, x, y, side) => {
  document.body.innerHTML = '<div data-pane-id="A"><div class="tab-bar"><div class="tab" data-tab-id="a"></div></div><main></main></div>';
  const pane = document.querySelector<HTMLElement>('[data-pane-id="A"]')!;
  pane.getBoundingClientRect = () => new DOMRect(0, 0, width, height);
  pane.querySelector(".tab-bar")!.getBoundingClientRect = () => new DOMRect(0, 0, width, 32);
  expect(tabDropTargetAt(pane.querySelector("main"), x, y, "B")).toEqual(
    side ? { kind: "split", paneId: "A", side } : { kind: "pane", paneId: "A" },
  );
  expect(tabDropTargetAt(pane.querySelector(".tab-bar"), width / 2, 15, "B")).toMatchObject({ kind: "tab", paneId: "A", before: false });
});

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
