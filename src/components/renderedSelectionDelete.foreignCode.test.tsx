// Which authored code fence owns a point in a rendered code card. A block's
// rendered content can draw code cards that are not its own: a page or block
// embed, a query's results, a block reference's target, a user macro's
// expansion. Only the cards the block renders from its own source are its
// fences, so a foreign card before an authored one must not shift which fence
// a click or a read-view Delete opens, and a point inside a foreign card is not
// the host's code at all.
import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from "vitest";
import { For } from "solid-js";
import { render } from "solid-js/web";
import { backend } from "../backend";
import { initParser } from "../render/parse";
import { loadFeed, pageByName, resetStore } from "../document";
import { doc } from "../document/model";
import { setGraphMeta } from "../graphSession";
import { editingId, endEdit } from "../editorController";
import { installKeybindings } from "../keybindings";
import { resetSharedQueryResultsForTests } from "../queryResultCache";
import { BUILTIN_HIDE, splitProps } from "../editor/properties";
import { FOREIGN_SOURCE_ATTR } from "../render/spans";
import { backendReadsQueries, blockRunResult } from "../tests/queryReadingsTestkit";
import { editorIn, type EditorKind } from "../tests/editorDriver";
import { useEditorKind } from "../tests/editorKind";
import type { BlockDto, PageDto } from "../types";
import { Block } from "./Block";
import { renderedClickOffset } from "./blockGestures";

class AllNearObserver implements IntersectionObserver {
  readonly root = null;
  readonly rootMargin = "0px";
  readonly thresholds = [0];
  constructor(private readonly callback: IntersectionObserverCallback) {}
  disconnect(): void {}
  takeRecords(): IntersectionObserverEntry[] { return []; }
  unobserve(): void {}
  observe(target: Element): void {
    this.callback([{ isIntersecting: true, target } as IntersectionObserverEntry], this);
  }
}

beforeAll(initParser);
beforeEach(() => vi.stubGlobal("IntersectionObserver", AllNearObserver));
let cleanup: (() => void)[] = [];
afterEach(() => {
  for (const f of cleanup.reverse()) f();
  cleanup = [];
  endEdit("page-navigation");
  window.getSelection()?.removeAllRanges();
  resetSharedQueryResultsForTests();
  setGraphMeta(null);
  resetStore();
  vi.restoreAllMocks();
  vi.unstubAllGlobals();
  document.body.replaceChildren();
});

const leaf = (id: string, raw: string): BlockDto => ({ id, raw, collapsed: false, children: [] });
const page = (name: string, blocks: BlockDto[]): PageDto => ({ name, title: name, kind: "page", pre_block: null, blocks });

const FOREIGN = "```\nzzzzzz\n```";
const ONE = "```\nabcdef\n```";
const TWO = "```\nabcdef\n```\n```\nuvwxyz\n```";

/** The host block's text before its authored fences, each drawing one foreign code card. */
const sources: { name: string; lead: string; card: string }[] = [
  { name: "a page embed", lead: "{{embed [[Foreign]]}}", card: ".embed-block" },
  { name: "a block embed", lead: "{{embed ((target))}}", card: ".embed-block" },
  { name: "a List query result", lead: "{{query (task TODO)}}", card: ".query-block" },
  { name: "a bare block reference", lead: "((target))", card: ".block-ref" },
  { name: "a user macro", lead: "{{code}}", card: ".macro-blocks" },
];

