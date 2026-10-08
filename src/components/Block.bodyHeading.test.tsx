import { afterEach, beforeAll, describe, expect, it } from "vitest";
import { render } from "solid-js/web";
import { initParser } from "../render/parse";
import { resetStore } from "../document";
import { loadSingle } from "../document/workingSet";
import type { PageDto } from "../types";
import { Block } from "./Block";

beforeAll(async () => {
  await initParser();
});
afterEach(() => {
  resetStore();
  document.body.innerHTML = "";
});

// F6 (og-F): a heading inside a bullet body (`  # H` under a bullet) is a child
// block whose raw keeps its two-space indent for byte fidelity. OG strips that
// indent in the block content (og@6e7afa8 block.cljs:419), so DISPLAY must show
// the heading with no leading whitespace. The stored raw stays byte-exact.
describe("a heading inside a bullet body", () => {
  it("displays without its raw indentation", async () => {
    const page: PageDto = {
      name: "Body Heading", kind: "page", title: "Body Heading", pre_block: null,
      blocks: [{ id: "bh-1", raw: "x", collapsed: false, children: [{ id: "bh-2", raw: "  # H", collapsed: false, children: [] }, { id: "bh-3", raw: "# H", collapsed: false, children: [] }] }],
    };
    loadSingle(page);
    const root = document.createElement("div");
    document.body.appendChild(root);
    const dispose = render(() => <Block id="bh-1" />, root);
    try {
      const content = (id: string) => root.querySelector(`[data-block-id="${id}"] .block-content`);
      const indented = content("bh-2");
      const plain = content("bh-3");
      expect(indented).not.toBeNull();
      expect(indented!.textContent).toBe("H"); // no leading whitespace on display
      expect(indented!.innerHTML).toBe(plain!.innerHTML); // same as OG's stripped `# H`
    } finally { dispose(); }
  });
});
