import { afterEach, beforeAll, describe, expect, it, vi } from "vitest";
import { For } from "solid-js";
import { render } from "solid-js/web";
import { initParser } from "../render/parse";
import { resetStore } from "../document";
import { loadSingle } from "../document/workingSet";
import { pageByName } from "../document/model";
import type { BlockDto, PageDto } from "../types";
import { Block } from "./Block";

beforeAll(async () => {
  await initParser();
});

afterEach(() => {
  vi.restoreAllMocks();
  resetStore();
  Reflect.deleteProperty(document, "elementFromPoint");
  document.documentElement.classList.remove("drag-selection-suppressed");
  document.body.innerHTML = "";
});

const block = (id: string): BlockDto => ({ id, raw: id, collapsed: false, children: [] });
const page = (): PageDto => ({
  name: "Drag",
  kind: "page",
  title: "Drag",
  pre_block: null,
  blocks: ["A", "B", "C", "D", "E"].map(block),
});

describe("Block move drag is not a text gesture (GH #424)", () => {
  it("suppresses document selection while a bullet drag is in flight and releases it on drop", async () => {
    loadSingle(page());

    const host = document.createElement("div");
    document.body.appendChild(host);
    const dispose = render(
      () => <For each={pageByName("Drag")?.roots ?? []}>{(id) => <Block id={id} />}</For>,
      host,
    );
    try {
      const target = host.querySelector<HTMLElement>('[data-block-id="C"]')!;
      const targetMain = target.querySelector<HTMLElement>(".block-main")!;
      vi.spyOn(targetMain, "getBoundingClientRect").mockReturnValue({
        x: 0, y: 100, top: 100, right: 200, bottom: 120, left: 0, width: 200, height: 20,
        toJSON: () => ({}),
      });
      Object.defineProperty(document, "elementFromPoint", {
        configurable: true,
        value: vi.fn(() => target),
      });

      // A selection the press itself started — exactly what WebKit smears
      // across the outline once the pointer moves.
      const removeAllRanges = vi.fn();
      vi.spyOn(document, "getSelection").mockReturnValue({
        rangeCount: 1,
        removeAllRanges,
      } as unknown as Selection);

      const bullet = host.querySelector<HTMLElement>(
        '[data-block-id="A"] > .block-main .bullet-container',
      )!;
      bullet.dispatchEvent(new MouseEvent("mousedown", { button: 0, bubbles: true, clientX: 0, clientY: 0 }));

      // Below the 4px threshold this is still an ordinary click, so nothing
      // may be suppressed yet.
      document.dispatchEvent(new MouseEvent("mousemove", { bubbles: true, clientX: 1, clientY: 1 }));
      expect(document.documentElement.classList.contains("drag-selection-suppressed")).toBe(false);

      document.dispatchEvent(new MouseEvent("mousemove", { bubbles: true, clientX: 10, clientY: 120 }));
      expect(document.documentElement.classList.contains("drag-selection-suppressed")).toBe(true);
      expect(removeAllRanges).toHaveBeenCalled();

      document.dispatchEvent(new MouseEvent("mouseup", { bubbles: true }));
      await Promise.resolve();
      expect(document.documentElement.classList.contains("drag-selection-suppressed")).toBe(false);
    } finally {
      dispose();
    }
  });
});
