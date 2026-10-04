import { afterEach, beforeAll, describe, expect, it } from "vitest";
import { For } from "solid-js";
import { render } from "solid-js/web";
import { endEdit, startEditing } from "../editorController";
import { installKeybindings } from "../keybindings";
import { initParser } from "../render/parse";
import { node, pageByName, resetStore, undo } from "../document";
import { loadSingle } from "../document/workingSet";
import { closeContextMenu, contextMenu } from "../ui";
import { Block } from "./Block";
import { ContextMenu } from "./ContextMenu";

beforeAll(initParser);
afterEach(() => { endEdit("page-navigation"); closeContextMenu(); resetStore(); document.body.innerHTML = ""; });
function mountCode(raw: string, format: "md" | "org" = "md", children = false) {
  loadSingle({ name: "Code deletion", title: "Code deletion", kind: "page", format, pre_block: null, blocks: [
    { id: "before", raw: "previous", collapsed: false, children: [] },
    { id: "code", raw, collapsed: false, children: children ? [{ id: "child", raw: "keep me", collapsed: false, children: [] }] : [] },
    { id: "after", raw: "next", collapsed: false, children: [] },
  ] });
  startEditing("code", 0);
  const root = document.createElement("div"); document.body.append(root);
  const dispose = render(() => <><For each={pageByName("Code deletion")?.roots ?? []}>{id => <Block id={id}/>}</For><ContextMenu/></>, root);
  const ta = root.querySelector("textarea")!;
  return { root, ta, dispose };
}
const key = (ta: HTMLTextAreaElement, name: string, ctrlKey = false) => ta.dispatchEvent(new KeyboardEvent("keydown", { key: name, ctrlKey, bubbles: true, cancelable: true }));

describe("existing deletion paths inside code blocks (GH #488)", () => {
  for (const [raw, format] of [["```\n```", "md"], ["~~~\n\n~~~", "md"], ["#+BEGIN_SRC python\n#+END_SRC", "org"]] as const) {
    it(`Backspace removes the empty ${format} wrapper and undo restores it`, () => {
      const { ta, dispose } = mountCode(raw, format);
      try {
        expect(ta.value).toBe(""); key(ta, "Backspace");
        expect(node("code")).toBeUndefined();
        expect(node("before")?.raw).toBe("previous"); expect(node("after")?.raw).toBe("next");
        undo(); expect(node("code")?.raw).toBe(raw);
      } finally { dispose(); }
    });
  }
  it("Backspace at the start of nonempty code preserves the wrapper and surrounding blocks", () => {
    const { ta, dispose } = mountCode("```\nx\n```");
    try { key(ta, "Backspace"); expect(node("code")?.raw).toBe("```\nx\n```"); expect(node("before")?.raw).toBe("previous"); }
    finally { dispose(); }
  });
  it("an empty wrapper with children is retained by Backspace", () => {
    const { ta, dispose } = mountCode("```\n```", "md", true);
    try { key(ta, "Backspace"); expect(node("child")?.raw).toBe("keep me"); expect(node("code")).toBeDefined(); }
    finally { dispose(); }
  });
  it("Mod+A over the whole payload escalates to block selection, then Delete removes the wrapper", () => {
    const disposeKeys = installKeybindings();
    const { ta, dispose } = mountCode("```\nx\n```", "md", true);
    try {
      ta.setSelectionRange(0, ta.value.length); key(ta, "a", true); key(document.body as unknown as HTMLTextAreaElement, "Delete");
      expect(node("code")).toBeUndefined(); expect(node("child")).toBeUndefined();
      expect(pageByName("Code deletion")?.roots).toEqual(["before", "after"]);
    } finally { dispose(); disposeKeys(); }
  });
  it("Backspace clears the wrapper of the last empty block so the page stays editable", () => {
    loadSingle({ name: "Last code", title: "Last code", kind: "page", pre_block: null,
      blocks: [{ id: "last", raw: "```\n```", collapsed: false, children: [] }] });
    startEditing("last", 0);
    const root = document.createElement("div"); document.body.append(root);
    const dispose = render(() => <Block id="last"/>, root);
    try {
      key(root.querySelector("textarea")!, "Backspace");
      expect(node("last")?.raw).toBe(""); expect(root.querySelector("textarea")).toBeTruthy();
      undo(); expect(node("last")?.raw).toBe("```\n```");
    } finally { dispose(); }
  });
  it("right-click inside the code editor offers the existing Delete block action", () => {
    const { root, ta, dispose } = mountCode("```js\nx = 1\n```");
    try {
      ta.dispatchEvent(new MouseEvent("contextmenu", { bubbles: true, cancelable: true }));
      expect(contextMenu()).toMatchObject({ kind: "block", blockId: "code" });
      const action = [...document.querySelectorAll<HTMLElement>(".ctx-item")].find(el => el.textContent?.trim() === "Delete block");
      expect(action, root.textContent ?? "").toBeDefined(); action!.click();
      expect(node("code")).toBeUndefined(); expect(node("after")?.raw).toBe("next");
    } finally { dispose(); }
  });
});
