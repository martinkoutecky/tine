import { afterEach, describe, expect, it, vi } from "vitest";
import { createSignal } from "solid-js";
import { render } from "solid-js/web";
import { PeekPopup } from "./PeekPopup";
import type { BlockDto } from "../types";

afterEach(() => { vi.restoreAllMocks(); document.body.replaceChildren(); });

describe("GH #750: preview placement", () => {
  it.each([720, 1080].flatMap(vh => [24, vh / 2, vh - 44].flatMap(y =>
    ["main-content", "right-sidebar"].map(pane => ({ vh, y, pane })))))(
    "clears the anchor at $y in $vh ($pane)", async ({ vh, y, pane }) => {
      vi.spyOn(window, "innerHeight", "get").mockReturnValue(vh);
      vi.spyOn(HTMLElement.prototype, "offsetHeight", "get").mockReturnValue(600);
      const host = document.createElement("div");
      host.className = pane;
      const anchor = document.createElement("a");
      host.append(anchor); document.body.append(host);
      anchor.getBoundingClientRect = () => ({ top: y, bottom: y + 20, left: 1200, right: 1250 }) as DOMRect;
      const dispose = render(() => <PeekPopup anchor={() => anchor} blocks={() => []}
        onPointerEnter={() => {}} onPointerLeave={() => {}} onDismiss={() => {}} />, host);
      try {
        await Promise.resolve();
        const popup = document.querySelector<HTMLElement>(".peek-popup")!;
        const cap = parseFloat(popup.style.maxHeight);
        const height = Math.min(600, cap);
        const top = popup.style.top ? parseFloat(popup.style.top) : vh - parseFloat(popup.style.bottom) - height;
        expect(top >= y + 20 + 6 || top + height <= y - 6).toBe(true);
        expect(top).toBeGreaterThanOrEqual(8);
        expect(top + height).toBeLessThanOrEqual(vh - 8);
        expect(cap).toBeGreaterThanOrEqual(100);
        expect(parseFloat(popup.style.left)).toBeLessThanOrEqual(window.innerWidth - 600 - 8);
      } finally { dispose(); }
    });

  it("keeps its position and cap when open content grows", async () => {
    const host = document.createElement("div"); document.body.append(host);
    const anchor = document.createElement("a"); host.append(anchor);
    anchor.getBoundingClientRect = () => ({ top: 600, bottom: 620, left: 100 }) as DOMRect;
    let height = 120;
    vi.spyOn(HTMLElement.prototype, "offsetHeight", "get").mockImplementation(() => height);
    const [blocks, setBlocks] = createSignal<BlockDto[]>([]);
    const dispose = render(() => <PeekPopup anchor={() => anchor} blocks={blocks}
      onPointerEnter={() => {}} onPointerLeave={() => {}} onDismiss={() => {}} />, host);
    try {
      await Promise.resolve();
      const popup = document.querySelector<HTMLElement>(".peek-popup")!;
      const before = popup.style.cssText;
      height = 600;
      setBlocks([{ id: "growth", raw: "More content", children: [], collapsed: false }]);
      await Promise.resolve();
      expect(popup.style.cssText).toBe(before);
    } finally { dispose(); }
  });

  it("an above preview reserves the room above, so later growth scrolls instead of shrinking it", async () => {
    vi.spyOn(window, "innerHeight", "get").mockReturnValue(720);
    let height = 120;
    vi.spyOn(HTMLElement.prototype, "offsetHeight", "get").mockImplementation(() => height);
    const host = document.createElement("div"); document.body.append(host);
    const anchor = document.createElement("a"); host.append(anchor);
    anchor.getBoundingClientRect = () => ({ top: 650, bottom: 670, left: 100 }) as DOMRect;
    const [blocks, setBlocks] = createSignal<BlockDto[]>([]);
    const dispose = render(() => <PeekPopup anchor={() => anchor} blocks={blocks}
      onPointerEnter={() => {}} onPointerLeave={() => {}} onDismiss={() => {}} />, host);
    try {
      await Promise.resolve();
      const popup = document.querySelector<HTMLElement>(".peek-popup")!;
      expect(parseFloat(popup.style.bottom)).toBe(720 - 650 + 6);
      expect(parseFloat(popup.style.maxHeight)).toBe(600); // min(cap, room above = 636)
      const before = popup.style.cssText;
      height = 500;
      setBlocks([{ id: "growth", raw: "More content", children: [], collapsed: false }]);
      await Promise.resolve();
      expect(popup.style.cssText).toBe(before);
    } finally { dispose(); }
  });

  it.each([
    { y: 300, natural: 80, side: "below" },
    { y: 600, natural: 200, side: "above" },
    { y: 340, natural: 600, side: "below" },
  ])("uses $side for a $natural px preview at $y", async ({ y, natural, side }) => {
    vi.spyOn(window, "innerHeight", "get").mockReturnValue(720);
    vi.spyOn(HTMLElement.prototype, "offsetHeight", "get").mockReturnValue(natural);
    const host = document.createElement("div"); document.body.append(host);
    const anchor = document.createElement("a"); host.append(anchor);
    anchor.getBoundingClientRect = () => ({ top: y, bottom: y + 20, left: 100 }) as DOMRect;
    const dispose = render(() => <PeekPopup anchor={() => anchor} blocks={() => []}
      onPointerEnter={() => {}} onPointerLeave={() => {}} onDismiss={() => {}} />, host);
    try {
      await Promise.resolve();
      const popup = document.querySelector<HTMLElement>(".peek-popup")!;
      if (side === "below") expect(parseFloat(popup.style.top)).toBe(y + 26);
      else expect(parseFloat(popup.style.bottom)).toBe(720 - y + 6);
    } finally { dispose(); }
  });
});