/** Mount block `host` holding `raw` with every foreign source resolvable. */
async function mountHost(raw: string, foreignSelector: string): Promise<HTMLElement> {
  const target = leaf("target", `${FOREIGN}\nid:: target`);
  const foreign = page("Foreign", [target]);
  loadFeed([page("Host", [leaf("host", raw)]), foreign]);
  setGraphMeta({ macros: { code: FOREIGN } } as never);
  vi.spyOn(backend(), "resolveBlocks").mockImplementation(async ids =>
    ids.map(id => (id === "target" ? { page: "Foreign", kind: "page", blocks: [{ ...target, children: [] }] } : null)));
  vi.spyOn(backend(), "getPage").mockResolvedValue(foreign as never);
  backendReadsQueries({ "(task TODO)": { form: "(task TODO)", opts: "" } });
  vi.spyOn(backend(), "queryRun").mockResolvedValue(blockRunResult([{ page: "Foreign", kind: "page", blocks: [{ ...target, children: [] }] }]));
  const root = document.createElement("div");
  document.body.append(root);
  cleanup.push(render(() => <For each={pageByName("Host")?.roots}>{id => <Block id={id} />}</For>, root));
  cleanup.push(installKeybindings());
  await vi.waitFor(() => expect(content().querySelector(`${foreignSelector} pre.code-block .code-text`)?.textContent).toContain("zzzzzz"));
  await vi.waitFor(() => expect(authoredText("abcdef")).not.toBeNull());
  return root;
}

const hostRow = () => document.querySelector('.ls-block[data-block-id="host"]')!;
const content = () => hostRow().querySelector(":scope > .block-main .block-content")!;

/** The text node of a host card whose text is `needle`, outside every foreign source. */
function authoredText(needle: string): Text | null {
  for (const code of content().querySelectorAll("pre.code-block .code-text")) {
    if (code.closest(`[${FOREIGN_SOURCE_ATTR}]`) || code.textContent?.trim() !== needle) continue;
    const walker = document.createTreeWalker(code, NodeFilter.SHOW_TEXT);
    for (let n = walker.nextNode(); n; n = walker.nextNode()) if ((n as Text).data.includes(needle)) return n as Text;
  }
  return null;
}

/** The text node of the foreign card drawn under `selector`. */
function foreignText(selector: string): Text {
  const code = content().querySelector(`${selector} pre.code-block .code-text`)!;
  const walker = document.createTreeWalker(code, NodeFilter.SHOW_TEXT);
  for (let n = walker.nextNode(); n; n = walker.nextNode()) if ((n as Text).data.includes("z")) return n as Text;
  throw new Error("no foreign code text");
}

function selectIn(t: Text, from: number, to: number): void {
  const sel = window.getSelection()!;
  sel.removeAllRanges();
  sel.setBaseAndExtent(t, from, t, to);
}

function press(key: string): KeyboardEvent {
  const e = new KeyboardEvent("keydown", { key, bubbles: true, cancelable: true });
  window.dispatchEvent(e);
  return e;
}

/** A click on `t` at `offset`, as `renderedClickOffset` reads it. */
function clickAt(t: Text, offset: number): number | null {
  const range = document.createRange();
  range.setStart(t, offset);
  range.collapse(true);
  (document as unknown as { caretRangeFromPoint: () => Range }).caretRangeFromPoint = () => range;
  cleanup.push(() => { delete (document as unknown as { caretRangeFromPoint?: unknown }).caretRangeFromPoint; });
  const e = new MouseEvent("click", { clientX: 1, clientY: 1 });
  Object.defineProperty(e, "target", { value: t.parentElement });
  return renderedClickOffset(content() as HTMLElement, doc.byId.host.raw, "md", e);
}

