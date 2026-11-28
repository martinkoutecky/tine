// Delete or Backspace over a read-view (rendered, not editing)
// text selection never writes the block. It opens the block's editor as a click
// and shift-click at the two ends would, with that range selected. Every row
// below wrote unselected bytes (or did nothing) when the key spliced the source
// from the rendered range: a link destination, a delimiter, an escape, inline
// HTML around a partial selection, a medium or a comment beside the text.
import { afterEach, beforeAll, describe, expect, it, vi } from "vitest";
import { render } from "solid-js/web";
import { initParser } from "../render/parse";
import * as documentApi from "../document";
import { resetStore, undo } from "../document";
import { doc, setDoc, type FeedPage, type Node as StoreNode } from "../document/model";
import { editingId, endEdit, registerEditEntryGuard, registerEditExitGuard, startEditing } from "../editorController";
import { installKeybindings } from "../keybindings";
import { BUILTIN_HIDE } from "../editor/properties";
import { editorOffsetFromRenderedRange } from "../render/spans";
import { editorIn, maybeEditorIn, type EditorKind } from "../tests/editorDriver";
import { useEditorKind } from "../tests/editorKind";
import { setLightbox, setTypographyMode } from "../ui";
import { Block, SurfaceContext } from "./Block";
import { Lightbox } from "./Toasts";
import * as layers from "../transientLayers";
import * as realm from "../windowRealm";

beforeAll(async () => {
  await initParser();
});

const PAGE = "Handoff";
let cleanup: (() => void)[] = [];

afterEach(() => {
  for (const f of cleanup.reverse()) f();
  cleanup = [];
  endEdit("page-navigation");
  window.getSelection()?.removeAllRanges();
  vi.restoreAllMocks();
  resetStore();
  document.body.innerHTML = "";
});

function mount(blocks: Record<string, string>, format: "md" | "org" = "md", surfaces = ["main"]) {
  const ids = Object.keys(blocks);
  const byId: Record<string, StoreNode> = {};
  for (const id of ids) byId[id] = { id, raw: blocks[id], collapsed: false, parent: null, page: PAGE, children: [] };
  const page: FeedPage = { name: PAGE, kind: "page", title: PAGE, preBlock: null, roots: ids, format, readOnly: false, guide: false };
  setDoc({ byId, pages: [page], feed: [PAGE], loaded: true });
  const hosts = surfaces.map(surface => {
    const host = document.createElement("div");
    host.dataset.surface = surface;
    document.body.appendChild(host);
    cleanup.push(render(() => <SurfaceContext.Provider value={surface}>{ids.map(id => <Block id={id} />)}</SurfaceContext.Provider>, host));
    return host;
  });
  cleanup.push(installKeybindings());
  return hosts;
}

function wrapperOf(id: string, host: ParentNode = document): HTMLElement {
  return host.querySelector(`[data-block-id="${id}"] > .block-main .block-content-wrapper`)!;
}

/** The `index`-th rendered text node of block `id` whose text contains `needle`. */
function text(id: string, needle: string, host: ParentNode = document, index = 0): Text {
  const walker = document.createTreeWalker(wrapperOf(id, host), NodeFilter.SHOW_TEXT);
  let seen = 0;
  for (let n = walker.nextNode(); n; n = walker.nextNode()) {
    if ((n.textContent ?? "").includes(needle) && seen++ === index) return n as Text;
  }
  throw new Error(`no rendered text ${JSON.stringify(needle)}`);
}

/** No non-collapsed document selection is left in a block's rendered content. */
function renderedSelectionGone(): boolean {
  const s = window.getSelection()!;
  if (!s.rangeCount || s.isCollapsed) return true;
  const at = s.anchorNode!;
  const el = at.nodeType === Node.ELEMENT_NODE ? at as Element : at.parentElement;
  return !el?.closest("[data-rendered-block]");
}

type Point = [Text, number];
function select(start: Point, end: Point, backward = false): Selection {
  const sel = window.getSelection()!;
  sel.removeAllRanges();
  if (backward) sel.setBaseAndExtent(end[0], end[1], start[0], start[1]);
  else sel.setBaseAndExtent(start[0], start[1], end[0], end[1]);
  return sel;
}

/** What a click at `start` and shift-click at `end` give the editor: p24 of each end. */
function clickPair(id: string, start: Point, end: Point, format: "md" | "org" = "md"): [number, number] {
  const raw = doc.byId[id].raw;
  const at = (p: Point) => editorOffsetFromRenderedRange(wrapperOf(id), { startContainer: p[0], startOffset: p[1] }, raw, BUILTIN_HIDE, format)!;
  return [at(start), at(end)];
}

function press(key: string, init: KeyboardEventInit = {}): KeyboardEvent {
  const e = new KeyboardEvent("keydown", { key, bubbles: true, cancelable: true, ...init });
  window.dispatchEvent(e);
  return e;
}

const firstOf = (needle: string, at = 0) => (id: string): Point => { const t = text(id, needle); return [t, t.data.indexOf(needle) + at]; };
const lastOf = (needle: string, at = needle.length) => (id: string): Point => { const t = text(id, needle); return [t, t.data.lastIndexOf(needle) + at]; };

