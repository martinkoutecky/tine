import { afterEach, beforeAll, describe, expect, it, vi } from "vitest";
import { render } from "solid-js/web";
import { initParser } from "../render/parse";
import { resetStore } from "../document";
import { doc, setDoc } from "../document/model";
import { Block } from "./Block";

beforeAll(initParser);
afterEach(() => {
  resetStore();
  document.body.innerHTML = "";
  vi.restoreAllMocks();
});

function load() {
  setDoc({
    loaded: true, feed: ["P"],
    pages: [{ name: "P", kind: "page", title: "P", preBlock: null, roots: ["first", "second"], format: "md", readOnly: false, guide: false }],
    byId: {
      first: { id: "first", raw: "First", collapsed: false, parent: null, page: "P", children: [] },
      second: { id: "second", raw: "Second", collapsed: false, parent: null, page: "P", children: [] },
    },
  });
}

describe("block drag ownership", () => {
  it("does not move a colliding block after the graph changes during a drag", async () => {
    load();
    const host = document.createElement("div");
    document.body.appendChild(host);
    const dispose = render(() => <><Block id="first" /><Block id="second" /></>, host);
    host.querySelector<HTMLElement>('[data-block-id="first"] .bullet-container')!
      .dispatchEvent(new MouseEvent("mousedown", { bubbles: true, button: 0, clientX: 0, clientY: 0 }));
    dispose();
    resetStore();
    load();
    const disposeNew = render(() => <><Block id="first" /><Block id="second" /></>, host);
    const target = host.querySelector<HTMLElement>('[data-block-id="second"]')!;
    Object.defineProperty(document, "elementFromPoint", { configurable: true, value: () => target });
    vi.spyOn(target.querySelector<HTMLElement>(".block-main")!, "getBoundingClientRect").mockReturnValue({ top: 0, height: 20 } as DOMRect);
    document.dispatchEvent(new MouseEvent("mousemove", { clientX: 10, clientY: 15 }));
    document.dispatchEvent(new MouseEvent("mouseup"));
    await Promise.resolve();
    expect(doc.pages[0].roots).toEqual(["first", "second"]);
    disposeNew();
  });
});