describe.each<EditorKind>(["textarea", "cm6"])("%s editor", kind => {
  useEditorKind(kind);

  describe.each(sources)("after $name", source => {
    it.each([["one", ONE], ["two", TWO]])("Delete in the authored card opens that fence on the selected text (%s authored fences)", async (_n, fences) => {
      const raw = `${source.lead}\n${fences}`;
      await mountHost(raw, source.card);
      for (const key of ["Delete", "Backspace"]) {
        endEdit("blur");
        selectIn(authoredText("abcdef")!, 1, 2);
        expect(press(key).defaultPrevented).toBe(true);
        expect(doc.byId.host.raw).toBe(raw);
        expect(editingId()).toBe("host");
        const editor = editorIn(hostRow());
        expect(editor.value()).toBe("abcdef");
        expect(editor.selection()).toEqual([1, 2]);
      }
    });

    it("a click in the authored card places the caret in that fence's body", async () => {
      const raw = `${source.lead}\n${TWO}`;
      await mountHost(raw, source.card);
      const visible = splitProps(raw, BUILTIN_HIDE, "md").visible;
      expect(clickAt(authoredText("abcdef")!, 1)).toBe(visible.indexOf("abcdef") + 1);
      expect(clickAt(authoredText("uvwxyz")!, 1)).toBe(visible.indexOf("uvwxyz") + 1);
    });

    it("a selection inside the foreign card does not open the host", async () => {
      const raw = `${source.lead}\n${TWO}`;
      await mountHost(raw, source.card);
      const t = foreignText(source.card);
      const z = t.data.indexOf("z");
      for (const key of ["Delete", "Backspace"]) {
        for (const [from, to] of [[1, 2], [0, 6]]) {
          endEdit("blur");
          selectIn(t, z + from, z + to);
          press(key);
          expect(doc.byId.host.raw).toBe(raw);
          expect(editingId()).not.toBe("host");
        }
      }
    });

    it("a click inside the foreign card is not a click in an authored fence's body", async () => {
      const raw = `${source.lead}\n${TWO}`;
      await mountHost(raw, source.card);
      const t = foreignText(source.card);
      const visible = splitProps(raw, BUILTIN_HIDE, "md").visible;
      const bodies = ["abcdef", "uvwxyz"].map(b => [visible.indexOf(b), visible.indexOf(b) + b.length]);
      for (const at of [1, 2, 5]) {
        const got = clickAt(t, t.data.indexOf("z") + at);
        expect(bodies.some(([from, to]) => got !== null && got >= from && got <= to)).toBe(false);
      }
    });
  });
});

describe("the card lookup inspects only the host's own cards", () => {
  /** A host content holding `foreign` foreign cards in one embed, then one authored card. */
  function contentWith(foreign: number): { root: HTMLElement; body: Text } {
    const root = document.createElement("div");
    root.className = "block-content";
    const card = (text: string) => {
      const pre = document.createElement("pre");
      pre.className = "code-block";
      const code = document.createElement("code");
      const textEl = document.createElement("span");
      textEl.className = "code-text";
      textEl.textContent = text;
      code.append(textEl);
      pre.append(code);
      return pre;
    };
    const embed = document.createElement("div");
    embed.setAttribute(FOREIGN_SOURCE_ATTR, "embed");
    for (let i = 0; i < foreign; i++) embed.append(card("zzzzzz"));
    root.append(embed);
    const own = card("abcdef");
    root.append(own);
    document.body.append(root);
    return { root, body: own.querySelector(".code-text")!.firstChild as Text };
  }

  it("does constant work for 0, 10, 100 and 1,000 foreign cards, and inspects none of them", () => {
    const raw = `{{embed [[Foreign]]}}\n${ONE}`;
    const visible = splitProps(raw, BUILTIN_HIDE, "md").visible;
    const counts: number[] = [];
    for (const n of [0, 10, 100, 1000]) {
      const { root, body } = contentWith(n);
      const seen: Element[] = [];
      const spies = (["hasAttribute", "contains", "matches", "closest"] as const).map(method => {
        const real = Element.prototype[method] as (...args: unknown[]) => unknown;
        return vi.spyOn(Element.prototype, method).mockImplementation(function (this: Element, ...args: unknown[]) {
          seen.push(this);
          return real.apply(this, args);
        } as never);
      });
      const range = document.createRange();
      range.setStart(body, 1);
      (document as unknown as { caretRangeFromPoint: () => Range }).caretRangeFromPoint = () => range;
      const e = new MouseEvent("click");
      Object.defineProperty(e, "target", { value: body.parentElement });
      const got = renderedClickOffset(root, raw, "md", e);
      for (const spy of spies) spy.mockRestore();
      delete (document as unknown as { caretRangeFromPoint?: unknown }).caretRangeFromPoint;
      expect(seen.filter(el => el.parentElement?.closest(`[${FOREIGN_SOURCE_ATTR}]`)).length).toBe(0);
      expect(got).toBe(visible.indexOf("abcdef") + 1);
      counts.push(seen.length);
      root.remove();
    }
    expect(new Set(counts).size).toBe(1);
  });
});