/** `shows`: the editor's selected text, when the row pins it. */
type Row = { name: string; raw: string; format?: "md" | "org"; start: (id: string) => Point; end: (id: string) => Point; selected?: string; shows?: string };
const rows: Row[] = [
  // Data loss on the write path (checks A–E): each wrote bytes the user did not select.
  { name: "a link label cut into the text after it", raw: "[label](https://e.org) after", start: firstOf("label", 2), end: firstOf(" after", 3) },
  { name: "an Org link label cut", raw: "[[https://e.org][label]] after", format: "org", start: firstOf("label", 2), end: firstOf(" after", 3) },
  { name: "across an emphasis delimiter", raw: "**a**b", start: firstOf("a", 0), end: lastOf("b") },
  { name: "an escaped asterisk", raw: "a \\*b", start: firstOf("*", 0), end: firstOf("*", 1) },
  { name: "part of inline HTML", raw: "<b>hello world</b>", start: firstOf("hello world", 3), end: firstOf("hello world", 7), selected: "lo w", shows: "<b>hello world</b>" },
  { name: "text beside an image in inline HTML", raw: "a <b><img src=\"x.png\">hello world</b> c", start: firstOf("hello world", 0), end: firstOf("hello world", 11) },
  { name: "text beside a hidden comment", raw: "<b><!-- keeper -->hello</b>", start: firstOf("hello", 1), end: firstOf("hello", 4) },
  { name: "the prose before a rendered dash", raw: "a -- b", start: firstOf("a", 0), end: firstOf("a", 2) },
  // Ordinary text: still opens, still writes nothing.
  { name: "plain prose", raw: "hello world", start: firstOf("world", 0), end: firstOf("world", 5), selected: "world", shows: "world" },
  { name: "a highlight", raw: "a ==x== b", start: firstOf("x", 0), end: firstOf("x", 1) },
  { name: "a heading", raw: "# Head line", start: firstOf("Head", 1), end: firstOf("Head", 4) },
  { name: "a quote", raw: "> quoted words", start: firstOf("quoted", 0), end: firstOf("quoted", 6) },
];

