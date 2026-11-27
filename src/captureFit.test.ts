// @vitest-environment jsdom
import { afterEach, describe, expect, it } from "vitest";
import { captureContentHeight } from "./captureFit";

function el(cls: string, bottom: number, scrollHeight = 0): HTMLElement {
  const node = document.createElement("div");
  node.className = cls;
  node.getBoundingClientRect = () => ({ bottom } as DOMRect);
  Object.defineProperty(node, "scrollHeight", { value: scrollHeight });
  document.body.append(node);
  return node;
}

afterEach(() => { document.body.innerHTML = ""; });

describe("captureContentHeight", () => {
  it("fits the one-block shell alone", () => {
    el("capture-shell", 100);
    expect(captureContentHeight(document, false)).toBe(118);
  });

  // tine#985: a file link followed in a one-block Capture opens the consent
  // prompt; the window must grow until its whole content, Cancel and Open
  // included, fits under the overlay's 12vh top gutter and 80vh cap.
  it("reserves the full height of an open consent prompt", () => {
    el("capture-shell", 100);
    el("export-modal graph-name-prompt link-scheme-prompt", 110, 240);
    const h = captureContentHeight(document, false);
    const view = h - 18;
    expect(view * 0.8).toBeGreaterThanOrEqual(240);
    expect(view * 0.12 + 240 + 8).toBeLessThanOrEqual(view);
  });

  it("keeps the date picker slab", () => {
    el("capture-shell", 100);
    expect(captureContentHeight(document, true)).toBe(410);
  });
});
