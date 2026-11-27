// A query's results, its "No results" line and its controls are generated: they
// are not the text of the block that holds the query. Selecting them and
// pressing Delete or Backspace must change nothing. Before this rule, a query
// inside a heading, quote or callout was deleted from its block: the selection
// end sat in output that carries no source offsets, so the Delete handler
// snapped it to an edge of the construct around the query.
import { afterEach, beforeAll, describe, expect, it, vi } from "vitest";
import { render } from "solid-js/web";
import { initParser } from "../render/parse";
import { backend } from "../backend";
import * as queryMacro from "../editor/queryMacro";
import { resetStore } from "../document";
import { doc, setDoc, type FeedPage, type Node as StoreNode } from "../document/model";
import { editingId } from "../editorController";
import { installKeybindings } from "../keybindings";
import { resetSharedQueryResultsForTests } from "../queryResultCache";
import { blockRunResult } from "../tests/queryReadingsTestkit";
import { Block } from "./Block";
import { editorIn } from "../tests/editorDriver";
import { LinkDepthContext, MAX_DEPTH_OF_LINKS } from "./linkDepth";

beforeAll(async () => {
  await initParser();
});

afterEach(() => {
  window.getSelection()?.removeAllRanges();
  vi.restoreAllMocks();
  resetSharedQueryResultsForTests();
  resetStore();
  localStorage.clear();
  document.body.innerHTML = "";
});

const QUERY = "{{query (task TODO)}}";

/** Mount one block holding `raw` on a writable page; its query answers nothing. */
async function mountQueryBlock(
  raw: string,
  notice?: { depth?: number; fail?: boolean; awaited: string },
): Promise<{ dispose: () => void; output: HTMLElement }> {
  const page: FeedPage = {
    name: "Host", kind: "page", title: "Host", preBlock: null,
    roots: ["host"], format: "md", readOnly: false, guide: false,
  };
  const node: StoreNode = { id: "host", raw, collapsed: false, parent: null, page: "Host", children: [] };
  setDoc({ byId: { host: node }, pages: [page], feed: ["Host"], loaded: true });
  vi.spyOn(backend(), "queryRun").mockResolvedValue(blockRunResult([]));
  if (notice?.fail) {
    // The query component alone asks about the bare macro; every host here has
    // more text around it, so only the component's own render throws.
    const real = queryMacro.queryMacroExtent;
    vi.spyOn(queryMacro, "queryMacroExtent").mockImplementation((text, ...rest) => {
      if (text === QUERY) throw new Error("the query could not be drawn");
      return real(text, ...rest);
    });
  }
  const awaited = notice?.awaited ?? ".query-empty";
  const root = document.createElement("div");
  document.body.appendChild(root);
  const disposeBlock = render(() => (
    <LinkDepthContext.Provider value={notice?.depth ?? 0}><Block id="host" /></LinkDepthContext.Provider>
  ), root);
  const disposeKeys = installKeybindings();
  await vi.waitFor(() => expect(root.querySelector(awaited)).not.toBeNull());
  return {
    dispose: () => { disposeKeys(); disposeBlock(); },
    output: root.querySelector(awaited) as HTMLElement,
  };
}

function firstText(el: Element): Text {
  const walker = document.createTreeWalker(el, NodeFilter.SHOW_TEXT);
  for (let n = walker.nextNode(); n; n = walker.nextNode()) {
    if ((n.textContent ?? "").trim().length > 1) return n as Text;
  }
  throw new Error("no text in the query's output");
}

function lastText(el: Element): Text {
  const walker = document.createTreeWalker(el, NodeFilter.SHOW_TEXT);
  let last: Text | null = null;
  for (let n = walker.nextNode(); n; n = walker.nextNode()) {
    if ((n.textContent ?? "").trim().length > 0) last = n as Text;
  }
  if (!last) throw new Error("no text in the query's output");
  return last;
}

function select(start: [Node, number], end: [Node, number]): void {
  const range = document.createRange();
  range.setStart(...start);
  range.setEnd(...end);
  const sel = window.getSelection();
  sel?.removeAllRanges();
  sel?.addRange(range);
}

function keydown(key: string): void {
  window.dispatchEvent(new KeyboardEvent("keydown", { key, bubbles: true, cancelable: true }));
}

const raw = () => doc.byId.host.raw;

