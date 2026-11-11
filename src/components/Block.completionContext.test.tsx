// Where the editor's completion menus open, asserted on the menu the user sees.
// Runs against the textarea (render suite) and CodeMirror (cm6 suite) editors.
import { afterEach, beforeAll, expect, it, vi } from "vitest";
import { For } from "solid-js";
import { render } from "solid-js/web";
import { backend } from "../backend";
import { endEdit, startEditing } from "../editorController";
import { pageByName, resetStore } from "../document";
import { loadSingle } from "../document/workingSet";
import { initParser } from "../render/parse";
import { editorIn, type EditorDriver } from "../tests/editorDriver";
import { Block } from "./Block";
import type { PageEntry } from "../types";

beforeAll(() => initParser());
afterEach(() => { endEdit("blur"); vi.restoreAllMocks(); resetStore(); document.body.innerHTML = ""; });
const flush = async () => { for (let i = 0; i < 40; i++) await Promise.resolve(); };
const settle = async () => { await flush(); await new Promise(resolve => setTimeout(resolve, 150)); await flush(); };

const PAGES = ["Sat, 01.11.2026", "Sat, 07.11.2026", "Topic"];
const entry = (name: string): PageEntry => ({ name, kind: "page", path: `pages/${name}.md`, date_key: null });
function pageSearch() {
  return vi.spyOn(backend(), "quickSwitch").mockImplementation(async (query, limit) => limit === 0 ? []
    : PAGES.filter(name => name.toLowerCase().includes(query.toLowerCase())).map(entry));
}

function mount(raw: string, caret: number) {
  loadSingle({ name: "Completion context", title: "Completion context", kind: "page", pre_block: null,
    blocks: [{ id: "context", raw, collapsed: false, children: [] }] });
  startEditing("context", caret);
  const root = document.createElement("div"); document.body.append(root);
  const dispose = render(() => <For each={pageByName("Completion context")?.roots ?? []}>{id => <Block id={id} />}</For>, root);
  return { editor: editorIn(root), dispose };
}

// CM receives a platform input notification after its transaction (see Block.completionInput.test.tsx).
function type(editor: EditorDriver, text: string) {
  editor.type(text);
  if (editor.kind === "cm6") editor.reportInput({ bubbles: true, inputType: "insertText", data: text });
}

const items = () => [...document.querySelectorAll(".autocomplete .ac-item")].map(item => item.textContent ?? "");

// GH #888 (D-20261109-LK): a deliberate difference from Logseq, which searches
// `[[` to the caret (components/editor.cljs:105-121 page-search, 6e7afa8eb).
it("editing inside a complete link offers the link's own page first, and picking it keeps one link (GH #888)", async () => {
  const search = pageSearch();
  const raw = "see [[Sat 07.11.2026]] later";
  const { editor, dispose } = mount(raw, raw.indexOf(" 07"));
  try {
    editor.select(raw.indexOf(" 07"));
    type(editor, ",");
    await vi.waitFor(() => expect(items().length).toBeGreaterThan(0));
    expect(items()[0]).toContain("Sat, 07.11.2026");
    expect(search.mock.calls.filter(([, limit]) => limit > 0).at(-1)?.[0]).toBe("Sat, 07.11.2026");
    editor.keydown({ key: "Enter", code: "Enter" });
    await vi.waitFor(() => expect(editor.value()).toBe("see [[Sat, 07.11.2026]] later"));
  } finally { dispose(); }
});

it("typing a new link still searches the text before the caret (GH #888)", async () => {
  const search = pageSearch();
  const { editor, dispose } = mount("", 0);
  try {
    for (const char of "[[Sat,") type(editor, char);
    await vi.waitFor(() => expect(items().length).toBeGreaterThan(0));
    expect(editor.value()).toBe("[[Sat,]]");
    expect(search.mock.calls.filter(([, limit]) => limit > 0).at(-1)?.[0]).toBe("Sat,");
    expect(items()[0]).toContain("Sat, 01.11.2026");
  } finally { dispose(); }
});

// CM6-D9 (Martin 2026-11-09): no completion menu inside a fenced code block in
// either editor; Logseq opens them there (handler/editor.cljs:1606, :1894).
it.each([
  ["a code-only block", "```\ncode\n```"],
  ["a fence inside prose", "intro\n```\ncode\n```"],
])("no menu opens while typing inside %s (CM6-D9)", async (_name, raw) => {
  const search = pageSearch();
  vi.spyOn(backend(), "listTemplates").mockResolvedValue([]);
  vi.spyOn(backend(), "search").mockResolvedValue([]);
  const { editor, dispose } = mount(raw, raw.indexOf("code") + 4);
  try {
    // Either editor may show the fence as its code body: find the code in what it shows.
    editor.selectByUser(editor.value().indexOf("code") + 4); await flush();
    editor.selectByUser(editor.value().indexOf("code") + 4); await flush();
    for (const typed of [" [[To", " /", " #To", " ((To"]) {
      for (const char of typed) type(editor, char);
      await settle();
      expect(document.querySelector(".autocomplete"), `${typed} inside code`).toBeNull();
    }
    expect(editor.value()).toContain("code [[To"); // the keys reached the code
    expect(search.mock.calls.filter(([, limit]) => limit > 0)).toEqual([]);
  } finally { dispose(); }
});

// Escape shows a code block's raw source in both editors (GH #865); its body is still code.
it("no menu opens inside a fence whose source Escape revealed (CM6-D9)", async () => {
  const search = pageSearch();
  const raw = "```\ncode\n```";
  const { editor, dispose } = mount(raw, raw.indexOf("code") + 4);
  try {
    editor.selectByUser(editor.value().indexOf("code") + 4); await flush();
    editor.keydown({ key: "Escape", code: "Escape" }); await flush();
    const shown = editorIn(document.body);
    expect(shown.value()).toBe(raw);
    shown.selectByUser(raw.indexOf("code") + 4); await flush();
    for (const char of " [[To") type(shown, char);
    await settle();
    expect(document.querySelector(".autocomplete")).toBeNull();
    expect(shown.value()).toContain("code [[To");
    expect(search.mock.calls.filter(([, limit]) => limit > 0)).toEqual([]);
  } finally { dispose(); }
});

it("the same menu still opens on the prose line beside a fence (CM6-D9)", async () => {
  pageSearch();
  const raw = "intro\n```\ncode\n```";
  const { editor, dispose } = mount(raw, 5);
  try {
    editor.selectByUser(5); await flush();
    for (const char of " [[To") type(editor, char);
    await vi.waitFor(() => expect(items()).toContain("Topic"));
  } finally { dispose(); }
});
