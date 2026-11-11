import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from "vitest";
import { For } from "solid-js";
import { render } from "solid-js/web";
import { Block, SurfaceContext } from "./Block";
import { beginPageHeaderEdit, pageByName, resetStore } from "../document";
import { doc } from "../document/model";
import { loadSingle } from "../document/workingSet";
import { editingId, endEdit, startEditing } from "../editorController";
import { setCm6EditorEnabled } from "../editor/cm6Setting";
import { preloadCm6Editor } from "./EditorGate";
import { editorIn, maybeEditorIn } from "../tests/editorDriver";
import { initParser } from "../render/parse";
import { logbookInfo } from "../logbook";
import { installKeybindings } from "../keybindings";
import { setToasts, toasts } from "../toasts";
import * as router from "../router";
import * as ui from "../ui";
import { registerWindow } from "../windowRealm";

beforeAll(async () => { await initParser(); await preloadCm6Editor(); });

const PAGE = "Editor exit parity";
const leaf = (raw: string) => ({ id: "exit-block", raw, collapsed: false, children: [] });
const CLOCK = "CLOCK: [2026-10-21 Wed 08:54:45]";
let dispose: (() => void) | undefined;
let disposeKeys: (() => void) | undefined;

function mount(raw: string, format: "md" | "org" = "md", header = false) {
  loadSingle({ name: PAGE, kind: "page", title: PAGE, format,
    pre_block: header ? "alias:: reading" : null, blocks: [leaf(raw)] });
  const id = header ? beginPageHeaderEdit(PAGE)! : "exit-block";
  startEditing(id, 0);
  const root = document.createElement("div");
  document.body.append(root);
  dispose = render(() => <For each={pageByName(PAGE)?.roots ?? []}>{id => <Block id={id} />}</For>, root);
  const editor = editorIn(root, id);
  editor.focus();
  return { root, editor, id };
}

function leave() {
  const button = document.createElement("button");
  document.body.append(button);
  button.focus();
}

function addMirror(id: string) {
  const root = document.createElement("div");
  document.body.append(root);
  const previous = dispose;
  const disposeMirror = render(() => <SurfaceContext.Provider value="sidebar:exit-parity"><Block id={id} /></SurfaceContext.Provider>, root);
  dispose = () => { disposeMirror(); previous?.(); };
  return editorIn(root, id);
}

const flush = () => new Promise<void>(resolve => queueMicrotask(resolve));

