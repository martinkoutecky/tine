import { describe, expect, it } from "vitest";
import { For } from "solid-js";
import { pageByName, undo } from "../document";
import { loadSingle } from "../document/workingSet";
import { doc } from "../document/model";
import { startEditing } from "../editorController";
import { Block } from "./Block";
import { installBlockEditorLifecycle, mount, blk, page, pressEnter } from "../tests/blockEditorTestkit";

installBlockEditorLifecycle();

// GH #66: Enter INSIDE a fenced code block must insert a newline and stay in the
// same block, not split off a new bullet (which breaks the fence).
describe("Enter inside a code fence", () => {
  it("inserts a newline and does NOT split the block", () => {
    loadSingle(page("Code", [blk("code-1", "```js\nconst x = 1\n```")]));
    const id = pageByName("Code")!.roots[0];

    startEditing(id, 0);
    const { root, dispose } = mount(() => (
      <For each={pageByName("Code")?.roots ?? []}>{(bid) => <Block id={bid} />}</For>
    ));

    try {
      const ta = root.querySelector("textarea") as HTMLTextAreaElement | null;
      expect(ta).not.toBeNull();
      // Caret at the end of the `const x = 1` content line (inside the fence).
      const caret = ta!.value.indexOf("const x = 1") + "const x = 1".length;
      pressEnter(ta!, caret);

      // The page still has exactly one root block — Enter did NOT split.
      expect(pageByName("Code")!.roots.length).toBe(1);
      // A newline was inserted inside the fence content.
      const raw = doc.byId[id]?.raw ?? ta!.value;
      expect(raw).toContain("const x = 1\n\n```");
    } finally {
      dispose();
    }
  });

  it("stays inside a four-backtick fence closed by a four-backtick run", () => {
    loadSingle(page("Code", [blk("code-4", "````js\nconst x = 1\n````") ]));
    const id = pageByName("Code")!.roots[0];
    startEditing(id, 0);
    const { root, dispose } = mount(() => (
      <For each={pageByName("Code")?.roots ?? []}>{(bid) => <Block id={bid} />}</For>
    ));
    try {
      const ta = root.querySelector("textarea") as HTMLTextAreaElement;
      const caret = ta.value.indexOf("const x = 1") + "const x = 1".length;
      pressEnter(ta, caret);
      expect(pageByName("Code")!.roots).toEqual([id]);
      expect(doc.byId[id].raw).toContain("const x = 1\n\n````");
    } finally {
      dispose();
    }
  });

  it("leaves a four-backtick fence at the parser's shorter closing run (OG/mldoc, not CommonMark)", () => {
    // mldoc closes the container at the inner three-backtick run, so the line after it is
    // ordinary text and Enter there splits the block (Martin 2026-10-01).
    loadSingle(page("Code", [blk("code-4", "````js\n```\nconst x = 1\n````") ]));
    const id = pageByName("Code")!.roots[0];
    startEditing(id, 0);
    const { root, dispose } = mount(() => (
      <For each={pageByName("Code")?.roots ?? []}>{(bid) => <Block id={bid} />}</For>
    ));
    try {
      const ta = root.querySelector("textarea") as HTMLTextAreaElement;
      const caret = ta.value.indexOf("const x = 1") + "const x = 1".length;
      pressEnter(ta, caret);
      expect(pageByName("Code")!.roots.length).toBe(2);
    } finally {
      dispose();
    }
  });

  it("continues a freshly typed opening fence in the same block", () => {
    loadSingle(page("Code", [blk("opening", "```js") ]));
    const id = pageByName("Code")!.roots[0];
    startEditing(id, 0);
    const { root, dispose } = mount(() => (
      <For each={pageByName("Code")?.roots ?? []}>{(bid) => <Block id={bid} />}</For>
    ));
    try {
      const ta = root.querySelector("textarea") as HTMLTextAreaElement;
      pressEnter(ta, ta.value.length);
      expect(pageByName("Code")!.roots).toEqual([id]);
      expect(doc.byId[id].raw).toBe("```js\n");
    } finally {
      dispose();
    }
  });

  it("still splits into a new block when Enter is pressed OUTSIDE any fence", () => {
    // Necessity guard: the fence special-case must not swallow ordinary Enter.
    loadSingle(page("Plain", [blk("plain-1", "hello world")]));
    const id = pageByName("Plain")!.roots[0];

    startEditing(id, 0);
    const { root, dispose } = mount(() => (
      <For each={pageByName("Plain")?.roots ?? []}>{(bid) => <Block id={bid} />}</For>
    ));

    try {
      const ta = root.querySelector("textarea") as HTMLTextAreaElement | null;
      expect(ta).not.toBeNull();
      pressEnter(ta!, ta!.value.length);
      // Ordinary text splits into a second block.
      expect(pageByName("Plain")!.roots.length).toBe(2);
    } finally {
      dispose();
    }
  });
});