/** A host whose block draws `zzzzzz` from another source, under the element `marker` matches. */
type ForeignCase = { name: string; host: string; hostFmt: "md" | "org"; target: string; targetFmt: "md" | "org"; marker: string };
const PROSE_AFTER = "ninechars";
const foreignCases: ForeignCase[] = [
  { name: "a block reference whose target has prose after its fence", host: "((target))", hostFmt: "md",
    target: `${FOREIGN}\n${PROSE_AFTER}`, targetFmt: "md", marker: ".block-ref" },
  { name: "a code-only macro in a Note callout", host: "> [!NOTE]\n> {{code}}", hostFmt: "md",
    target: "t", targetFmt: "md", marker: ".macro-blocks" },
  { name: "a code-only macro in a Tip callout", host: "> [!TIP]\n> {{code}}", hostFmt: "md",
    target: "t", targetFmt: "md", marker: ".macro-blocks" },
  { name: "a block reference to an Org source card", host: "((target))", hostFmt: "org",
    target: `#+BEGIN_SRC\nzzzzzz\n#+END_SRC\n${PROSE_AFTER}`, targetFmt: "org", marker: ".block-ref" },
  { name: "a block reference to an Org example card", host: "((target))", hostFmt: "org",
    target: `#+BEGIN_EXAMPLE\nzzzzzz\n#+END_EXAMPLE\n${PROSE_AFTER}`, targetFmt: "org", marker: ".block-ref" },
  { name: "prose before a block reference to a code card", host: "before ((target))", hostFmt: "md",
    target: `${FOREIGN}\n${PROSE_AFTER}`, targetFmt: "md", marker: ".block-ref" },
];

async function mountForeign(c: ForeignCase): Promise<void> {
  const target = leaf("target", c.target);
  const foreign: PageDto = { ...page("Foreign", [target]), format: c.targetFmt };
  loadFeed([{ ...page("Host", [leaf("host", c.host)]), format: c.hostFmt }, foreign]);
  setGraphMeta({ macros: { code: FOREIGN } } as never);
  vi.spyOn(backend(), "resolveBlocks").mockImplementation(async ids =>
    ids.map(id => (id === "target" ? { page: "Foreign", kind: "page", blocks: [{ ...target, children: [] }] } : null)));
  vi.spyOn(backend(), "getPage").mockResolvedValue(foreign as never);
  const root = document.createElement("div");
  document.body.append(root);
  cleanup.push(render(() => <For each={pageByName("Host")?.roots}>{id => <Block id={id} />}</For>, root));
  cleanup.push(installKeybindings());
  await vi.waitFor(() => expect(textUnder(c.marker, "zzzzzz")).not.toBeNull());
}

/** The first text node holding `needle` below an element `selector` matches in the host's content (or anywhere in it, for null). */
function textUnder(selector: string | null, needle: string): Text | null {
  const scope = selector ? content().querySelector(selector) : content();
  if (!scope) return null;
  const walker = document.createTreeWalker(scope, NodeFilter.SHOW_TEXT);
  for (let n = walker.nextNode(); n; n = walker.nextNode()) if ((n as Text).data.includes(needle)) return n as Text;
  return null;
}

