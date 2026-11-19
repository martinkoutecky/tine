// og-enter-family-1109: the Enter family of gestures that edit the block's text and change the outline
// in one keystroke. Each one either commits all of its edit, as one undo unit, or none of it: on a
// refusal or a parser failure the editor text and selection, the block, the outline, the undo history,
// the dirty bit and the editing block stay as they were, and exactly one error toast says so.
//
// - Enter over a selection deletes the selection, then splits at its start (OG 6e7afa8eb
//   src/main/frontend/handler/editor.cljs `compute-fst-snd-block-text`, called by `insert-new-block!`
//   from `keydown-new-block`). The textarea used to split at the selection's start and keep the
//   selected text in the new block (HONSw).
// - Completing a template from the slash menu removes the trigger text and inserts the template's
//   outline (HONP4). The removal used to be written before the insert was asked, so a failed insert
//   kept it.
// - Pasting multi-line text over a selection inserts the pasted blocks after the block and keeps its
//   text (OG `paste-blocks`, editor.cljs:2013). A failed insert must change nothing (HONP4).
//
// Both editors share these paths, so one table drives both; `useEditorKind` runs each half on its engine.
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { For } from "solid-js";
import { render } from "solid-js/web";

const wasm = vi.hoisted(() => ({ failNextApply: false, calls: {} as Record<string, number> }));
const writes = vi.hoisted(() => ({ document: 0 }));
// Every store write a gesture makes, counted at the one setter all document edits go through.
vi.mock("../document/model", async (importOriginal) => {
  const real = await importOriginal<typeof import("../document/model")>();
  return { ...real, setDoc: Object.assign((...args: unknown[]) => { writes.document++; return (real.setDoc as (...a: unknown[]) => unknown)(...args); }, real.setDoc) };
});
vi.mock("../render/wasm/lsdoc_wasm.js", async (importOriginal) => {
  const real = await importOriginal<typeof import("../render/wasm/lsdoc_wasm.js")>();
  // Every parser call by door, to pin what a gesture asks the parser.
  const counted: Record<string, unknown> = {};
  for (const [name, value] of Object.entries(real)) {
    counted[name] = typeof value === "function" && !name.startsWith("__") && name !== "default" && name !== "initSync"
      ? (...args: unknown[]) => { wasm.calls[name] = (wasm.calls[name] ?? 0) + 1; return (value as (...a: unknown[]) => unknown)(...args); }
      : value;
  }
  return {
    ...real,
    ...counted,
    outline_apply_json: (request: string) => {
      wasm.calls.outline_apply_json = (wasm.calls.outline_apply_json ?? 0) + 1;
      if (wasm.failNextApply) {
        wasm.failNextApply = false;
        // What the glue's guard throws after it reinstantiated a trapped instance.
        throw new Error("lsdoc-wasm trap in outline_apply_json: Out of bounds memory access [instance: 1 calls, 0 resets, 25 pages; args string:400]");
      }
      return real.outline_apply_json(request);
    },
  };
});

import { Block } from "./Block";
import { backend } from "../backend";
import { editingId, endEdit, startEditing } from "../editorController";
import { isDirty, pageByName, redo, resetStore, setRaw, undo, undoTopTag } from "../document";
import { loadSingle } from "../document/workingSet";
import { bindTestHost } from "../document/host/wiring.test.support";
import { openAsLoaded } from "./hostConflict.test.support";
import { doc } from "../document/model";
import { initParser } from "../render/parse";
import { setToasts, toasts } from "../toasts";
import { bumpGraphEpoch, setGraphMeta } from "../graphSession";
import type { GraphMeta } from "../types";
import { editorIn, type EditorDriver, type EditorKind } from "../tests/editorDriver";
import { useEditorKind } from "../tests/editorKind";