describe("Delete over a selection in a query's generated output", () => {
  const hosts: [string, string][] = [
    ["a heading", `# ${QUERY}`],
    ["a heading:: property", `${QUERY}\nheading:: 2`],
    ["a heading:: property after text", `intro ${QUERY}\nheading:: 2`],
    ["a heading on a later line", `intro\n# ${QUERY}`],
    ["a quote", `> ${QUERY}`],
    ["a callout", `> [!NOTE]\n> ${QUERY}`],
    ["a paragraph", `before ${QUERY} after`],
  ];

  it.each(hosts)("keeps the query in %s when text of its output is selected", async (_name, source) => {
    const m = await mountQueryBlock(source);
    try {
      const queryRoot = m.output.closest(".query-block") as HTMLElement;
      const first = firstText(queryRoot);
      const last = lastText(queryRoot);
      // A few characters, then everything the query drew: the second range
      // ends in the far half of the construct around the query, which is where
      // the unguarded handler snapped to that construct's other edge.
      const ranges: [[Node, number], [Node, number]][] = [
        [[first, 0], [first, 2]],
        [[first, 0], [last, last.data.length]],
      ];
      for (const key of ["Delete", "Backspace"]) {
        for (const [start, end] of ranges) {
          select(start, end);
          keydown(key);
          expect(raw()).toBe(source);
          expect(editingId()).toBeNull();
        }
      }
    } finally {
      m.dispose();
    }
  });

  it("keeps a heading whose selection ends were normalised to the edges of the query's element", async () => {
    const source = `# ${QUERY}`;
    const m = await mountQueryBlock(source);
    try {
      const queryRoot = m.output.closest(".query-block") as HTMLElement;
      let child: Element = queryRoot;
      while (child.parentElement && !child.parentElement.hasAttribute("data-so")) child = child.parentElement;
      const span = child.parentElement as HTMLElement;
      const index = Array.prototype.indexOf.call(span.childNodes, child);
      expect(index).toBeGreaterThanOrEqual(0);
      select([span, index], [span, index + 1]);
      keydown("Delete");
      expect(raw()).toBe(source);
      expect(editingId()).toBeNull();
    } finally {
      m.dispose();
    }
  });

  it("marks the query's output as another source, so no reader maps it onto the block's text", async () => {
    const m = await mountQueryBlock(`# ${QUERY}`);
    try {
      expect(m.output.closest(".query-block")?.getAttribute("data-foreign-source")).toBe("query");
    } finally {
      m.dispose();
    }
  });

  // A notice drawn in place of a query is generated text too.
  const notices: [string, { depth?: number; fail?: boolean; awaited: string }][] = [
    ["the embed-depth warning", { depth: MAX_DEPTH_OF_LINKS + 1, awaited: ".link-depth-warning" }],
    ["a failed region's notice", { fail: true, awaited: ".region-failure" }],
  ];
  const noticeHosts: [string, string][] = [
    ["a heading", `# ${QUERY}`],
    ["a heading:: property after text", `intro ${QUERY}\nheading:: 2`],
    ["a quote", `> ${QUERY}`],
    ["a callout", `> [!NOTE]\n> ${QUERY}`],
  ];
  describe.each(notices)("when %s stands in for the query", (_notice, how) => {
    it.each(noticeHosts)("keeps the query in %s", async (_name, source) => {
      const m = await mountQueryBlock(source, how);
      try {
        const first = firstText(m.output);
        const last = lastText(m.output);
        const ranges: [[Node, number], [Node, number]][] = [
          [[first, 0], [first, 2]],
          [[first, 0], [last, last.data.length]],
        ];
        for (const key of ["Delete", "Backspace"]) {
          for (const [start, end] of ranges) {
            select(start, end);
            keydown(key);
            expect(raw()).toBe(source);
            expect(editingId()).toBeNull();
          }
        }
      } finally {
        m.dispose();
      }
    });
  });

  // A callout's title has no source position of its own: its ends map to an
  // edge of the whole callout, chosen by how far into the callout's rendered
  // text they lie, and that text counts what the query drew. With a short
  // result the end of the title is past the middle, so the range ran to the
  // callout's far edge and took the query with it.
  it("keeps a query in a callout when the selection lies in the callout's title", async () => {
    const source = `> [!NOTE] A title long enough to pass the middle\n> ${QUERY}`;
    const m = await mountQueryBlock(source);
    try {
      const wrapper = document.querySelector('[data-block-id="host"] > .block-main .block-content-wrapper') as HTMLElement;
      const walker = document.createTreeWalker(wrapper, NodeFilter.SHOW_TEXT);
      let title: Text | null = null;
      for (let n = walker.nextNode(); n; n = walker.nextNode()) {
        if ((n.textContent ?? "").includes("long enough")) { title = n as Text; break; }
      }
      expect(title).toBeTruthy();
      for (const key of ["Delete", "Backspace"]) {
        select([title!, title!.data.indexOf("A title")], [title!, title!.data.length]);
        keydown(key);
        expect(raw()).toBe(source);
        expect(editingId()).toBeNull();
      }
    } finally {
      m.dispose();
    }
  });

  // Both ends lie in the block's own text and the user visibly selected the
  // whole query between them: the editor opens with that range selected
  //, and nothing is written.
  it.each([
    ["a paragraph", `abc ${QUERY} xyz`, "bc " + QUERY + " xy"],
    ["a heading", `# abc ${QUERY} xyz`, "bc " + QUERY + " xy"],
  ])("opens the editor on a selection that spans the whole query in %s", async (_name, source, expected) => {
    const m = await mountQueryBlock(source);
    try {
      const wrapper = document.querySelector('[data-block-id="host"] > .block-main .block-content-wrapper') as HTMLElement;
      const walker = document.createTreeWalker(wrapper, NodeFilter.SHOW_TEXT);
      let before: Text | null = null;
      let after: Text | null = null;
      for (let n = walker.nextNode(); n; n = walker.nextNode()) {
        if (!before && (n.textContent ?? "").includes("abc")) before = n as Text;
        if ((n.textContent ?? "").includes("xyz")) after = n as Text;
      }
      expect(before && after).toBeTruthy();
      select([before!, before!.data.indexOf("abc") + 1], [after!, after!.data.indexOf("xyz") + 2]);
      keydown("Delete");
      expect(raw()).toBe(source);
      expect(editingId()).toBe("host");
      const editor = editorIn(document.querySelector('[data-block-id="host"]')!);
      const [s, e] = editor.selection();
      expect(editor.value().slice(s, e)).toBe(expected);
    } finally {
      m.dispose();
    }
  });
});