// GH #93: the second Enter on a trailing blank line is an escape gesture. It
// removes that sentinel blank line and creates a normal sibling in one undo unit.
describe("Double Enter exits a trailing code or calculator block", () => {
  it("exits a fenced block and undo restores the exact pre-exit state", () => {
    const original = "```js\nconst x = 1\n\n```";
    loadSingle(page("Code exit", [blk("code-exit", original)]));
    const id = pageByName("Code exit")!.roots[0];
    startEditing(id, 0);
    const { root, dispose } = mount(() => (
      <For each={pageByName("Code exit")?.roots ?? []}>{(bid) => <Block id={bid} />}</For>
    ));
    try {
      const ta = root.querySelector("textarea") as HTMLTextAreaElement;
      // The closer's mandatory separator is wrapper-owned, so the one explicit
      // trailing payload newline places the caret on the blank exit row.
      const blankLine = ta.value.length;
      pressEnter(ta, blankLine);
      expect(pageByName("Code exit")!.roots).toHaveLength(2);
      expect(doc.byId[id].raw).toBe("```js\nconst x = 1\n```");
      expect(doc.byId[pageByName("Code exit")!.roots[1]].raw).toBe("");

      undo();
      expect(pageByName("Code exit")!.roots).toEqual([id]);
      expect(doc.byId[id].raw).toBe(original);
    } finally {
      dispose();
    }
  });

  it("exits a calculator block and preserves its canonical fence", () => {
    loadSingle(page("Calc exit", [blk("calc-exit", "```calc\n1 + 1\n\n```")]));
    const id = pageByName("Calc exit")!.roots[0];
    startEditing(id, 0);
    const { root, dispose } = mount(() => (
      <For each={pageByName("Calc exit")?.roots ?? []}>{(bid) => <Block id={bid} />}</For>
    ));
    try {
      const ta = root.querySelector("textarea") as HTMLTextAreaElement;
      expect(ta.value).toBe("1 + 1\n");
      pressEnter(ta, ta.value.length);
      expect(pageByName("Calc exit")!.roots).toHaveLength(2);
      expect(doc.byId[id].raw).toBe("```calc\n1 + 1\n```");
    } finally {
      dispose();
    }
  });

  it("does not reinterpret an ordinary trailing blank line as a special-block exit", () => {
    loadSingle(page("Plain blank", [blk("plain-blank", "hello\n")]));
    const id = pageByName("Plain blank")!.roots[0];
    startEditing(id, 0);
    const { root, dispose } = mount(() => (
      <For each={pageByName("Plain blank")?.roots ?? []}>{(bid) => <Block id={bid} />}</For>
    ));
    try {
      const ta = root.querySelector("textarea") as HTMLTextAreaElement;
      pressEnter(ta, ta.value.length);
      expect(pageByName("Plain blank")!.roots).toHaveLength(2);
      // Still no special-block exit (a new ordinary block follows). The first
      // block drops the boundary newline itself: GH #361's unified rule —
      // Enter immediately after an in-block newline consumes that break as the
      // structural separator instead of leaving it as an empty line.
      expect(doc.byId[id].raw).toBe("hello");
    } finally {
      dispose();
    }
  });
});