const META: GraphMeta = {
  root: "/tmp/enter-family-graph", journals_dir: "journals", pages_dir: "pages", preferred_workflow: "now",
  shortcuts: {}, start_of_week: 6, block_hidden_properties: [], linked_references_collapsed_threshold: 100, ref_default_open_blocks_level: 2, default_journal_template: null,
  favorites: [], journal_page_title_format: "MMM do, yyyy", journal_file_name_format: "yyyy_MM_dd",
  preferred_format: "md", macros: {}, enable_tooltip: true, file_name_format: "legacy", doc_mode_enter_for_new_block: false, logical_outdenting: false, enable_search_remove_accents: true, quick_capture_template_text: null, quick_capture_template_media: null, quick_capture_default_page: null, quick_capture_insert_today: false, quick_capture_redirect_page: false, enable_timetracking: true, show_brackets: true, logbook_with_second_support: true,
  logbook_enabled_in_timestamped_blocks: false, logbook_enabled_in_all_blocks: false, guide_announced: true, mobile_gestures_disabled_in_block_with_tags: [],
};



describe.each<EditorKind>(["textarea", "cm6"])("%s editor: the Enter family commits all of a gesture or none of it", (kind) => {
  useEditorKind(kind);
  let dispose: (() => void) | undefined;
  let root: HTMLElement;
  beforeEach(async () => { await initParser(); setToasts([]); });
  afterEach(() => {
    wasm.failNextApply = false;
    vi.restoreAllMocks();
    dispose?.(); dispose = undefined;
    setGraphMeta(null); bumpGraphEpoch();
    endEdit("page-navigation"); resetStore(); document.body.innerHTML = "";
  });

  /** The page `Third`: `first`, then `target` holding `raw`, edited with the caret at `caret`. */
  function mount(raw: string, caret = raw.length): EditorDriver {
    loadSingle({ name: "Third", title: "Third", kind: "page", pre_block: null, format: "md",
      blocks: [{ id: "first", raw: "first", collapsed: false, children: [] },
        { id: "target", raw, collapsed: false, children: [] }] });
    startEditing("target", caret);
    root = document.createElement("div"); document.body.append(root);
    dispose = render(() => <For each={pageByName("Third")?.roots ?? []}>{(id) => <Block id={id} />}</For>, root);
    const editor = editorIn(root, "target");
    editor.focus();
    return editor;
  }
  const quietly = <T,>(run: () => T): T => {
    const onError = (e: ErrorEvent) => e.preventDefault();
    window.addEventListener("error", onError);
    try { return run(); } finally { window.removeEventListener("error", onError); }
  };
  const enter = (editor: EditorDriver) => quietly(() => editor.keydown({ key: "Enter", code: "Enter", bubbles: true, cancelable: true }));
  async function acceptSuggestion(editor: EditorDriver, label: string) {
    const row = await vi.waitFor(() => {
      const labels = [...document.querySelectorAll(".autocomplete .ac-label")].map(item => item.textContent);
      expect(labels).toContain(label);
      return labels.indexOf(label);
    });
    for (let i = 0; i < row; i++) editor.keydown({ key: "ArrowDown", code: "ArrowDown", bubbles: true, cancelable: true });
    enter(editor);
  }
  const roots = () => pageByName("Third")!.roots;
  const texts = () => roots().map((id) => doc.byId[id].raw);
  const errors = () => toasts().filter((t) => t.kind === "error");
  const settle = () => new Promise<void>((resolve) => setTimeout(resolve, 0));
  // The kernel's doors and the clock: what a gesture asks of the parser beyond rendering.
  const kernelCalls = () => ({ reads: wasm.calls.outline_reads_json ?? 0, apply: wasm.calls.outline_apply_json ?? 0,
    clock: wasm.calls.logbook_apply_marker_transition ?? 0 });

  /** What a refusal must leave: the editor, the block, the outline, history and the toast. */
  function expectUnchanged(editor: EditorDriver, before: { value: string; selection: [number, number]; raw: string; slot: unknown; undo: string | null; dirty: boolean }) {
    expect(editor.value()).toBe(before.value);
    expect(editor.selection()).toEqual(before.selection);
    expect(doc.byId.target.raw).toBe(before.raw);
    expect(roots()).toEqual(["first", "target"]);
    expect(editingId()).toBe("target");
    // The same mounted editor, still the block's own: its node was not replaced and it still accepts edits.
    expect(doc.byId.target).toBe(before.slot);
    expect(editorIn(root, "target").element).toBe(editor.element);
    expect(editor.isReadOnly()).toBe(false);
    // A refusal is not a write: no store write, no undo entry, nothing newly dirty.
    expect(writes.document).toBe(0);
    expect(undoTopTag()).toBe(before.undo);
    expect(isDirty("Third")).toBe(before.dirty);
    // I-9: the failure is shown once, and "nothing was changed" is true.
    expect(errors().map((t) => t.message)).toHaveLength(1);
  }
  const snapshot = (editor: EditorDriver) => ({ value: editor.value(), selection: editor.selection(), raw: doc.byId.target.raw,
    slot: doc.byId.target, undo: undoTopTag(), dirty: isDirty("Third") });

  describe("Enter over a selection (HONSw)", () => {
    it("deletes the selection and splits at its start, as Logseq does, and one Undo restores the block", () => {
      const editor = mount("alpha beta gamma");
      editor.select(6, 11); // "beta "
      expect(enter(editor).defaultPrevented).toBe(true);
      // The selected text is gone: the head keeps what preceded it, the new block what followed it.
      expect(texts()).toEqual(["first", "alpha ", "gamma"]);
      expect(editingId()).toBe(roots()[2]);
      expect(undoTopTag()).toBe("split");
      expect(errors()).toHaveLength(0);
      // One gesture, one undo step: the selected text and the single block come back together.
      expect(undo()).toBe(true);
      expect(texts()).toEqual(["first", "alpha beta gamma"]);
    });

    it("a failed Enter over a selection changes nothing, and the retry splits", () => {
      const editor = mount("alpha beta gamma");
      editor.select(6, 11);
      const before = snapshot(editor);
      writes.document = 0;
      wasm.failNextApply = true;
      expect(enter(editor).defaultPrevented).toBe(true);
      expectUnchanged(editor, before);
      enter(editor);
      expect(texts()).toEqual(["first", "alpha ", "gamma"]);
    });

    it("a failed Enter over a selection keeps the redo history", () => {
      const editor = mount("alpha beta gamma");
      setRaw("first", "edited elsewhere");
      expect(undo()).toBe(true);
      editor.select(6, 11);
      wasm.failNextApply = true;
      enter(editor);
      redo();
      expect(doc.byId.first.raw).toBe("edited elsewhere");
    });
  });

  it("keeps later property values when accepting a page suggestion (#853)", async () => {
    const prefix = "project:: [[pr";
    setGraphMeta(META); bumpGraphEpoch();
    vi.spyOn(backend(), "quickSwitch").mockImplementation(async query => query === "pro"
      ? [{ name: "Programming", kind: "page", date_key: null, path: "pages/Programming.md" }]
      : []);
    const editor = mount(`${prefix}, Other, [[Existing]]`, prefix.length);
    editor.type("o");
    await vi.waitFor(() => expect([...document.querySelectorAll(".autocomplete .ac-label")]
      .map(row => row.textContent)).toContain("Programming"));
    enter(editor);
    expect(editor.value()).toMatch(/^project:: \[\[Programming\]\] ?, Other, \[\[Existing\]\]$/);
  });

  it.each(["; ((Keep ]] intact))", "\t{{embed [[Existing]]}}"])(
    "finishes an unfinished page reference before %j (FORK-853b)", async following => {
      const prefix = "project:: [[pr";
      setGraphMeta(META); bumpGraphEpoch();
      vi.spyOn(backend(), "quickSwitch").mockResolvedValue([
        { name: "Programming", kind: "page", date_key: null, path: "pages/Programming.md" },
      ]);
      const editor = mount(prefix + following, prefix.length);
      editor.type("o");
      await acceptSuggestion(editor, "Programming");
      const completed = "project:: [[Programming]]";
      expect(editor.value()).toBe(completed + " " + following);
      expect(editor.selection()).toEqual([completed.length + 1, completed.length + 1]);
      expect(roots()).toEqual(["first", "target"]);
    },
  );

  it("keeps the literal closer and following text when accepting a property suggestion (#853)", async () => {
    const prefix = "project:: `[[pro";
    const following = " text]]";
    const propertyValue = "`[[project`";
    setGraphMeta(META); bumpGraphEpoch();
    vi.spyOn(backend(), "queryFacets").mockResolvedValue([["project", [propertyValue]]]);
    vi.spyOn(backend(), "quickSwitch").mockResolvedValue([
      { name: "Result", kind: "page", date_key: null, path: "pages/Result.md" },
    ]);
    const editor = mount(prefix + following, prefix.length);
    editor.input(prefix + "`" + following, prefix.length + 1, { inputType: "insertFromPaste", data: null });
    expect(editor.value()).toBe(prefix + "`" + following);
    const shown = await vi.waitFor(() => {
      const choice = [...document.querySelectorAll(".autocomplete .ac-label")]
        .find(row => row.textContent === propertyValue || row.textContent === "Result");
      expect(choice).toBeDefined();
      return choice!.textContent!;
    });
    await acceptSuggestion(editor, shown);
    expect(editor.value()).toBe(`project:: ${propertyValue}${following}`);
  });

  it.each([
    ["[[", " [x] title"],
    ["[[", " #[[child]] title"],
    ["[[", " [ title"],
    ["[[", " \\]] title"],
    ["[[", " [[child]] title"],
    ["#[[", " [x] title"],
  ])("replaces the whole %s reference containing %s (#853)", async (opener, suffix) => {
      const prefix = `project:: ${opener}pr`;
      const following = ", [[Existing]]";
      setGraphMeta(META); bumpGraphEpoch();
      vi.spyOn(backend(), "quickSwitch").mockResolvedValue([
        { name: "Programming", kind: "page", date_key: null, path: "pages/Programming.md" },
      ]);
      const editor = mount(`${prefix}${suffix}]]${following}`, prefix.length);
      editor.type("o");
      await acceptSuggestion(editor, "Programming");
      expect(editor.value().replace("]] ,", "]],")).toBe(`project:: ${opener}Programming]]${following}`);
    },
  );

  it.each([
    ["[[", "[[Existing]]", "Programming", "[[Programming]]"],
    ["#[[", "#[[Existing]]", "Programming", "#[[Programming]]"],
    ["#", "[[Existing]]", "#Programming notes", "#[[Programming notes]]"],
    ["#", "#[[Existing]]", "#Programming notes", "#[[Programming notes]]"],
  ])("preserves later property values after %s completion before %s (#853)", async (opener, following, label, inserted) => {
    const prefix = `project:: ${opener}pr`;
    const name = label.startsWith("#") ? label.slice(1) : label;
    setGraphMeta(META); bumpGraphEpoch();
    vi.spyOn(backend(), "quickSwitch").mockResolvedValue([{ name, kind: "page", date_key: null, path: `pages/${name}.md` }]);
    const editor = mount(`${prefix}, ${following}`, prefix.length);
    editor.type("o");
    await acceptSuggestion(editor, label);
    expect(editor.value().replace("]] ,", "]],")).toBe(`project:: ${inserted}, ${following}`);
  });

  it.each(["((existing))", "[[Keep )) intact]]", "#[[Keep )) intact]]"])(
    "preserves %s after accepting a block suggestion (#853)", async following => {
      const prefix = "project:: ((pr";
      const id = "11111111-1111-4111-8111-111111111111";
      setGraphMeta(META); bumpGraphEpoch();
      openAsLoaded(await bindTestHost());
      const target = { id, raw: `first\nid:: ${id}`, collapsed: false, children: [] };
      vi.spyOn(backend(), "getPage").mockResolvedValue({
        id: "pages/Source.md", name: "Source", kind: "page", title: "Source", pre_block: null, blocks: [target],
      });
      vi.spyOn(backend(), "search").mockResolvedValue([{
        page: "Source", kind: "page", evidence: [], blocks: [{ ...target, properties: [["id", id]] }],
      }]);
      const editor = mount(`${prefix}, ${following}`, prefix.length);
      editor.type("o");
      await acceptSuggestion(editor, "first");
      await vi.waitFor(() => expect(editor.value().replace(")) ,", ")),"))
        .toBe(`project:: ((${id})), ${following}`));
    },
  );

  describe("template slash-completion (HONP4)", () => {
    const DAILY = [{ id: "tpl", raw: "from template", collapsed: false, children: [{ id: "tpl-child", raw: "template child", collapsed: false, children: [] }] }];
    async function openTemplate(raw: string, blocks = DAILY, name = "Daily"): Promise<EditorDriver> {
      vi.spyOn(backend(), "listTemplates").mockResolvedValue([{ name, page: "Templates", kind: "page", blocks }]);
      setGraphMeta(META); bumpGraphEpoch();
      const editor = mount(raw);
      editor.reportInput({ bubbles: true, inputType: "insertText", data: raw.at(-1) });
      await vi.waitFor(() => expect(document.body.querySelector(".autocomplete .ac-label")?.textContent).toBe(`Template: ${name}`));
      return editor;
    }

    it.each(["typing", "paste"])("finds an existing page by %s in the inserted meeting template's empty project property (#853)", async method => {
      const raw = "###\nevent:: #meeting\nproject::";
      const editor = await openTemplate("/meeting", [{ id: "tpl", raw, collapsed: false, children: [] }], "meeting");
      vi.spyOn(backend(), "quickSwitch").mockImplementation(async query => query === "pro"
        ? [{ name: "Programming", kind: "page", date_key: null, path: "pages/Programming.md" }]
        : []);
      vi.spyOn(backend(), "queryFacets").mockResolvedValue([["project", []]]);
      enter(editor);
      const inserted = roots()[1];
      const shown = await vi.waitFor(() => editorIn(root, inserted));
      expect(shown.value()).toBe(raw);
      expect(shown.selection()).toEqual([raw.length, raw.length]);
      shown.type(" ");
      if (method === "typing") {
        for (const ch of "[[pro") shown.type(ch);
      } else {
        const paste = shown.paste({ text: "[[pro" });
        if (!paste.defaultPrevented) shown.input(`${raw} [[pro`, raw.length + 6, { inputType: "insertFromPaste" });
      }
      await vi.waitFor(() => expect([...document.querySelectorAll(".autocomplete .ac-label")]
        .map(row => row.textContent)).toContain("Programming"));
      enter(shown);
      expect(shown.value()).toMatch(/^###\nevent:: #meeting\nproject:: \[\[Programming\]\] ?$/);
    });

    it.each([["after text", "intro /Daily", "intro "], ["in an empty block", "/Daily", null]] as const)(
      "%s: a failed insert keeps the trigger, in the editor and in the block", async (_, raw, _head) => {
        const editor = await openTemplate(raw);
        const before = snapshot(editor);
        writes.document = 0;
        wasm.failNextApply = true;
        expect(enter(editor).defaultPrevented).toBe(true);
        await settle();
        expectUnchanged(editor, { ...before, selection: [raw.length, raw.length] });
      });

    it.each([["after text", "intro /Daily", "intro "], ["in an empty block", "/Daily", null]] as const)(
      "%s: a completed template removes the trigger and inserts the outline, as one undo step", async (_, raw, head) => {
        const editor = await openTemplate(raw);
        enter(editor);
        await settle();
        expect(texts()).toEqual(head === null ? ["first", "from template"] : ["first", head, "from template"]);
        const inserted = roots()[roots().length - 1];
        expect(doc.byId[inserted].children.map((id) => doc.byId[id].raw)).toEqual(["template child"]);
        expect(errors()).toHaveLength(0);
        // The caret ends at the end of the template's text, in the block it inserted, as a freshly opened
        // editor puts it (OG `insert-template!` → `edit-last-block-after-inserted!`, caret `:max`). In an
        // empty block the template took the block's place, so its own editor is the one that shows it.
        expect(editingId()).toBe(inserted);
        const shown = editorIn(root, inserted);
        expect(shown.value()).toBe("from template");
        expect(shown.selection()).toEqual([13, 13]);
        // One Undo restores the trigger text and removes the template.
        expect(undo()).toBe(true);
        expect(texts()).toEqual(["first", raw]);
      });

    it.each([["after text", "intro /Daily"], ["in an empty block", "/Daily"]] as const)(
      "%s: a calculator template opens in calculator mode, with its results and the caret at its end", async (_, raw) => {
        const editor = await openTemplate(raw, [{ id: "tpl", raw: "```calc\n1 + 1\n```", collapsed: false, children: [] }]);
        enter(editor);
        await settle();
        const inserted = roots()[roots().length - 1];
        expect(doc.byId[inserted].raw).toBe("```calc\n1 + 1\n```");
        expect(editingId()).toBe(inserted);
        const shown = editorIn(root, inserted);
        // Calculator mode shows the expressions without their fence, and their results beside them.
        expect(shown.value()).toBe("1 + 1");
        expect(shown.selection()).toEqual([5, 5]);
        expect(root.querySelector(".calc-results")?.textContent).toContain("2");
      });

    it("a completed template asks the kernel once and writes the trigger's removal and the outline once each", async () => {
      const editor = await openTemplate("intro /Daily");
      writes.document = 0; wasm.calls = {};
      enter(editor);
      await settle();
      expect(texts()).toEqual(["first", "intro ", "from template"]);
      // One kernel ask (insert_outline needs no read set), one clock transition for the staged removal,
      // and one store write each for the removal and the outline.
      expect(kernelCalls()).toEqual({ reads: 0, apply: 1, clock: 1 });
      expect(writes.document).toBe(2);
    });
  });

  describe("multi-line paste over a selection (HONP4)", () => {
    it("a failed paste changes nothing", async () => {
      const editor = mount("alpha beta gamma");
      editor.select(6, 10);
      const before = snapshot(editor);
      writes.document = 0;
      wasm.failNextApply = true;
      quietly(() => editor.paste({ text: "- one\n- two" }));
      await settle();
      expectUnchanged(editor, before);
    });

    it("a one-root outline with children pasted into an empty block takes its place, the caret at the root's end", async () => {
      const editor = mount("");
      editor.paste({ text: "- one\n  - child" });
      await settle();
      // The paste reuses the empty block for its root (OG `paste-blocks`, `replace-empty-target?`).
      expect(texts()).toEqual(["first", "one"]);
      expect(roots()[1]).toBe("target");
      expect(doc.byId.target.children.map((id) => doc.byId[id].raw)).toEqual(["child"]);
      expect(editingId()).toBe("target");
      const shown = editorIn(root, "target");
      expect(shown.value()).toBe("one");
      expect(shown.selection()).toEqual([3, 3]);
      expect(errors()).toHaveLength(0);
    });

    it("a multi-line paste asks the kernel once and writes the text and the outline once each", async () => {
      const editor = mount("alpha beta gamma");
      editor.select(6, 10);
      writes.document = 0; wasm.calls = {};
      editor.paste({ text: "- one\n- two" });
      await settle();
      expect(texts()).toEqual(["first", "alpha beta gamma", "one", "two"]);
      // One kernel ask and one store write, the outline's: the text as typed is unchanged, so it writes nothing.
      expect(kernelCalls()).toEqual({ reads: 0, apply: 1, clock: 0 });
      expect(writes.document).toBe(1);
    });

    it("a paste keeps the block's text, as Logseq does, inserts the blocks after it, and undoes as one step", async () => {
      const editor = mount("alpha beta gamma");
      editor.select(6, 10);
      editor.paste({ text: "- one\n- two" });
      await settle();
      expect(texts()).toEqual(["first", "alpha beta gamma", "one", "two"]);
      expect(errors()).toHaveLength(0);
      expect(undo()).toBe(true);
      expect(texts()).toEqual(["first", "alpha beta gamma"]);
    });
  });
});