describe.each<EditorKind>(["textarea", "cm6"])("%s editor", kind => {
  useEditorKind(kind);

  describe("read-view Delete opens the editor", () => {
    describe.each(["Delete", "Backspace"])("%s", key => {
      it.each(rows)("$name: writes nothing and opens the editor on the clicked range", row => {
        setTypographyMode("render");
        mount({ a: row.raw }, row.format);
        const setRaw = vi.spyOn(documentApi, "setRaw");
        const start = row.start("a"), end = row.end("a");
        const expected = clickPair("a", start, end, row.format);
        const sel = select(start, end);
        if (row.selected) expect(sel.toString()).toBe(row.selected);
        const e = press(key);
        expect(e.defaultPrevented).toBe(true);
        expect(doc.byId.a.raw).toBe(row.raw);
        expect(setRaw).not.toHaveBeenCalled();
        expect(undo()).toBe(false);
        expect(editingId()).toBe("a");
        const editor = editorIn(document.querySelector('[data-block-id="a"]')!);
        expect(editor.kind).toBe(kind);
        expect(editor.selection()).toEqual(expected);
        if (row.shows) expect(editor.value().slice(...expected)).toBe(row.shows);
        expect(renderedSelectionGone()).toBe(true);
      });
    });

    it("keeps the selection's direction", () => {
      mount({ a: "hello world" });
      select(firstOf("world")("a"), firstOf("world", 5)("a"), true);
      press("Delete");
      const editor = editorIn(document.querySelector('[data-block-id="a"]')!);
      expect(editor.selection()).toEqual([6, 11]);
      expect(editor.direction()).toBe("backward");
    });

    it("a code card opens in body mode with the body range selected, as a body click does", () => {
      const raw = "```\nabcdef\n```";
      mount({ a: raw });
      const body = wrapperOf("a").querySelector(".code-text")!.firstChild as Text;
      select([body, 1], [body, 2]);
      press("Delete");
      expect(doc.byId.a.raw).toBe(raw);
      expect(editingId()).toBe("a");
      const editor = editorIn(document.querySelector('[data-block-id="a"]')!);
      expect(editor.value()).toBe("abcdef");
      expect(editor.selection()).toEqual([1, 2]);
    });

    it("opens the selecting instance when the block is shown twice", () => {
      const [main, side] = mount({ a: "hello world" }, "md", ["main", "sidebar:x"]);
      startEditing("a", 0);
      endEdit("blur");
      // Select in the second surface's rendering.
      const t = text("a", "world", side);
      select([t, 6], [t, 11]);
      press("Delete");
      expect(editingId()).toBe("a");
      expect(maybeEditorIn(main)).toBeNull();
      expect(editorIn(side).selection()).toEqual([6, 11]);
      expect(doc.byId.a.raw).toBe("hello world");
    });

    it.each(["entry", "exit"])("a refused %s guard keeps the rendered selection and writes nothing", which => {
      mount({ a: "abc", other: "other" });
      if (which === "exit") {
        startEditing("other", 0);
        cleanup.push(registerEditExitGuard(() => false));
      } else cleanup.push(registerEditEntryGuard(() => false));
      const t = text("a", "abc");
      const sel = select([t, 1], [t, 2]);
      const e = press("Delete");
      expect(e.defaultPrevented).toBe(true);
      expect(doc.byId.a.raw).toBe("abc");
      expect(editingId()).not.toBe("a");
      expect(sel.toString()).toBe("b");
    });

    it.each([
      ["a calc block", "```calc\n1 + 2\n```", "1 + 2"],
      ["an annotation block", "Highlighted text\nls-type:: annotation\nhl-page:: 2\nhl-color:: yellow", "Highlighted"],
    ])("refuses %s as before", (_name, raw, needle) => {
      mount({ a: raw });
      let t: Text;
      try { t = text("a", needle); } catch { return; } // nothing selectable is also a refusal
      select([t, t.data.indexOf(needle)], [t, t.data.indexOf(needle) + 2]);
      const e = press("Delete");
      expect(e.defaultPrevented).toBe(false);
      expect(doc.byId.a.raw).toBe(raw);
      expect(editingId()).toBeNull();
    });

    it("refuses behind an open modal as before", async () => {
      mount({ a: "abc" });
      const host = document.createElement("div");
      document.body.appendChild(host);
      cleanup.push(render(() => <Lightbox />, host));
      setLightbox("blob:image");
      cleanup.push(() => setLightbox(null));
      await Promise.resolve();
      expect(document.querySelector(".lightbox-overlay")).not.toBeNull();
      const t = text("a", "abc");
      select([t, 1], [t, 2]);
      press("Delete");
      expect(doc.byId.a.raw).toBe("abc");
      expect(editingId()).toBeNull();
    });

    it("a note's own modal-overlay class is not a modal: the handoff opens the editor and writes nothing", () => {
      mount({ a: "hello world", note: "<span class=\"modal-overlay\">x</span>" });
      expect(document.querySelector('[data-block-id="note"] .modal-overlay')).not.toBeNull();
      const setRaw = vi.spyOn(documentApi, "setRaw");
      const start = firstOf("world")("a"), end = firstOf("world", 5)("a");
      const expected = clickPair("a", start, end);
      select(start, end);
      expect(press("Delete").defaultPrevented).toBe(true);
      expect(doc.byId.a.raw).toBe("hello world");
      expect(setRaw).not.toHaveBeenCalled();
      expect(editingId()).toBe("a");
      expect(editorIn(document.querySelector('[data-block-id="a"]')!).selection()).toEqual(expected);
    });

    it("with no modal open, the handoff reads the open-modal registry once and queries no window for one", () => {
      mount({ a: "hello world" });
      const reads = vi.spyOn(layers, "modalOpen");
      const windows = vi.spyOn(realm, "queryAllWindows");
      select(firstOf("world")("a"), firstOf("world", 5)("a"));
      expect(press("Delete").defaultPrevented).toBe(true);
      expect(editingId()).toBe("a");
      expect(reads).toHaveBeenCalledTimes(1);
      expect(windows).not.toHaveBeenCalled();
    });
  });

  describe("a held Delete or Backspace never reaches the editor it opened", () => {
    function openOnB(key: string) {
      mount({ a: "abc" });
      const t = text("a", "abc");
      select([t, 1], [t, 2]);
      press(key);
      const editor = editorIn(document.querySelector('[data-block-id="a"]')!);
      const reached: KeyboardEvent[] = [];
      const listen = (e: Event) => reached.push(e as KeyboardEvent);
      editor.element.addEventListener("keydown", listen);
      cleanup.push(() => editor.element.removeEventListener("keydown", listen));
      return { editor, reached };
    }

    it.each(["Delete", "Backspace"])("%s auto-repeat is swallowed until keyup, then the next press is the editor's", key => {
      const { editor, reached } = openOnB(key);
      for (let i = 0; i < 3; i++) expect(editor.keydown({ key, repeat: true }).defaultPrevented).toBe(true);
      expect(reached).toHaveLength(0);
      expect(doc.byId.a.raw).toBe("abc");
      expect(undo()).toBe(false);
      expect(editor.selection()).toEqual([1, 2]);
      editor.keyup({ key });
      editor.keydown({ key });
      expect(reached.map(e => e.key)).toEqual([key]);
    });

    it("a window blur ends the hold", () => {
      const { editor, reached } = openOnB("Delete");
      editor.keydown({ key: "Delete", repeat: true });
      window.dispatchEvent(new FocusEvent("blur"));
      expect(doc.byId.a.raw).toBe("abc");
      expect(reached).toHaveLength(0);
      editor.keydown({ key: "Delete", repeat: true });
      expect(reached).toHaveLength(1);
    });

    it("another key ends the hold", () => {
      const { editor, reached } = openOnB("Delete");
      editor.keydown({ key: "ArrowLeft" });
      editor.keydown({ key: "Delete", repeat: true });
      expect(reached.map(e => e.key)).toEqual(["ArrowLeft", "Delete"]);
    });
  });
});
