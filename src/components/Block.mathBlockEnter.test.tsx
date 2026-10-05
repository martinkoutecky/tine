import { describe, expect, it } from "vitest";
import { For } from "solid-js";
import { pageByName, undo } from "../document";
import { loadSingle } from "../document/workingSet";
import { doc } from "../document/model";
import { startEditing } from "../editorController";
import { Block } from "./Block";
import { installBlockEditorLifecycle, mount, blk, page, pressEnter } from "../tests/blockEditorTestkit";

installBlockEditorLifecycle();

function editing(name: string, raw: string) {
  loadSingle(page(name, [blk(`${name}-1`, raw)]));
  const id = pageByName(name)!.roots[0];
  startEditing(id, 0);
  const { root, dispose } = mount(() => (
    <For each={pageByName(name)?.roots ?? []}>{(bid) => <Block id={bid} />}</For>
  ));
  const ta = root.querySelector("textarea") as HTMLTextAreaElement;
  return { id, ta, dispose };
}

// GH #278: Enter inside a multi-line `$$ … $$` environment must insert a newline
// and stay in the block, exactly as it does inside a code fence.
//
// This is a DELIBERATE DIVERGENCE FROM OG, approved by Martin. OG's Enter dwim
// (frontend/util/thingatpt.cljs `admonition&src-at-point`) knows only ``` and
// `#+BEGIN_`, so OG splits the bullet and breaks the environment too.
describe("Enter inside a $$ display-math environment", () => {
  it("inserts a newline and does NOT split the block", () => {
    const { id, ta, dispose } = editing("Math", "$$\ntest\n$$");
    try {
      pressEnter(ta, ta.value.indexOf("test") + "test".length);
      expect(pageByName("Math")!.roots).toEqual([id]);
      expect(doc.byId[id].raw).toBe("$$\ntest\n\n$$");
    } finally {
      dispose();
    }
  });

  it("continues a freshly typed opening $$ in the same block", () => {
    const { id, ta, dispose } = editing("Math open", "$$");
    try {
      pressEnter(ta, ta.value.length);
      expect(pageByName("Math open")!.roots).toEqual([id]);
      expect(doc.byId[id].raw).toBe("$$\n");
    } finally {
      dispose();
    }
  });

  it("splits normally after a CLOSED inline $$…$$", () => {
    // Necessity guard: finishing inline math and pressing Enter is the common
    // gesture for "next bullet". The environment is closed, so it must split.
    const { ta, dispose } = editing("Inline math", "energy is $$E = mc^2$$");
    try {
      pressEnter(ta, ta.value.length);
      expect(pageByName("Inline math")!.roots).toHaveLength(2);
    } finally {
      dispose();
    }
  });

  it("splits normally when the caret is before the opening $$", () => {
    const { ta, dispose } = editing("Before math", "intro $$\nx\n$$");
    try {
      pressEnter(ta, "intro".length);
      expect(pageByName("Before math")!.roots).toHaveLength(2);
    } finally {
      dispose();
    }
  });

  it("treats $$ inside a code fence as literal text, not a math delimiter", () => {
    // The fence owns the region; a `$$` in a shell snippet opens nothing, so the
    // ordinary fence rule (stay in the block) is what applies here.
    const { id, ta, dispose } = editing("Fenced dollars", "```sh\necho $$\n```");
    try {
      pressEnter(ta, ta.value.indexOf("echo $$") + "echo $$".length);
      expect(pageByName("Fenced dollars")!.roots).toEqual([id]);
      expect(doc.byId[id].raw).toBe("```sh\necho $$\n\n```");
    } finally {
      dispose();
    }
  });
});

describe("Double Enter exits a trailing $$ environment", () => {
  it("removes the sentinel blank line, and undo restores the pre-exit state", () => {
    const original = "$$\ntest\n\n$$";
    const { id, ta, dispose } = editing("Math exit", original);
    try {
      pressEnter(ta, ta.value.indexOf("\n\n") + 1);
      expect(pageByName("Math exit")!.roots).toHaveLength(2);
      expect(doc.byId[id].raw).toBe("$$\ntest\n$$");
      expect(doc.byId[pageByName("Math exit")!.roots[1]].raw).toBe("");

      undo();
      expect(pageByName("Math exit")!.roots).toEqual([id]);
      expect(doc.byId[id].raw).toBe(original);
    } finally {
      dispose();
    }
  });
});
