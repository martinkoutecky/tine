import { editorIn, type EditorDriver } from "../tests/editorDriver";
// GH #939: after Enter a new block sometimes shows no caret. The diagnostic
// record `editor_focus_missed` must tell the ranked causes apart (window
// without focus, a held edit guard, a focus thief after mount), stay silent on
// a normal Enter, and be rate-limited. Content-free fields only.
import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from "vitest";
import { For } from "solid-js";
import { render } from "solid-js/web";

const recorded = vi.hoisted(() => [] as Array<{ kind: string; fields?: unknown }>);
vi.mock("../debug", async (importOriginal) => ({
  ...(await importOriginal<typeof import("../debug")>()),
  recordDiagnostic: (kind: string, fields?: unknown) => {
    recorded.push({ kind, fields });
    return Promise.resolve();
  },
}));

import { EDITOR_FOCUS_MISS_INTERVAL_MS, endEdit, registerEditEntryGuard, registerEditExitGuard, registerHistoryEditorTarget, startEditing } from "../editorController";
import { initParser } from "../render/parse";
import { pageByName, resetStore } from "../document";
import { loadSingle } from "../document/workingSet";
import type { BlockDto, PageDto } from "../types";
import { nextFrame } from "../windowRealm";
import { Block } from "./Block";

beforeAll(() => initParser());

const realNow = performance.now.bind(performance);
let skew = 0;
beforeEach(() => {
  // Each test starts outside the previous test's rate-limit window.
  skew += EDITOR_FOCUS_MISS_INTERVAL_MS + 1;
  vi.spyOn(performance, "now").mockImplementation(() => realNow() + skew);
  recorded.length = 0;
});

afterEach(() => {
  vi.restoreAllMocks();
  endEdit("page-navigation");
  resetStore();
  document.body.innerHTML = "";
});

const misses = () => recorded.filter((entry) => entry.kind === "editor_focus_missed");

function mountEditor() {
  const block: BlockDto = { id: "focus-939", raw: "alpha", collapsed: false, children: [] };
  const page: PageDto = { name: "Focus 939", kind: "page", title: "Focus 939", pre_block: null, blocks: [block] };
  loadSingle(page);
  startEditing("focus-939", 5);
  const root = document.createElement("div");
  document.body.append(root);
  const dispose = render(() => (
    <For each={pageByName("Focus 939")?.roots ?? []}>{(id) => <Block id={id} />}</For>
  ), root);
  const editor = editorIn(root);
  editor.focus();
  editor.select(5, 5);
  return { root, editor, dispose };
}

/** The tag the record names for a focused block editor: a textarea, or
 *  CodeMirror's contenteditable `div` (activeIsBlockEditor tells it apart). */
function blockEditorTag(editor: EditorDriver): string {
  expect(editor.element.localName).toBe(editor.kind === "cm6" ? "div" : "textarea");
  return editor.element.localName;
}

async function pressEnter(editor: EditorDriver) {
  editor.keydown({ key: "Enter", code: "Enter", bubbles: true, cancelable: true });
}

async function settle() {
  await nextFrame();
  await nextFrame();
}

