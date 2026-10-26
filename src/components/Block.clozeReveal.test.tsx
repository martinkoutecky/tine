import { afterEach, beforeAll, describe, expect, it } from "vitest";
import { render } from "solid-js/web";
import { initParser } from "../render/parse";
import { resetStore } from "../document";
import { type FeedPage, type Node as StoreNode } from "../document/model";
import { setDoc } from "../document/model";
import { Block } from "./Block";

beforeAll(async () => {
  await initParser();
});

afterEach(() => {
  resetStore();
  document.body.innerHTML = "";
});

// GH #763: the hidden cloze reveals in place on click; the press must not
// open the block editor first (which would unmount the cloze before its click).
describe("{{cloze}} click-to-reveal (GH #763)", () => {
  it("a click on the hidden cloze reveals the answer and keeps the block rendered", () => {
    const id = "cloze-block";
    const page: FeedPage = {
      name: "Cloze",
      kind: "page",
      title: "Cloze",
      preBlock: null,
      roots: [id],
      format: "md",
      readOnly: false,
      guide: false,
    };
    const node: StoreNode = {
      id,
      raw: "Capital of France is {{cloze Paris}}",
      collapsed: false,
      parent: null,
      page: page.name,
      children: [],
    };
    setDoc({ byId: { [id]: node }, pages: [page], feed: [], loaded: true });

    const host = document.createElement("div");
    document.body.appendChild(host);
    const dispose = render(() => <Block id={id} />, host);
    try {
      const cloze = host.querySelector<HTMLElement>(".cloze");
      expect(cloze?.textContent).toBe("[...]");
      cloze!.dispatchEvent(new MouseEvent("mousedown", { bubbles: true, cancelable: true, button: 0 }));
      cloze!.dispatchEvent(new MouseEvent("mouseup", { bubbles: true, cancelable: true, button: 0 }));
      cloze!.click();
      expect(cloze!.isConnected).toBe(true);
      expect(host.querySelector(".cloze")).toBe(cloze);
      expect(cloze!.classList.contains("revealed")).toBe(true);
      expect(cloze!.textContent).toBe("Paris");
      cloze!.click();
      expect(cloze!.textContent).toBe("[...]");
    } finally {
      dispose();
    }
  });
});
