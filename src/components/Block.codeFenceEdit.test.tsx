import { afterEach, beforeAll, describe, expect, it } from "vitest";
import { For, type JSX } from "solid-js";
import { render } from "solid-js/web";
import { initParser } from "../render/parse";
import { pageByName, resetStore, setRaw } from "../document";
import { loadSingle } from "../document/workingSet";
import { startEditing } from "../editorController";
import type { BlockDto, PageDto } from "../types";
import { Block } from "./Block";
import { Editor } from "./Block";

// GH #357: rendered fenced code blocks are a mono, no-wrap, padded card
// (.code-block). The editor used to be the ordinary proportional wrapped
// textarea, so clicking a code block changed every line's layout. While the
// block IS code-shaped, the editor now presents as the same code card; mixed
// blocks keep ordinary editing presentation.
//
// GH #412/#413 follow-up (the Martin-authorized body-only code editor
// contract): a COMPLETE whole-block wrapper's editor shows only the payload
// between the wrapper lines and preserves the wrapper bytes on commit. The
// earlier "raw text, fences editable" assertion below is intentionally
// replaced by that contract; the card presentation (mono, no-wrap) is not.

beforeAll(async () => {
  await initParser();
});

afterEach(() => {
  resetStore();
  document.body.innerHTML = "";
});

function mount(node: () => JSX.Element): { root: HTMLDivElement; dispose: () => void } {
  const root = document.createElement("div");
  document.body.appendChild(root);
  const dispose = render(node, root);
  return { root, dispose };
}

function blk(id: string, raw: string): BlockDto {
  return { id, raw, collapsed: false, children: [] };
}

function page(name: string, blocks: BlockDto[]): PageDto {
  return { name, kind: "page", title: name, pre_block: null, blocks };
}

describe("code-fence editor presentation", () => {
  it("presents the editor of a code-only fenced block as the same code card", () => {
    loadSingle(page("Code", [blk("c1", "```js\nconst x = 1;\nconsole.log(x);\n```")]));
    const id = pageByName("Code")!.roots[0];
    startEditing(id, 0);
    const { root, dispose } = mount(() => (
      <For each={pageByName("Code")?.roots ?? []}>{(bid) => <Block id={bid} />}</For>
    ));
    try {
      const ta = root.querySelector("textarea")!;
      expect(ta.classList.contains("code-edit")).toBe(true);
      // Default is horizontal scrolling, shared with the rendered code card.
      expect(ta.getAttribute("wrap")).toBe("off");
      // Body-only code view (GH #412/#413): the payload, without the fences;
      // the wrapper bytes are preserved on commit (see codeBodyEdit tests).
      expect(ta.value).toBe("const x = 1;\nconsole.log(x);");
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
      const ta = root.querySelector("textarea")!;
      expect(ta.classList.contains("code-edit")).toBe(false);
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
      const ta = root.querySelector("textarea")!;
      expect(ta.classList.contains("code-edit")).toBe(false);
      // Calc strips the fence in the editor and shows the live results panel.
      expect(ta.value).toBe("1+1");
      expect(root.querySelector(".calc-results")).not.toBeNull();
    } finally {
      dispose();
    }
  });

  it("org #+BEGIN_SRC blocks get the same code-card editing", () => {
    loadSingle({ ...page("OrgCode", [blk("c4", "#+BEGIN_SRC python\nx = 1\n#+END_SRC")]), format: "org" } as PageDto);
    const id = pageByName("OrgCode")!.roots[0];
    startEditing(id, 0);
    const { root, dispose } = mount(() => (
      <For each={pageByName("OrgCode")?.roots ?? []}>{(bid) => <Block id={bid} />}</For>
    ));
    try {
      const ta = root.querySelector("textarea")!;
      expect(ta.classList.contains("code-edit")).toBe(true);
      expect(ta.getAttribute("wrap")).toBe("off");
      expect(root.querySelector(".code-language")?.textContent).toBe("python");
      expect(root.querySelector(".calc-lineno")?.textContent).toBe("1");
    } finally {
      dispose();
    }
  });

  it("live transitions update the presentation when the block's code shape changes", async () => {
    const fenced = "```js\nconst x = 1;\n```";
    loadSingle(page("Live", [blk("c5", fenced)]));
    const id = pageByName("Live")!.roots[0];
    startEditing(id, 0);
    const { root, dispose } = mount(() => (
      <For each={pageByName("Live")?.roots ?? []}>{(bid) => <Editor id={bid} />}</For>
    ));
    try {
      const ta = root.querySelector("textarea")!;
      expect(ta.classList.contains("code-edit")).toBe(true);
      expect(ta.value).toBe("const x = 1;");
      // The code-only shape breaks (text after the fence): the class drops
      // live and the editor returns to the honest raw text.
      setRaw(id, "```js\nconst x = 1;\n```\nplain note");
      await Promise.resolve();
      expect(ta.classList.contains("code-edit")).toBe(false);
      expect(ta.value).toBe("```js\nconst x = 1;\n```\nplain note");
      // …and back: the body-only code view flips in again.
      setRaw(id, fenced);
      await Promise.resolve();
      expect(ta.classList.contains("code-edit")).toBe(true);
      expect(ta.value).toBe("const x = 1;");
    } finally {
      dispose();
    }
  });
});
