// GH #211 (master 0236f5785, og 22b): drag-reorder of right-sidebar open items — pointer drag reorders
// within the list only, clicks under the threshold keep navigating, and the
// reordered array persists through the existing rightSidebar owner.
import { afterEach, beforeAll, describe, expect, it, vi } from "vitest";
import { render } from "solid-js/web";
import { backend } from "../backend";
import { initParser } from "../render/parse";
import { resetStore } from "../document";
import { loadSingle } from "../document/workingSet";
import { route, openJournals } from "../router";
import { applySidebarSession, rightSidebar, setRightSidebar } from "../ui";
import { rowReorderClickSuppressed } from "./rowReorder";
import { RightSidebar } from "./RightSidebar";
import type { PageDto } from "../types";

function pageDto(name: string): PageDto {
  return {
    name,
    kind: "page",
    title: name,
    pre_block: null,
    blocks: [{ id: `${name}-root`, raw: `${name} body`, collapsed: false, children: [] }],
  };
}

beforeAll(async () => {
  await initParser();
});

afterEach(async () => {
  // Let completed-drag click-suppression timers (setTimeout(0)) elapse between
  // tests — a browser swallows the drop's synthesized click in the same task,
  // but these tests dispatch clicks explicitly without a browser interlock.
  await new Promise((r) => setTimeout(r, 5));
  setRightSidebar([]);
  applySidebarSession({ right: false, items: [] });
  openJournals();
  resetStore();
  document.body.innerHTML = "";
  localStorage.clear();
  vi.restoreAllMocks();
});

function rect(left: number, top: number, width: number, height: number): DOMRect {
  return {
    x: left, y: top, width, height, left, top,
    right: left + width, bottom: top + height, toJSON: () => ({}),
  } as DOMRect;
}

function setRect(el: Element | null, left: number, top: number, width: number, height: number) {
  Object.defineProperty(el, "getBoundingClientRect", {
    configurable: true,
    value: () => rect(left, top, width, height),
  });
}

function pointer(type: string, x: number, y: number): PointerEvent {
  const Ctor = (window as { PointerEvent?: typeof PointerEvent }).PointerEvent ?? MouseEvent;
  return new Ctor(type, { bubbles: true, cancelable: true, clientX: x, clientY: y, button: 0, buttons: 1 }) as PointerEvent;
}

function mountThreePages() {
  loadSingle(pageDto("Alpha"));
  loadSingle(pageDto("Beta"));
  loadSingle(pageDto("Gamma"));
  applySidebarSession({
    right: true,
    items: [
      { kind: "page" as const, name: "Alpha", pageKind: "page" as const },
      { kind: "page" as const, name: "Beta", pageKind: "page" as const },
      { kind: "page" as const, name: "Gamma", pageKind: "page" as const },
    ],
  });
  vi.spyOn(backend(), "getBacklinks").mockResolvedValue([]);
  vi.spyOn(backend(), "getUnlinkedRefs").mockResolvedValue([]);
  const root = document.createElement("div");
  document.body.appendChild(root);
  document.title = "";
  const dispose = render(() => <RightSidebar />, root);
  const rows = () => [...root.querySelectorAll<HTMLElement>(".rs-item")];
  const names = () => rightSidebar().map((item) => (item.kind === "page" ? item.name : item.page));
  return { root, dispose, rows, names };
}

describe("right-sidebar drag reorder (GH #211)", () => {
  it("PROBE: plain title click navigates without any drag", () => {
    const { dispose, rows } = mountThreePages();
    const first = rows()[0]!;
    (first.querySelector(".rs-item-title") as HTMLElement).click();
    expect(route()).toMatchObject({ kind: "page", name: "Alpha" });
    dispose();
  });

  it("reorders within the list on a pointer drag past the threshold and persists", () => {
    const { dispose, rows, names } = mountThreePages();
    expect(names()).toEqual(["Alpha", "Beta", "Gamma"]);

    const [first, second, third] = rows();
    setRect(first, 0, 0, 200, 40);
    setRect(second, 0, 40, 200, 40);
    setRect(third, 0, 80, 200, 40);

    // Drag the FIRST row down past Gamma's midpoint (insert after Gamma).
    first.querySelector(".rs-item-head")!.dispatchEvent(pointer("pointerdown", 10, 10));
    const prevElementFromPoint = document.elementFromPoint;
    try {
      document.elementFromPoint = () => third;
      document.dispatchEvent(pointer("pointermove", 10, 100));
      // Live indicator marks Gamma as the drop-after target.
      expect(third.classList.contains("row-drop-after")).toBe(true);
      document.dispatchEvent(pointer("pointerup", 10, 100));
    } finally {
      document.elementFromPoint = prevElementFromPoint;
    }

    expect(names()).toEqual(["Beta", "Gamma", "Alpha"]);
    expect(third.classList.contains("row-drop-after")).toBe(false);
    // Persistence owner wrote the new order to localStorage (session save is mocked).
    const persisted = JSON.parse(localStorage.getItem("logseq-claude.rightSidebarItems") ?? "[]") as { name?: string }[];
    expect(persisted.map((p) => p.name)).toEqual(["Beta", "Gamma", "Alpha"]);
    dispose();
  });

  it("keeps a sub-threshold press as an ordinary click (no reorder, navigation fires)", () => {
    const { dispose, rows, names } = mountThreePages();
    const first = rows()[0]!;
    setRect(first, 0, 0, 200, 40);

    first.querySelector(".rs-item-head")!.dispatchEvent(pointer("pointerdown", 10, 10));
    document.dispatchEvent(pointer("pointermove", 12, 12)); // < 4px
    document.dispatchEvent(pointer("pointerup", 12, 12));

    expect(names()).toEqual(["Alpha", "Beta", "Gamma"]);
    expect(rowReorderClickSuppressed()).toBe(false);
    // The title click still navigates.
    (first.querySelector(".rs-item-title") as HTMLElement).click();
    expect(route()).toMatchObject({ kind: "page", name: "Alpha" });
    dispose();
  });

  it("the click that ends a completed drag is swallowed; a later click navigates", async () => {
    const { root, dispose, rows } = mountThreePages();
    const first = rows()[0]!;
    const second = rows()[1]!;
    setRect(first, 0, 0, 200, 40);
    setRect(second, 0, 40, 200, 40);

    first.querySelector(".rs-item-head")!.dispatchEvent(pointer("pointerdown", 10, 10));
    const prevElementFromPoint = document.elementFromPoint;
    document.elementFromPoint = () => second;
    try {
      document.dispatchEvent(pointer("pointermove", 10, 60)); // second's lower half
      document.dispatchEvent(pointer("pointerup", 10, 60));
    } finally {
      document.elementFromPoint = prevElementFromPoint;
    }
    expect(rightSidebar().length).toBe(3);

    // The click synthesized by the drop is suppressed — no navigation.
    (root.querySelector(".rs-item-title") as HTMLElement).click();
    expect(route()).toMatchObject({ kind: "journals" });

    // After the suppression window, an ordinary click navigates again.
    await new Promise((r) => setTimeout(r, 5));
    (root.querySelector(".rs-item-title") as HTMLElement).click();
    expect(route()).toMatchObject({ kind: "page" });
    dispose();
  });
});