describe.each<EditorKind>(["textarea", "cm6"])("%s editor: a selection reaching into another source", kind => {
  useEditorKind(kind);

  it.each(foreignCases.map(c => [c.name, c] as const))("does nothing for %s, as before the handoff", async (_n, c) => {
    await mountForeign(c);
    const z = textUnder(c.marker, "zzzzzz")!;
    const at = z.data.indexOf("zzzzzz");
    const before = textUnder(null, "before");
    const ranges: [Text, number, Text, number][] = [[z, at + 1, z, at + 2], [z, at, z, at + 6]];
    // From the card into the target's prose after it: the two ends fall either side of the reference's middle.
    const after = textUnder(c.marker, PROSE_AFTER);
    if (after) ranges.push([z, at + 1, after, after.data.indexOf(PROSE_AFTER) + PROSE_AFTER.length]);
    if (c.target.includes(PROSE_AFTER)) expect(after).not.toBeNull();
    if (before && !before.parentElement!.closest(`[${FOREIGN_SOURCE_ATTR}]`)) ranges.push([before, 1, z, at + 3], [z, at + 3, before, 1]);
    for (const key of ["Delete", "Backspace"]) {
      for (const [a, ao, f, fo] of ranges) {
        endEdit("blur");
        window.getSelection()!.setBaseAndExtent(a, ao, f, fo);
        expect(press(key).defaultPrevented).toBe(false);
        expect(doc.byId.host.raw).toBe(c.host);
        expect(editingId()).not.toBe("host");
      }
    }
    // Every mixed case ran: two in-card ranges, the card-to-prose range, and both host-to-card directions.
    if (c.host.startsWith("before")) expect(ranges.length).toBe(5);
  });
});

describe("a click on another source's code card", () => {
  /** Host content: `spans` positioned words, then a macro expansion drawing one code card. */
  function contentWith(spans: number): { root: HTMLElement; code: Text } {
    const root = document.createElement("div");
    root.className = "block-content";
    for (let i = 0; i < spans; i++) {
      const s = document.createElement("span");
      s.setAttribute("data-so", "2");
      s.setAttribute("data-se", "3");
      s.textContent = "w";
      root.append(s);
    }
    const macro = document.createElement("div");
    macro.className = "macro-blocks";
    macro.setAttribute(FOREIGN_SOURCE_ATTR, "macro");
    macro.innerHTML = `<pre class="code-block"><code><span class="code-text">zzzzzz</span></code></pre>`;
    root.append(macro);
    document.body.append(root);
    return { root, code: macro.querySelector(".code-text")!.firstChild as Text };
  }

  it("opens the block at its end, in source, without reading layout, at 0, 10, 100 and 1,000 spans", () => {
    const raw = "w {{code}}";
    const rectReads: number[] = [];
    const stubs: [object, string, unknown][] = [];
    let reads = 0;
    const rect = () => { reads++; return { x: 0, y: 0, width: 0, height: 0, top: 0, left: 0, right: 0, bottom: 0 } as DOMRect; };
    const rects = () => { reads++; return [] as unknown as DOMRectList; };
    for (const [proto, name, fn] of [[Element.prototype, "getBoundingClientRect", rect], [Element.prototype, "getClientRects", rects],
      [Range.prototype, "getBoundingClientRect", rect], [Range.prototype, "getClientRects", rects]] as const) {
      stubs.push([proto, name, Object.getOwnPropertyDescriptor(proto, name)]);
      Object.defineProperty(proto, name, { value: fn, configurable: true, writable: true });
    }
    try {
      for (const n of [0, 10, 100, 1000]) {
        const { root, code } = contentWith(n);
        const range = document.createRange();
        range.setStart(code, 2);
        (document as unknown as { caretRangeFromPoint: () => Range }).caretRangeFromPoint = () => range;
        const e = new MouseEvent("click", { clientX: 1, clientY: 1 });
        Object.defineProperty(e, "target", { value: code.parentElement });
        reads = 0;
        expect(renderedClickOffset(root, raw, "md", e)).toBe(raw.length);
        rectReads.push(reads);
        delete (document as unknown as { caretRangeFromPoint?: unknown }).caretRangeFromPoint;
        root.remove();
      }
    } finally {
      for (const [proto, name, desc] of stubs) {
        if (desc) Object.defineProperty(proto, name, desc as PropertyDescriptor);
        else delete (proto as Record<string, unknown>)[name];
      }
    }
    expect(rectReads).toEqual([0, 0, 0, 0]);
  });
});
