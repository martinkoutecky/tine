import { editorIn, maybeEditorIn } from "../tests/editorDriver";
import { describe, expect, it, vi } from "vitest";
import { writeClipboardText } from "../clipboard";
import { For } from "solid-js";
import { pageByName, setRaw } from "../document";
import { loadSingle } from "../document/workingSet";
import { startEditing } from "../editorController";
import type { PageDto } from "../types";
import { Block } from "./Block";
import { installBlockEditorLifecycle, mount, blk, page } from "../tests/blockEditorTestkit";
import { Editor } from "./Block";

// GH #357/#412/#413: body editing retains the code card and exact wrapper
// bytes. D-20261114-CS879: raw source always uses ordinary block presentation.

installBlockEditorLifecycle();

vi.mock("../clipboard", () => ({ writeClipboardText: vi.fn().mockResolvedValue(undefined) }));

describe("code-fence editor presentation", () => {
  it.each(["md", "org"] as const)("copies the current %s code body while preserving the editor and caret", async format => {
    const open = format === "md" ? "```js\n" : "#+BEGIN_SRC js\n";
    const close = format === "md" ? "\n```" : "\n#+END_SRC";
    loadSingle({ ...page("Copy code", [blk("copy-code", `${open}const old = 1;${close}`)]), format });
    const id = pageByName("Copy code")!.roots[0];
    startEditing(id, open.length);
    const { root, dispose } = mount(() => <Block id={id} />);
    try {
      const editor = editorIn(root);
      editor.input("const fresh = '文😀';", 6);
      editor.focus();
      const copy = root.querySelector<HTMLButtonElement>(".code-copy");
      expect(copy).not.toBeNull();
      const press = new MouseEvent("mousedown", { bubbles: true, cancelable: true });
      copy!.dispatchEvent(press);
      expect(press.defaultPrevented).toBe(true);
      copy!.click();
      await Promise.resolve();
      expect(writeClipboardText).toHaveBeenLastCalledWith("const fresh = '文😀';");
      expect(document.activeElement).toBe(editor.element);
      expect(editor.selection()).toEqual([6, 6]);
      // Raw source drops the code-only chrome along with card presentation.
      editor.keydown({ key: "Escape" });
      expect(editorIn(root).value()).toBe(`${open}const fresh = '文😀';${close}`);
      expect(root.querySelector(".code-copy")).toBeNull();
    } finally { dispose(); }
  });

  it("presents the editor of a code-only fenced block as the same code card", () => {
    loadSingle(page("Code", [blk("c1", "```js\nconst x = 1;\nconsole.log(x);\n```")]));
    const id = pageByName("Code")!.roots[0];
    startEditing(id, "```js\n".length);
    const { root, dispose } = mount(() => (
      <For each={pageByName("Code")?.roots ?? []}>{(bid) => <Block id={bid} />}</For>
    ));
    try {
      const ta = maybeEditorIn(root)!;
      expect(ta.element.classList.contains("code-edit")).toBe(true);
      // Default is horizontal scrolling, shared with the rendered code card.
      if (ta.kind === "textarea") expect(ta.element.getAttribute("wrap")).toBe("off");
      else expect(ta.element.classList.contains("cm-lineWrapping")).toBe(false);
      // Body-only code view (GH #412/#413): the payload, without the fences;
      // the wrapper bytes are preserved on commit (see codeBodyEdit tests).
      expect(ta.value()).toBe("const x = 1;\nconsole.log(x);");
      expect.soft(root.querySelector(".code-language")?.textContent).toBe("js");
      expect.soft([...root.querySelectorAll(".calc-lineno")].map(el => el.textContent)).toEqual(["1", "2"]);
    } finally {
      dispose();
    }
  });

  it("keeps ordinary editing presentation for mixed paragraph+fence content", () => {
    loadSingle(page("Code", [blk("c2", "intro\n```js\nx\n```") ]));
    const id = pageByName("Code")!.roots[0];
    startEditing(id, 0);
    const { root, dispose } = mount(() => (
      <For each={pageByName("Code")?.roots ?? []}>{(bid) => <Block id={bid} />}</For>
    ));
    try {
      const ta = maybeEditorIn(root)!;
      expect(ta.element.classList.contains("code-edit")).toBe(false);
    } finally {
      dispose();
    }
  });

  it("keeps a calc fence in its specialized editor mode (not code presentation)", () => {
    loadSingle(page("Calc", [blk("c3", "```calc\n1+1\n```") ]));
    const id = pageByName("Calc")!.roots[0];
    startEditing(id, 0);
    const { root, dispose } = mount(() => (
      <For each={pageByName("Calc")?.roots ?? []}>{(bid) => <Block id={bid} />}</For>
    ));
    try {
      const ta = maybeEditorIn(root)!;
      expect(ta.element.classList.contains("code-edit")).toBe(false);
      // Calc strips the fence in the editor and shows the live results panel.
      expect(ta.value()).toBe("1+1");
      expect(root.querySelector(".calc-results")).not.toBeNull();
    } finally {
      dispose();
    }
  });

  it("org #+BEGIN_SRC blocks get the same code-card editing", () => {
    loadSingle({ ...page("OrgCode", [blk("c4", "#+BEGIN_SRC python\nx = 1\n#+END_SRC")]), format: "org" } as PageDto);
    const id = pageByName("OrgCode")!.roots[0];
    startEditing(id, "#+BEGIN_SRC python\n".length);
    const { root, dispose } = mount(() => (
      <For each={pageByName("OrgCode")?.roots ?? []}>{(bid) => <Block id={bid} />}</For>
    ));
    try {
      const ta = maybeEditorIn(root)!;
      expect(ta.element.classList.contains("code-edit")).toBe(true);
      if (ta.kind === "textarea") expect(ta.element.getAttribute("wrap")).toBe("off");
      else expect(ta.element.classList.contains("cm-lineWrapping")).toBe(false);
      expect(root.querySelector(".code-language")?.textContent).toBe("python");
      expect(root.querySelector(".calc-lineno")?.textContent).toBe("1");
    } finally {
      dispose();
    }
  });

  it("raw source keeps ordinary presentation when surrounding prose changes", async () => {
    const fenced = "```js\nconst x = 1;\n```";
    loadSingle(page("Live", [blk("c5", fenced)]));
    const id = pageByName("Live")!.roots[0];
    startEditing(id, 0);
    const { root, dispose } = mount(() => (
      <For each={pageByName("Live")?.roots ?? []}>{(bid) => <Editor id={bid} />}</For>
    ));
    try {
      const ta = maybeEditorIn(root)!;
      expect(ta.element.classList.contains("code-edit")).toBe(false);
      expect(ta.value()).toBe(fenced);
      setRaw(id, "```js\nconst x = 1;\n```\nplain note");
      await Promise.resolve();
      expect(ta.element.classList.contains("code-edit")).toBe(false);
      expect(ta.value()).toBe("```js\nconst x = 1;\n```\nplain note");
      // Removing unrelated prose cannot turn source into a code card.
      setRaw(id, fenced);
      await Promise.resolve();
      expect(ta.element.classList.contains("code-edit")).toBe(false);
      expect(ta.value()).toBe(fenced);
    } finally {
      dispose();
    }
  });
});
