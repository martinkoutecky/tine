import { afterAll, afterEach, beforeAll, describe, expect, it } from "vitest";
import { For, type JSX } from "solid-js";
import { render } from "solid-js/web";
import { startEditing, editingId, endEdit } from "../editorController";
import { installKeybindings } from "../keybindings";
import { initParser } from "../render/parse";
import { hasSelection, pageByName, resetStore, selectedIds } from "../document";
import { loadSingle } from "../document/workingSet";
import { doc } from "../document/model";
import type { BlockDto, PageDto } from "../types";
import { Block } from "./Block";

// GH #412/#413 body-only code editor contract: while the whole visible block
// is ONE complete code wrapper (markdown fence or org #+BEGIN_SRC/EXAMPLE,
// ```calc excluded, mixed/incomplete excluded), the block editor shows ONLY
// the payload between the wrapper lines, and commits re-attach the exact raw
// wrapper bytes. Consequences pinned here:
//   - fences stay out of the editing surface (GH #413's second complaint);
//   - Ctrl/Cmd+A's native first press selects exactly the payload (GH #412);
//   - the GH #262 second-press block-selection escalation still applies.
// This replaces the GH #357 "raw text, fences editable" assertion (only that;
// the code-card presentation itself is unchanged).

let disposeKeys: (() => void) | null = null;
beforeAll(async () => {
  await initParser();
  disposeKeys = installKeybindings();
});

afterAll(() => {
  disposeKeys?.();
});

afterEach(() => {
  endEdit("page-navigation");
  resetStore();
  document.body.innerHTML = "";
});

function mount(node: () => JSX.Element): { root: HTMLDivElement; dispose: () => void } {
  const root = document.createElement("div");
  document.body.appendChild(root);
  return { root, dispose: render(node, root) };
}

let seq = 0;
function mountPage(raw: string, format: "md" | "org" = "md") {
  const name = `Code${++seq}`;
  const block: BlockDto = { id: `code-${seq}`, raw, collapsed: false, children: [] };
  const page: PageDto = { name, kind: "page", title: name, pre_block: null, blocks: [block], format };
  loadSingle(page);
  startEditing(block.id, 0);
  const mounted = mount(() => (
    <For each={pageByName(name)?.roots ?? []}>{(id) => <Block id={id} />}</For>
  ));
  const textarea = mounted.root.querySelector("textarea.block-editor") as HTMLTextAreaElement;
  return { ...mounted, textarea, blockId: block.id };
}

function typeText(ta: HTMLTextAreaElement, insert: string) {
  const caret = ta.selectionStart;
  const nv = ta.value.slice(0, caret) + insert + ta.value.slice(ta.selectionEnd);
  ta.value = nv;
  ta.setSelectionRange(caret + insert.length, caret + insert.length);
  const last = insert.at(-1) ?? "";
  ta.dispatchEvent(new InputEvent("input", {
    bubbles: true,
    inputType: "insertText",
    data: last || null,
  }));
}

const modA = (el: HTMLTextAreaElement) =>
  el.dispatchEvent(new KeyboardEvent("keydown", { key: "a", ctrlKey: true, bubbles: true, cancelable: true }));

