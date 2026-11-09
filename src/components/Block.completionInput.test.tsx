import { afterEach, beforeAll, expect, it, vi } from "vitest";
import { For } from "solid-js";
import { render } from "solid-js/web";
import { backend } from "../backend";
import { startEditing } from "../editorController";
import { pageByName, resetStore } from "../document";
import { loadSingle } from "../document/workingSet";
import { initParser } from "../render/parse";
import { editorIn, type EditorDriver } from "../tests/editorDriver";
import { Block } from "./Block";
import type { PageEntry } from "../types";

beforeAll(() => initParser());
afterEach(() => { vi.restoreAllMocks(); resetStore(); document.body.innerHTML = ""; });
const flush = async () => { for (let i = 0; i < 40; i++) await Promise.resolve(); };
const row: PageEntry = { name: "Topic", kind: "page", path: "pages/Topic.md", date_key: null };

function mount() {
  loadSingle({ name: "Completion input", title: "Completion input", kind: "page", pre_block: null,
    blocks: [{ id: "completion-input", raw: "", collapsed: false, children: [] }] });
  startEditing("completion-input", 0);
  const root = document.createElement("div"); document.body.append(root);
  const dispose = render(() => <For each={pageByName("Completion input")?.roots ?? []}>{id => <Block id={id} />}</For>, root);
  return { editor: editorIn(root), dispose };
}

// Native CM first reconciles the edit, then also receives a platform input
// notification. The shared driver reports the transaction; add its notification
// explicitly so both event paths participate in this regression.
function type(editor: EditorDriver, text: string, notificationFirst = false) {
  if (editor.kind === "cm6" && notificationFirst)
    editor.reportInput({ bubbles: true, inputType: "insertText", data: text });
  editor.type(text);
  if (editor.kind === "cm6" && !notificationFirst)
    editor.reportInput({ bubbles: true, inputType: "insertText", data: text });
}

it.each(["[[", "#"])("%s starts one read per input and paints the first reply", async trigger => {
  let reply!: (rows: PageEntry[]) => void;
  const read = vi.spyOn(backend(), "quickSwitch").mockImplementation((_q, limit) => limit === 0
    ? Promise.resolve([]) : new Promise(resolve => { reply = resolve; }));
  const positive = () => read.mock.calls.filter(([, limit]) => limit > 0);
  const { editor, dispose } = mount();
  try {
    type(editor, trigger); await flush();
    expect(positive()).toHaveLength(0);
    for (const [i, char] of [..."Top"].entries()) {
      type(editor, char, i % 2 === 1); await flush();
      expect(positive()).toHaveLength(i + 1);
      expect(positive().at(-1)?.[0]).toBe("Top".slice(0, i + 1));
      reply([row]); await flush();
      // A duplicate refresh invalidates this first reply and queues another
      // read. Hold that potential second reply so the missing rows are visible.
      expect(document.querySelector(".autocomplete")?.textContent ?? "").toContain("Topic");
      expect(positive()).toHaveLength(i + 1);
    }
    editor.selectByUser(0); await flush();
    expect(document.querySelector(".autocomplete")).toBeNull();
    expect(positive()).toHaveLength(3);
    editor.selectByUser(editor.value().length); await flush();
    expect(positive()).toHaveLength(3);
  } finally { dispose(); }
});

it.each(["block", "command", "property-name", "property-value"])("%s keeps its existing read budget", async kind => {
  const search = vi.spyOn(backend(), "search").mockResolvedValue([{
    page: "Target", kind: "page", evidence: [],
    blocks: [{ id: "target", raw: "Topic", collapsed: false, children: [] }],
  }]);
  const facets = vi.spyOn(backend(), "queryFacets").mockResolvedValue([["alpha", ["two"]]]);
  const templates = vi.spyOn(backend(), "listTemplates").mockResolvedValue([]);
  const { editor, dispose } = mount();
  try {
    const opener = kind === "block" ? "((" : kind === "command" ? "/"
      : kind === "property-name" ? "::" : "alpha:: ";
    for (const char of opener) type(editor, char);
    await flush();
    const chars = kind === "property-name" ? "al" : kind === "property-value" ? "tw" : "To";
    for (const [index, char] of [...chars].entries()) {
      type(editor, char);
      if (kind === "block") await vi.waitFor(() => expect(search).toHaveBeenCalledTimes(index + 1));
      await vi.waitFor(() => expect(document.querySelector(".autocomplete .ac-item")).not.toBeNull());
      await flush();
    }
    if (kind === "block") {
      expect(search.mock.calls.map(([q]) => q)).toEqual(["T", "To"]);
      expect(facets).not.toHaveBeenCalled();
    } else if (kind === "command") {
      expect(search).not.toHaveBeenCalled();
      expect(facets).not.toHaveBeenCalled();
      expect(templates.mock.calls.length).toBeLessThanOrEqual(1);
    } else {
      expect(search).not.toHaveBeenCalled();
      expect(facets).toHaveBeenCalledTimes(1);
      expect(facets).toHaveBeenCalledWith(true, kind === "property-name" ? { keys: true } : { key: "alpha" });
      expect(document.querySelector(".autocomplete")?.textContent).toContain(kind === "property-name" ? "alpha" : "two");
    }
  } finally { dispose(); }
});

it("pasting starts one page read and composition starts none until it finishes", async () => {
  const read = vi.spyOn(backend(), "quickSwitch").mockResolvedValue([row]);
  const positive = () => read.mock.calls.filter(([, limit]) => limit > 0);
  const { editor, dispose } = mount();
  try {
    const paste = editor.paste({ text: "[[To" });
    // jsdom has no browser paste default (same boundary as property tests).
    if (!paste.defaultPrevented) editor.input("[[To", 4, { inputType: "insertFromPaste", data: null });
    await flush();
    expect(positive().map(([q]) => q)).toEqual(["To"]);
    editor.compositionStart();
    editor.input("[[Top", 5, { inputType: "insertCompositionText", data: "p", isComposing: true });
    await flush(); expect(positive()).toHaveLength(1);
    editor.compositionEnd("p"); await flush();
    expect(positive().map(([q]) => q)).toEqual(["To", "Top"]);
    editor.reportInput({ bubbles: true, inputType: "insertText", data: "p" }); await flush();
    expect(positive()).toHaveLength(2);
    expect(document.querySelector(".autocomplete")?.textContent).toContain("Topic");
  } finally { dispose(); }
});