describe.each(["textarea", "cm6"] as const)("%s commit and exit parity", kind => {
  beforeEach(() => {
    setCm6EditorEnabled(kind === "cm6");
    // jsdom has no text layout; these journeys assert state, focus and caret.
    if (!Range.prototype.getClientRects) Range.prototype.getClientRects = () => [] as unknown as DOMRectList;
    if (!Range.prototype.getBoundingClientRect) Range.prototype.getBoundingClientRect = () => new DOMRect();
    vi.spyOn(document, "hasFocus").mockReturnValue(true);
    setToasts([]);
    disposeKeys = installKeybindings();
  });
  afterEach(() => {
    dispose?.(); dispose = undefined;
    disposeKeys?.(); disposeKeys = undefined;
    endEdit("page-navigation");
    resetStore();
    setToasts([]);
    setCm6EditorEnabled(false);
    vi.restoreAllMocks();
    document.body.innerHTML = "";
  });

  it.each(["md", "org"] as const)("D2 typing DONE over a bare DOING closes the clock (%s)", format => {
    const { editor, id } = mount(`DOING\n:LOGBOOK:\n${CLOCK}\n:END:`, format);
    editor.select(0, 5);
    for (const ch of "DONE") editor.type(ch);
    expect(doc.byId[id].raw.split("\n")[0]).toBe("DONE");
    const rows = logbookInfo(doc.byId[id].raw, format).rows;
    expect(rows).toHaveLength(1);
    expect(rows[0].end).not.toBeNull();
  });

  it.each(["md", "org"] as const)("D3 leaving moves authored planning below the task title (%s)", format => {
    const { editor, id } = mount("TODO ship it\nsome note", format);
    editor.input("TODO ship it\nsome note\nSCHEDULED: <2026-07-06 Mon>");
    leave();
    expect(editingId()).toBeNull();
    expect(doc.byId[id].raw).toBe("TODO ship it\nSCHEDULED: <2026-07-06 Mon>\nsome note");
  });

  it.each([
    ["md", "```\nline one\nline two\nSCHEDULED: <2026-07-06 Mon>\n```"],
    ["org", "#+BEGIN_SRC text\nline one\nline two\nSCHEDULED: <2026-07-06 Mon>\n#+END_SRC"],
    ["md", "```calc\n1 + 1\n2 + 2\nSCHEDULED: <2026-07-06 Mon>\n```"],
  ] as const)("D3 leaving a literal body preserves its line order (%s)", (format, raw) => {
    const { id } = mount(raw, format);
    leave();
    expect(doc.byId[id].raw).toBe(raw);
  });

  it("D3 switching occurrences normalizes only when the destination exits", async () => {
    const { editor, id } = mount("TODO ship it\nsome note");
    const text = "TODO ship it\nsome note\nSCHEDULED: <2026-07-06 Mon>";
    editor.input(text);
    const mirror = addMirror(id);
    mirror.focus();
    await flush();
    expect(editingId()).toBe(id);
    expect(doc.byId[id].raw).toBe(text);
    leave();
    expect(doc.byId[id].raw).toBe("TODO ship it\nSCHEDULED: <2026-07-06 Mon>\nsome note");
  });

  it("D4 leaving an invalid page header explains why it cannot save", () => {
    const { editor, id } = mount("body", "md", true);
    editor.input("alias:: reading\nnot a property");
    leave();
    expect(doc.byId[id].raw).toContain("not a property");
    expect(toasts().map(t => t.message).join("\n")).toContain("not a property");
    expect(toasts().some(t => t.kind === "error")).toBe(true);
  });

  it("D4 leaving a cleared transient page header removes its empty root", () => {
    const { editor, id } = mount("body", "md", true);
    editor.input("");
    leave();
    expect(pageByName(PAGE)!.roots).toEqual(["exit-block"]);
    expect(doc.byId[id]).toBeUndefined();
  });

  it("D4 leaving a page header removes leading blank lines", () => {
    const { editor, id } = mount("body", "md", true);
    editor.input("\n\nalias:: reading");
    leave();
    expect(doc.byId[id].raw).toBe("alias:: reading");
  });

  it("D6 returning from another application resumes the same editor and caret", () => {
    const { root, editor, id } = mount("TODO ship it\nsome note");
    const text = "TODO ship it\nsome note\nSCHEDULED: <2026-07-06 Mon>";
    editor.input(text);
    editor.select(3, 8);
    vi.mocked(document.hasFocus).mockReturnValue(false);
    editor.blur();
    expect(editingId()).toBe(id);
    expect(doc.byId[id].raw).toBe(text);
    vi.mocked(document.hasFocus).mockReturnValue(true);
    window.dispatchEvent(new Event("focus"));
    const resumed = maybeEditorIn(root)!;
    expect(resumed.element).toBe(editor.element);
    expect(document.activeElement).toBe(resumed.element);
    expect(resumed.selection()).toEqual([3, 8]);
  });

  it("D6 moving to another Tine window finishes the suspended edit", () => {
    const { editor, id } = mount("TODO ship it\nsome note");
    editor.input("TODO ship it\nsome note\nSCHEDULED: <2026-07-06 Mon>");
    vi.mocked(document.hasFocus).mockReturnValue(false);
    editor.blur();
    const frame = document.createElement("iframe");
    document.body.append(frame);
    const other = frame.contentWindow!;
    const unregister = registerWindow("exit-parity-other", other);
    try {
      other.dispatchEvent(new Event("focus"));
      expect(editingId()).toBeNull();
      expect(doc.byId[id].raw).toBe("TODO ship it\nSCHEDULED: <2026-07-06 Mon>\nsome note");
    } finally { unregister(); frame.remove(); }
  });

  it.each([false, true])("D16 follows the link at the caret (sidebar: %s)", sidebar => {
    const openPage = vi.spyOn(router, "openPage").mockImplementation(() => {});
    const openSidebar = vi.spyOn(ui, "openPageInSidebar").mockImplementation(() => {});
    const { editor } = mount("[[First]] then [[Destination]]");
    editor.select(21, 21);
    editor.keydown({ key: "o", code: "KeyO", ctrlKey: true, shiftKey: sidebar });
    expect(sidebar ? openSidebar : openPage).toHaveBeenCalledWith("Destination");
    expect(sidebar ? openPage : openSidebar).not.toHaveBeenCalled();
  });

  it("D16 follows the focused occurrence after another occurrence mounts", async () => {
    const openPage = vi.spyOn(router, "openPage").mockImplementation(() => {});
    const { editor, id } = mount("[[First]] then [[Destination]]");
    editor.select(21, 21);
    addMirror(id);
    await flush();
    expect(document.activeElement).toBe(editor.element);
    editor.keydown({ key: "o", code: "KeyO", ctrlKey: true });
    expect(openPage).toHaveBeenCalledWith("Destination");
  });

  it("D16 reads the Org source format when choosing a link", () => {
    const openPage = vi.spyOn(router, "openPage").mockImplementation(() => {});
    const { editor } = mount("~[[Literal]]~ then [[Destination]]", "org");
    editor.select(4, 4);
    editor.keydown({ key: "o", code: "KeyO", ctrlKey: true });
    expect(openPage).toHaveBeenCalledWith("Destination");
  });
});
