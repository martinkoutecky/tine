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
