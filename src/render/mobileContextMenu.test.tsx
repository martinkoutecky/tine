import { afterEach, beforeAll, describe, expect, it, vi } from "vitest";
import { render } from "solid-js/web";

vi.mock("../nativeChrome", async (importOriginal) => ({
  ...(await importOriginal<typeof import("../nativeChrome")>()),
  isAndroidPlatform: true,
  isMobilePlatform: true,
}));

import { backend } from "../backend";
import { resetStore } from "../store";
import { AstBody } from "./body";
import { PageRef } from "./inline";
import { initParser } from "./parse";

beforeAll(async () => initParser());

afterEach(() => {
  resetStore();
  vi.restoreAllMocks();
  document.body.replaceChildren();
});

function nativeContextMenuRemainsAvailable(element: Element): void {
  let bubbled = 0;
  const parent = element.parentElement!;
  parent.addEventListener("contextmenu", () => { bubbled += 1; });
  const event = new MouseEvent("contextmenu", { bubbles: true, cancelable: true });
  expect(element.dispatchEvent(event)).toBe(true);
  expect(event.defaultPrevented).toBe(false);
  expect(bubbled).toBe(1);
}

function nativeContextMenuIsOwned(element: Element): void {
  const event = new MouseEvent("contextmenu", { bubbles: true, cancelable: true });
  expect(element.dispatchEvent(event)).toBe(false);
  expect(event.defaultPrevented).toBe(true);
}

describe("Android text-selection contextmenu policy (GH #162)", () => {
  it("gives an explicit page reference's native hold to its page menu", () => {
    const host = document.createElement("div");
    document.body.appendChild(host);
    const dispose = render(() => <PageRef name="Target page" />, host);
    try {
      const pageRef = host.querySelector(".page-ref")!;
      expect(pageRef.hasAttribute("data-page-context-menu")).toBe(true);
      nativeContextMenuIsOwned(pageRef);
    } finally {
      dispose();
    }
  });

  it("does not let a resolved inline block reference bypass native selection", async () => {
    const id = "16200000-0000-4000-8000-000000000001";
    vi.spyOn(backend(), "resolveBlocks").mockResolvedValue([{
      page: "Target page",
      kind: "page",
      blocks: [{ id, raw: `Selectable target\nid:: ${id}`, collapsed: false, children: [] }],
    }]);
    const host = document.createElement("div");
    document.body.appendChild(host);
    const dispose = render(() => <AstBody raw={`See ((${id}))`} />, host);
    try {
      await vi.waitFor(() => expect(host.querySelector(".block-ref")?.textContent).toBe("Selectable target"));
      nativeContextMenuRemainsAvailable(host.querySelector(".block-ref")!);
    } finally {
      dispose();
    }
  });
});

describe("Android page-link hold ownership (GH #207)", () => {
  it("keeps hover preview desktop-only", async () => {
    vi.useFakeTimers();
    const getPage = vi.spyOn(backend(), "getPage").mockResolvedValue({
      name: "Target page",
      kind: "page",
      title: "Target page",
      pre_block: null,
      blocks: [{ id: "target", raw: "Preview body", collapsed: false, children: [] }],
    });
    const host = document.createElement("div");
    document.body.appendChild(host);
    const dispose = render(() => <PageRef name="Target page" />, host);
    try {
      host.querySelector(".page-ref")!.dispatchEvent(new MouseEvent("mouseenter", { bubbles: true }));
      await vi.advanceTimersByTimeAsync(1_000);
      expect(getPage).not.toHaveBeenCalled();
      expect(host.querySelector(".peek-popup")).toBeNull();
    } finally {
      dispose();
      vi.useRealTimers();
    }
  });

});
