import { afterEach, beforeAll, describe, expect, it } from "vitest";
import { For, type JSX } from "solid-js";
import { render } from "solid-js/web";
import { initParser } from "../render/parse";
import { doc, loadSingle, pageByName, resetStore } from "../store";
import { startEditing } from "../editorController";
import type { BlockDto, PageDto } from "../types";
import { Block } from "./Block";

beforeAll(() => initParser());
afterEach(() => { resetStore(); document.body.innerHTML = ""; });

function mount(node: () => JSX.Element) {
  const root = document.createElement("div");
  document.body.appendChild(root);
  return { root, dispose: render(node, root) };
}

function paste(textarea: HTMLTextAreaElement, text: string) {
  const event = new Event("paste", { bubbles: true, cancelable: true });
  Object.defineProperty(event, "clipboardData", { value: { files: [], getData: () => text } });
  textarea.dispatchEvent(event);
}

describe("multiline paste into editor-visible empty blocks", () => {
  it("replaces an id-only host instead of leaving a ghost blank bullet", () => {
    const block: BlockDto = {
      id: "11111111-1111-4111-8111-111111111111",
      raw: "id:: 11111111-1111-4111-8111-111111111111",
      collapsed: false,
      children: [],
    };
    const page: PageDto = { name: "Paste", kind: "page", title: "Paste", pre_block: null, blocks: [block] };
    loadSingle(page);
    startEditing(block.id, 0);
    const { root, dispose } = mount(() => (
      <For each={pageByName("Paste")?.roots ?? []}>{(id) => <Block id={id} />}</For>
    ));
    try {
      const textarea = root.querySelector("textarea") as HTMLTextAreaElement;
      expect(textarea.value).toBe("");
      paste(textarea, "- first\n- second");
      expect(pageByName("Paste")!.roots).toHaveLength(2);
      expect(pageByName("Paste")!.roots).not.toContain(block.id);
      expect(pageByName("Paste")!.roots.map((id) => doc.byId[id].raw)).toEqual(["first", "second"]);
    } finally {
      dispose();
    }
  });
});