describe("body-only code editor (GH #412/#413)", () => {
  it("shows only the payload of a complete markdown fence, never the fences", () => {
    const { textarea, dispose } = mountPage("```\necho hello\n```");
    try {
      expect(textarea.value).toBe("echo hello");
      expect(textarea.value).not.toContain("```");
      // The code-card presentation itself is unchanged (GH #357 contract).
      expect(textarea.classList.contains("code-edit")).toBe(true);
      expect(textarea.getAttribute("wrap")).toBe("off");
    } finally {
      dispose();
    }
  });

  it("commits through the body and preserves the exact raw wrapper bytes", () => {
    const original = "````sh\necho hi\n````";
    const { textarea, blockId, dispose } = mountPage(original);
    try {
      textarea.setSelectionRange(7, 7);
      typeText(textarea, ", world");
      expect(doc.byId[blockId].raw).toBe("````sh\necho hi, world\n````");
    } finally {
      dispose();
    }
    // A focus cycle without edits is byte-neutral (guard against normalization churn).
    const again = mountPage("~~~\nx\n~~~");
    try {
      endEdit("page-navigation");
      expect(doc.byId[again.blockId].raw).toBe("~~~\nx\n~~~");
    } finally {
      again.dispose();
    }
  });

  it("shows only the body of an org #+BEGIN_SRC block and keeps org wrapper bytes", () => {
    const { textarea, blockId, dispose } = mountPage("#+BEGIN_SRC python\nx = 1\n#+END_SRC", "org");
    try {
      expect(textarea.value).toBe("x = 1");
      textarea.setSelectionRange(0, 0);
      typeText(textarea, "# ");
      expect(doc.byId[blockId].raw).toBe("#+BEGIN_SRC python\n# x = 1\n#+END_SRC");
    } finally {
      dispose();
    }
  });

  it("keeps raw editing for mixed and incomplete wrappers and the calc mode", () => {
    const mixed = mountPage("intro\n```js\nx\n```");
    try {
      expect(mixed.textarea.value).toContain("```js");
      expect(mixed.textarea.classList.contains("code-edit")).toBe(false);
    } finally {
      mixed.dispose();
    }
    const typing = mountPage("```python\nx = 1");
    try {
      // An unclosed fence is still being authored: raw editing, code-card metrics.
      expect(typing.textarea.value).toContain("```python");
      expect(typing.textarea.classList.contains("code-edit")).toBe(true);
    } finally {
      typing.dispose();
    }
    const calc = mountPage("```calc\n1+1\n```");
    try {
      expect(calc.textarea.value).toBe("1+1");
      expect(calc.textarea.classList.contains("code-edit")).toBe(false);
    } finally {
      calc.dispose();
    }
  });

  it("first Mod+A stays the native payload-only selection; the second press escalates to the block (GH #412)", () => {    const child: BlockDto = { id: "code-kid", raw: "child", collapsed: false, children: [] };
    const holder: BlockDto = { id: "code-holder", raw: "```\necho hello\n```", collapsed: false, children: [child] };
    const name = `CodeSel${seq}`;
    loadSingle({ name, kind: "page", title: name, pre_block: null, blocks: [holder] });
    startEditing("code-holder", 0);
    const m = mount(() => <For each={pageByName(name)?.roots ?? []}>{(id) => <Block id={id} />}</For>);
    try {
      const ta = m.root.querySelector("textarea.block-editor") as HTMLTextAreaElement;
      expect(ta.value).toBe("echo hello");

      // Press 1: falls through to the native select-all — which CANNOT reach
      // the fences because they are not part of the edited text at all.
      modA(ta);
      expect(hasSelection()).toBe(false);
      expect(editingId()).toBe("code-holder");
      // The browser applies select-all: exactly the payload is selected.
      ta.setSelectionRange(0, ta.value.length);
      expect(ta.value.slice(ta.selectionStart, ta.selectionEnd)).toBe("echo hello");
      expect(ta.value.slice(ta.selectionStart, ta.selectionEnd)).not.toContain("```");

      // Press 2: the existing GH #262 escalation selects the block subtree.
      modA(ta);
      expect(hasSelection()).toBe(true);
      expect(editingId()).toBeNull();
      expect(selectedIds()).toEqual(["code-holder", "code-kid"]);
    } finally {
      m.dispose();
    }
  });

  it("a multiline paste inside the code view stays literal text inside the wrapper (no outline splintering)", () => {
    const { textarea, blockId, dispose } = mountPage("```py\nx = 1\n```");
    try {
      textarea.setSelectionRange(5, 5);
      const event = new Event("paste", { bubbles: true, cancelable: true });
      Object.defineProperty(event, "clipboardData", {
        value: {
          files: [],
          items: [],
          types: ["text/plain"],
          getData: () => "y = 2\nz = 3",
        },
      });
      textarea.dispatchEvent(event);
      // One block, literal text, exact wrapper preserved.
      expect(pageByName(`Code${seq}`)!.roots).toEqual([blockId]);
      expect(doc.byId[blockId].raw).toBe("```py\nx = 1y = 2\nz = 3\n```");
    } finally {
      dispose();
    }
  });
});
