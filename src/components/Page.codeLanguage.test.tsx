import { afterEach, describe, expect, it, vi } from "vitest";
import { backend } from "../backend";
import { node, flushPage, selectBlock } from "../document";
import { installKeybindings } from "../keybindings";
import { bindTestHost, mailPage, submittedPages } from "../document/host/wiring.test.support";
import { mainPaneRouter, resetTabsToJournals } from "../router";
import { endEdit, startEditing } from "../editorController";
import { editorIn, type EditorKind } from "../tests/editorDriver";
import { useEditorKind } from "../tests/editorKind";
import { installBlockEditorLifecycle, mount } from "../tests/blockEditorTestkit";
import { PageView } from "./Page";
import { PaneContext, paneRouter, resetPaneLayoutToSingle, splitPane } from "../panes";
import { SurfaceContext } from "./Block";
import { RightSidebar } from "./RightSidebar";
import { applySidebarSession } from "../ui";
import { AstBody } from "../render/body";
import { parseBlock } from "../render/parse";
import * as clipboard from "../clipboard";
import { dispatchFocusedEditorCommand } from "../editorCommandBridge";

installBlockEditorLifecycle();
afterEach(() => { endEdit("page-navigation"); resetTabsToJournals(); resetPaneLayoutToSingle(); applySidebarSession({ right: false, items: [] }); vi.restoreAllMocks(); });
describe.each<EditorKind>(["textarea", "cm6"])("code language source in %s", kind => {
  useEditorKind(kind);
  // lsdoc v0.5.11 parse.rs/org.rs: fence_marker/fence_lang and block_begin/begin_lang.
  it.each((["md", "org"] as const).flatMap(format => [
    ["```", "```", ""], ["~~~", "~~~", ""],
    ["```js :results output", "```", "js"], ["~~~\t js :results output", "~~~", "js"],
    ["````js :results output", "````", "`js"], ["~~~~~", "~~~~~", "~~"],
    ["  ```", "  ```", ""], ["  #+BEGIN_SRC", "  #+END_SRC", ""],
    ["#+BEGIN_SRC", "#+END_SRC", ""], ["#+begin_src", "#+end_src", ""],
    ["#+BeGiN_sRc", "#+EnD_sRc", ""],
    ["#+BEGIN_SRC js :results output", "#+END_SRC", "js"],
    ["#+begin_src\tjs :results output", "#+end_src", "js"],
    ["#+BEGIN_SRC ", "#+END_SRC", ""],
    ["#+BEGIN_EXAMPLE", "#+END_EXAMPLE", ""], ["#+begin_example", "#+end_example", ""],
  ].flatMap(([open, close, lang]) => [false, true].flatMap(mixed => ["leave", "typing", "paste", "programmatic paste", "IME", "drop", "replacement", "plain input"].map(entry => ({ format, open, close, lang, mixed, entry })))))) (
    "keeps the source block and language after label activation: %j", async ({ format, open, close, lang, mixed, entry }) => {
      const raw = `${mixed ? "前 🐈\n" : ""}${open}\nx\n${close}${mixed ? "\nAfter" : ""}`;
      const literal = /begin_example/i.test(open) ? "example" : "src";
      const parsed = parseBlock(raw, format === "org").find(block => block.kind === literal)!;
      expect(parsed).toMatchObject(literal === "src" ? { kind: "src", lang } : { kind: "example" });
      const dto = { id: `pages/Code.${format}`, name: "Code", title: "Code", kind: "page" as const, pre_block: null,
        blocks: [{ id: "code", raw, children: [], collapsed: false }] };
      vi.spyOn(backend(), "getPage").mockResolvedValue(dto);
      mainPaneRouter.replaceActiveRoute({ kind: "page", name: "Code", pageKind: "page" });
      const { root, dispose } = mount(() => <PageView />);
      try {
        await vi.waitFor(() => expect(root.querySelector("button.code-language")).not.toBeNull());
        const host = await bindTestHost();
        vi.spyOn(backend(), "pageOpen").mockImplementation(async (_session, id) => {
          queueMicrotask(() => host.deliver({ key: dto.id, page: mailPage(1, dto),
            answer: { id, version: 1, took: false, outcome: { kind: "applied" } } }));
          return { key: dto.id, baselineEntry: true };
        });
        const submits = vi.spyOn(backend(), "pageSubmit");
        const reads = vi.mocked(backend().getPage).mock.calls.length;
        root.querySelector<HTMLButtonElement>("button.code-language")!.click();
        await vi.waitFor(() => expect(editorIn(root).value()).toBe(raw));
        expect(submits).not.toHaveBeenCalled();
        expect(vi.mocked(backend().getPage).mock.calls).toHaveLength(reads);
        if (entry === "leave") {
          endEdit("page-navigation");
          expect(node("code").raw, "activation without typing preserves every byte").toBe(raw);
          expect(submits).not.toHaveBeenCalled();
          return;
        }
        const editor = editorIn(root), [at] = editor.selection();
        editor.select(at, at + lang.length);
        const inserted = raw.slice(0, at) + "rust" + raw.slice(at + lang.length);
        if (entry === "typing") for (const ch of "rust") editorIn(root).type(ch);
        else if (entry === "paste") {
          const paste = editor.paste({ text: "rust" });
          // jsdom omits textarea's browser paste default; CM runs its native pipeline.
          if (!paste.defaultPrevented) editor.input(inserted, at + 4, { inputType: "insertFromPaste", data: null });
        } else if (entry === "programmatic paste") {
          vi.spyOn(clipboard, "readClipboardText").mockResolvedValue("rust");
          editor.focus();
          expect(dispatchFocusedEditorCommand("editor/paste-plain")).toBe(true);
          await vi.waitFor(() => expect(node("code").raw).not.toBe(raw));
        } else if (entry === "IME") {
          editor.compositionStart();
          editor.input(inserted, at + 4, { inputType: "insertCompositionText", data: "rust", isComposing: true });
          expect(node("code").raw, "preedit stays local").toBe(raw);
          editor.compositionEnd("rust");
          await Promise.resolve();
        } else editor.input(inserted, at + 4, entry === "plain input" ? { eventKind: "plain" } :
          { inputType: entry === "drop" ? "insertFromDrop" : "insertReplacementText", data: null });
        endEdit("page-navigation");
        await flushPage("Code");
        const saved = submittedPages(submits, "Code").at(-1)!.blocks[0].raw;
        const expected = raw.slice(0, at) + (["#+begin_src", "#+begin_example"].includes(open.trimStart().toLowerCase()) ? " " : "") + "rust" + raw.slice(at + lang.length);
        expect(saved).toBe(expected);
        expect(parseBlock(saved, format === "org").find(block => block.kind === literal)).toMatchObject(
          literal === "src" ? { kind: "src", lang: "rust", code: (parsed as { code: string }).code } : { kind: "example", code: (parsed as { code: string }).code });
      } finally { dispose(); }
    });
  it.each(["before", "after"])("opens another fence %s the currently edited fence", async side => {
    const raw = "```js\nx\n```\nMiddle 汉字\n```python\ny\n```";
    vi.spyOn(backend(), "getPage").mockResolvedValue({ id: "pages/Code.md", name: "Code", title: "Code", kind: "page", pre_block: null,
      blocks: [{ id: "code", raw, children: [], collapsed: false }] });
    mainPaneRouter.replaceActiveRoute({ kind: "page", name: "Code", pageKind: "page" });
    const { root, dispose } = mount(() => <PageView />);
    try {
      await vi.waitFor(() => expect(root.querySelectorAll("button.code-language")).toHaveLength(2));
      startEditing("code", side === "before" ? raw.indexOf("\ny") + 1 : raw.indexOf("\nx") + 1);
      const target = side === "before" ? "js" : "python";
      Array.from(root.querySelectorAll<HTMLButtonElement>("button.code-language")).find(button => button.textContent === target)!.click();
      await vi.waitFor(() => expect(editorIn(root).value()).toBe(raw));
      const caret = raw.indexOf(target);
      expect(editorIn(root).selection()).toEqual([caret, caret]);
    } finally { dispose(); }
  });
  it.each(["split", "sidebar"])("edits the occurrence in the %s", async surface => {
    const raw = "Before\n```js\nx\n```\nAfter";
    vi.spyOn(backend(), "getPage").mockResolvedValue({ id: "pages/Code.md", name: "Code", title: "Code", kind: "page", pre_block: null,
      blocks: [{ id: "code", raw, children: [], collapsed: false }] });
    const router = surface === "split" ? paneRouter(splitPane("main", "row")!) : mainPaneRouter;
    router.replaceActiveRoute({ kind: "page", name: "Code", pageKind: "page" });
    if (surface === "sidebar") applySidebarSession({ right: true, items: [{ kind: "page", name: "Code", pageKind: "page" }] });
    const { root, dispose } = mount(() => surface === "sidebar" ? <RightSidebar /> :
      <PaneContext.Provider value={{ paneId: router.paneId, router }}><SurfaceContext.Provider value={`pane:${router.paneId}`}><PageView /></SurfaceContext.Provider></PaneContext.Provider>);
    try {
      await vi.waitFor(() => expect(root.querySelector("button.code-language")).not.toBeNull());
      root.querySelector<HTMLButtonElement>("button.code-language")!.click();
      await vi.waitFor(() => expect(editorIn(root).value()).toBe(raw));
      expect(editorIn(root).selection()).toEqual([10, 10]);
    } finally { dispose(); }
  });
  it.each([false, true].flatMap(mixed => ["js", ""].flatMap(lang => [false, true, ...(kind === "cm6" && mixed ? ["prose"] : [])].flatMap(editing => ["click", "Enter", " "].map(activation => ({ mixed, lang, editing, activation }))))))(
    "opens the language on a routed page: %j", async ({ mixed, lang, editing, activation }) => {
      let before = mixed ? "Intro 汉字\n" : "";
      let raw = `${before}\`\`\`${lang}\nconst n = 1;\n\`\`\`${mixed ? "\nAfter" : ""}`;
      vi.spyOn(backend(), "getPage").mockResolvedValue({ id: "pages/Code.md", name: "Code", title: "Code", kind: "page", pre_block: null,
        blocks: [{ id: "code", raw, children: [], collapsed: false }] });
      mainPaneRouter.replaceActiveRoute({ kind: "page", name: "Code", pageKind: "page" });
      const { root, dispose } = mount(() => <PageView />);
      const releaseKeys = installKeybindings();
      try {
        await vi.waitFor(() => expect(root.querySelector(".code-language")).not.toBeNull());
        await bindTestHost();
        if (editing) startEditing("code", editing === "prose" ? 0 : raw.indexOf("const"));
        if (editing === "prose") {
          editorIn(root).type("Edited ");
          before = "Edited " + before;
          raw = "Edited " + raw;
        }
        if (!editing && activation !== "click") selectBlock("code");
        const submits = vi.spyOn(backend(), "pageSubmit");
        const reads = vi.mocked(backend().getPage).mock.calls.length;
        const label = root.querySelector<HTMLButtonElement>("button.code-language");
        if (editing === "prose") {
          // Explicit source editing shows the fences themselves, without a
          // caret-dependent code preview or its language button (CS879).
          expect(label).toBeNull();
          expect(editorIn(root).element.textContent).toContain("```" + lang);
          editorIn(root).select(before.length + 3);
        } else {
          expect(label, "a focusable language control is available even without a language").not.toBeNull();
          expect(label!.getAttribute("aria-label")).toBe("Edit code language");
          // Review HOLD on bc643657b: with the window focused, the editor's blur removed the label before activation.
          vi.spyOn(document, "hasFocus").mockReturnValue(true);
          label!.focus();
          expect(document.activeElement).toBe(label);
          expect(label!.isConnected, "focusing the label keeps the editing session that renders it").toBe(true);
          if (activation !== "click") {
            const event = new KeyboardEvent("keydown", { key: activation, bubbles: true, cancelable: true });
            label!.dispatchEvent(event);
            expect(event.defaultPrevented, "native activation must remain available").toBe(false);
          }
          if (activation === "click") label!.dispatchEvent(new MouseEvent("mousedown", { bubbles: true, cancelable: true, button: 0 }));
          label!.click(); // jsdom omits keyboard default activation; Chromium exercises it natively.
        }
        await vi.waitFor(() => expect(editorIn(root).value()).toBe(raw));
        expect(editorIn(root).selection()).toEqual([before.length + 3, before.length + 3]);
        expect(vi.mocked(backend().getPage).mock.calls).toHaveLength(reads);
        expect(submits).not.toHaveBeenCalled();
        editorIn(root).type("text");
        endEdit("page-navigation");
        expect(node("code").raw).toBe(raw.slice(0, before.length + 3) + "text" + raw.slice(before.length + 3));
        expect(node("code").raw).not.toContain("plain");
        await flushPage("Code");
        const saved = submittedPages(submits, "Code").at(-1)!;
        expect(saved.blocks[0].raw).toBe(node("code").raw);
        expect(JSON.stringify(saved)).not.toContain("plain");
      } finally { releaseKeys(); dispose(); }
    });
});

it("other versions and output keep an inert label", () => {
  const { root, dispose } = mount(() => <AstBody raw={"```\nx\n```"} />);
  try {
    expect(root.querySelector(".code-language")?.textContent).toBe("plain");
    expect(root.querySelector("button.code-language")).toBeNull();
  } finally { dispose(); }
});
it.each(["guide", "read_only"])("a %s routed page keeps an inert label", async flag => {
  vi.spyOn(backend(), "getPage").mockResolvedValue({ id: "pages/Code.md", name: "Code", title: "Code", kind: "page", pre_block: null,
    [flag]: true, blocks: [{ id: "code", raw: "```\nx\n```", children: [], collapsed: false }] });
  mainPaneRouter.replaceActiveRoute({ kind: "page", name: "Code", pageKind: "page" });
  const { root, dispose } = mount(() => <PageView />);
  try {
    await vi.waitFor(() => expect(root.querySelector(".code-language")?.textContent).toBe("plain"));
    expect(root.querySelector("button.code-language")).toBeNull();
  } finally { dispose(); }
});
