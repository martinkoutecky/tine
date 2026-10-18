import { afterEach, beforeAll, describe, expect, it } from "vitest";
import { For, type JSX } from "solid-js";
import { render } from "solid-js/web";
import { startEditing } from "./editorController";
import { initParser } from "./render/parse";
import { resetStore } from "./document";
import { loadSingle } from "./document/workingSet";
import { pageByName } from "./document/model";
import { Block } from "./components/Block";
import { installTextareaPrimarySelectionGuard } from "./textareaPrimarySelection";

// GH #530. jsdom cannot show WebKitGTK's PRIMARY flavours; the native journey
// (scripts/e2e-editor-bugs.mjs) asserts the user outcome. This pins the
// mechanism that journey proves: every editing textarea has a placeholder
// element after its inner editor, so WebKit's selection serialization has an end.

beforeAll(() => initParser());
afterEach(() => {
  resetStore();
  document.body.innerHTML = "";
});

function mount(node: () => JSX.Element) {
  const root = document.createElement("div");
  document.body.appendChild(root);
  return { root, dispose: render(node, root) };
}

describe("textarea PRIMARY-selection guard (GH #530)", () => {
  it("the block editor textarea always carries a non-empty placeholder", () => {
    loadSingle({ name: "Primary", kind: "page", title: "Primary", pre_block: null, blocks: [
      { id: "one", raw: "single line block", collapsed: false, children: [] },
    ] });
    startEditing("one", 0);
    const mounted = mount(() => <For each={pageByName("Primary")?.roots ?? []}>{(id) => <Block id={id} />}</For>);
    try {
      const textarea = mounted.root.querySelector<HTMLTextAreaElement>("textarea.block-editor")!;
      expect(textarea.placeholder).not.toBe("");
    } finally {
      mounted.dispose();
    }
  });

  it("guards other textareas on focus and keeps a real placeholder", () => {
    const uninstall = installTextareaPrimarySelectionGuard();
    try {
      const bare = document.createElement("textarea");
      const hinted = document.createElement("textarea");
      hinted.placeholder = "Search";
      const input = document.createElement("input");
      document.body.append(bare, hinted, input);
      bare.focus();
      hinted.focus();
      input.focus();
      expect(bare.placeholder).not.toBe("");
      expect(hinted.placeholder).toBe("Search");
      expect(input.placeholder).toBe("");
    } finally {
      uninstall();
    }
  });
});