describe("editor_focus_missed (GH #939)", () => {
  it("records nothing for a normal Enter whose new editor takes focus", async () => {
    const { root, editor, dispose } = mountEditor();
    try {
      await settle();
      recorded.length = 0;
      await pressEnter(editor);
      await settle();
      expect(pageByName("Focus 939")!.roots).toHaveLength(2);
      expect(editorIn(root).value()).toBe("");
      expect(misses()).toEqual([]);
    } finally {
      dispose();
    }
  });

  it("tells a window without OS focus apart: the editor is active but the document has no focus", async () => {
    const { editor, dispose } = mountEditor();
    try {
      await settle();
      recorded.length = 0;
      vi.spyOn(document, "hasFocus").mockReturnValue(false);
      await pressEnter(editor);
      await settle();
      expect(misses()).toEqual([{ kind: "editor_focus_missed", fields: { editorFocus: {
        entry: "enter-split", documentHasFocus: false, guard: "none", targetMounted: true,
        activeTag: blockEditorTag(editor), activeIsBlockEditor: true,
      } } }]);
    } finally {
      dispose();
    }
  });

  it("tells a held edit-exit guard apart: startEditing returned early and the new block never mounted an editor", async () => {
    const { editor, dispose } = mountEditor();
    const release = registerEditExitGuard(() => false);
    try {
      await settle();
      recorded.length = 0;
      await pressEnter(editor);
      await settle();
      expect(pageByName("Focus 939")!.roots).toHaveLength(2);
      expect(misses()).toEqual([{ kind: "editor_focus_missed", fields: { editorFocus: {
        entry: "enter-split", documentHasFocus: true, guard: "exit-guard", targetMounted: false,
        activeTag: blockEditorTag(editor), activeIsBlockEditor: true,
      } } }]);
    } finally {
      release();
      dispose();
    }
  });

  it("names a held edit-entry guard (a sheet cell draft) separately", async () => {
    const { editor, dispose } = mountEditor();
    const release = registerEditEntryGuard(() => false);
    try {
      await settle();
      recorded.length = 0;
      await pressEnter(editor);
      await settle();
      expect(misses()).toHaveLength(1);
      expect(misses()[0].fields).toMatchObject({ editorFocus: { guard: "entry-guard", targetMounted: false } });
    } finally {
      release();
      dispose();
    }
  });

  it("tells a focus thief after mount apart: focus sits on a non-editor element", async () => {
    const { editor, dispose } = mountEditor();
    const thief = document.createElement("input");
    document.body.append(thief);
    try {
      await settle();
      recorded.length = 0;
      await pressEnter(editor);
      thief.focus();
      await settle();
      const [miss] = misses();
      expect(misses()).toHaveLength(1);
      expect(miss.fields).toMatchObject({ editorFocus: {
        entry: "enter-split", documentHasFocus: true, guard: "none",
        activeTag: "input", activeIsBlockEditor: false,
      } });
    } finally {
      dispose();
    }
  });

  it("records a non-Enter request as entry 'other', with no ids or text", async () => {
    const { dispose } = mountEditor();
    try {
      await settle();
      recorded.length = 0;
      startEditing("not-rendered-939", 0);
      await settle();
      expect(misses()).toHaveLength(1);
      const fields = JSON.stringify(misses()[0].fields);
      expect(fields).toContain("\"entry\":\"other\"");
      expect(fields).toContain("\"targetMounted\":false");
      expect(fields).not.toContain("939");
      expect(fields).not.toContain("alpha");
    } finally {
      dispose();
    }
  });

  it("writes at most one record per interval, and again after it", async () => {
    const { editor, dispose } = mountEditor();
    try {
      await settle();
      recorded.length = 0;
      vi.spyOn(document, "hasFocus").mockReturnValue(false);
      await pressEnter(editor);
      await settle();
      await pressEnter(editorIn(document.body));
      await settle();
      expect(pageByName("Focus 939")!.roots).toHaveLength(3);
      expect(misses()).toHaveLength(1);
      skew += EDITOR_FOCUS_MISS_INTERVAL_MS;
      await pressEnter(editorIn(document.body));
      await settle();
      expect(misses()).toHaveLength(2);
    } finally {
      dispose();
    }
  });

  it("does not scan the mounted editors when the requested editor holds focus", async () => {
    const { editor, dispose } = mountEditor();
    let reads = 0;
    const unregister = registerHistoryEditorTarget({
      get blockId() { reads++; return "other-939"; },
      owner: null,
      surface: "probe",
      selection: () => ({ start: 0, end: 0 }),
    });
    try {
      await settle();
      await pressEnter(editor);
      // The split itself captures history context synchronously; only the
      // frame-later focus check is counted.
      reads = 0;
      await settle();
      expect(misses()).toEqual([]);
      expect(reads).toBe(0);
    } finally {
      unregister();
      dispose();
    }
  });

  it("counts a focused CodeMirror editor as a block editor when the window has no focus", async () => {
    const cm = document.createElement("div");
    cm.setAttribute("data-cm6-editor", "cm-939");
    cm.tabIndex = 0;
    document.body.append(cm);
    cm.focus();
    vi.spyOn(document, "hasFocus").mockReturnValue(false);
    startEditing("cm-939", 0);
    await settle();
    expect(misses()).toHaveLength(1);
    expect(misses()[0].fields).toMatchObject({ editorFocus: {
      documentHasFocus: false, activeTag: "div", activeIsBlockEditor: true,
    } });
  });
});
