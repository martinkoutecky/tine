import { afterEach, beforeAll, expect, it, vi } from "vitest";
import { render } from "solid-js/web";
import type { PageInventory } from "../types";

// UI-CM-SLASH-DAY-TREE-PARENT: the namespace tree lists `a/b/` and `a/c` below
// `a`. That tree is generated output, not the block's text: selecting from its
// label into a child link and pressing Delete must leave the source alone in
// every construct that can hold the macro.
const backendMock = vi.hoisted(() => ({
  graphBindingGeneration: () => 0,
  warmDone: async () => true,
  pageInventory: vi.fn(async (): Promise<PageInventory> => ({
    rev: "1",
    entries: [
      { key: "a", name: "a", is_journal: false, day: null, namespace_parent: true,
        target: { kind: "existing" as const, id: "pages/a.md", others: [] } },
      { key: "a/b/", name: "a/b/", is_journal: false, day: null, parent: { name: "a", kind: "page" as const },
        target: { kind: "absent" as const, id: "pages/a___b___.md" } },
      { key: "a/c", name: "a/c", is_journal: false, day: null, parent: { name: "a", kind: "page" as const },
        target: { kind: "existing" as const, id: "pages/a___c.md", others: [] } },
    ],
  })),
}));
vi.mock("../backend", async (orig) => ({ ...(await orig() as object), backend: () => backendMock }));

import { initParser } from "../render/parse";
import { resetStore } from "../document";
import { doc, setDoc } from "../document/model";
import { installKeybindings } from "../keybindings";
import { Block } from "./Block";
import { refreshPageIndex } from "../pageIndex";

beforeAll(async () => { await initParser(); });
afterEach(() => { window.getSelection()?.removeAllRanges(); resetStore(); document.body.innerHTML = ""; });

const CONSTRUCTS = [
  "{{namespace a}}",
  "# {{namespace a}}",
  "> {{namespace a}}",
  "{{namespace a}}\nheading:: true",
  "#+BEGIN_NOTE\n{{namespace a}}\n#+END_NOTE",
  "## x\n{{namespace a}}",
];

for (const raw of CONSTRUCTS) {
  it(`Delete over the generated namespace tree keeps the source: ${JSON.stringify(raw)}`, async () => {
    await refreshPageIndex();
    setDoc({
      byId: { x: { id: "x", raw, collapsed: false, parent: null, page: "P", children: [] } },
      pages: [{ name: "P", kind: "page", title: "P", preBlock: null, roots: ["x"], format: "md", readOnly: false, guide: false }],
      feed: ["P"],
      loaded: true,
    });
    const host = document.createElement("div");
    document.body.appendChild(host);
    const dispose = render(() => <Block id="x" />, host);
    const keys = installKeybindings();
    try {
      await vi.waitFor(() => expect(host.querySelector(".ns-macro-tree .page-ref")).toBeTruthy());
      const label = host.querySelector(".ns-macro-label")!.firstChild!;
      const child = document.createTreeWalker(host.querySelector(".ns-macro-tree .page-ref")!, NodeFilter.SHOW_TEXT)
        .nextNode() as Text;
      const range = document.createRange();
      range.setStart(label, 0);
      range.setEnd(child, child.length);
      window.getSelection()!.removeAllRanges();
      window.getSelection()!.addRange(range);
      window.dispatchEvent(new KeyboardEvent("keydown", { key: "Delete", bubbles: true, cancelable: true }));
      expect(doc.byId["x"].raw).toBe(raw);
    } finally {
      keys();
      dispose();
    }
  });
}
