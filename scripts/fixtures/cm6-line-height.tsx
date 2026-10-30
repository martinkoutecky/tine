import { render } from "solid-js/web";
import { For } from "solid-js";
import { initParser } from "../../src/render/parse";
import { loadSingle } from "../../src/document/workingSet";
import { pageByName } from "../../src/document/model";
import { Block } from "../../src/components/Block";
import { startEditing, endEdit } from "../../src/editorController";
import { setCm6EditorEnabled } from "../../src/editor/cm6Setting";
import "../../src/styles/inter.css";
import "../../src/styles/theme.css";
import "../../src/styles/app.css";
import "../../src/styles/ls-shim.css";

// One block per construct whose line box could differ between the rendered body and the editor.
export const CASES: Record<string, string> = {
  plain: "A plain paragraph of ordinary Latin text that is long enough to wrap over several lines in the narrow column of this fixture page, so that every line pitch is measured.",
  martin: "**v3 (2026-10-29), after Martin's review:** theorems in presentation order: A vc, B td (branching down an elimination forest, poly space), C nd (ILP), then the kernel block \"How small?\": D q-Colouring by vc (k + k^q, fixed q, 2 frames), BJK stated (no poly kernel for χ by vc), E χ by nd has a polynomial kernel (Martin's proximity argument + compression + reduction back to 3-Colouring; result known via solve-or-encode: Lafond–Luo, DAM 371, 2025). Also: nd figure, print handout.",
  cjk: "Wrapped text with 日本語の文字 and 中文字符 mixed in, plus ñandú and ΑΒΓ greek and кириллица, repeated: 日本語の文字 中文字符 ΑΒΓ кириллица 日本語の文字 中文字符 ΑΒΓ кириллица.",
  emoji: "Emoji 😀 and 🎉 and 🚀 inside wrapped text that goes on long enough to wrap over a few lines, 😀 🎉 🚀 😀 🎉 🚀 more words more words more words more words.",
  h1: "Heading one that is long enough to wrap over several lines in a narrow column indeed\nheading:: 1",
  h2: "Heading two that is long enough to wrap over several lines in a narrow column indeed\nheading:: 2",
  h3: "Heading three that is long enough to wrap over several lines in a narrow column indeed\nheading:: 3",
  h4: "Heading four that is long enough to wrap over several lines in a narrow column indeed\nheading:: 4",
  h5: "Heading five that is long enough to wrap over several lines in a narrow column indeed\nheading:: 5",
  h6: "Heading six that is long enough to wrap over several lines in a narrow column indeed\nheading:: 6",
  mdh1: "# Markdown heading that is long enough to wrap over several lines in a narrow column indeed",
  mdh1body: "# Markdown heading that is long enough to wrap over several lines in a narrow column\nbody line below the heading",
  h2body: "Title that is long enough to wrap over several lines in a narrow column indeed\nsecond line of the body\nheading:: 2",
  hauto: "Auto heading level from the depth\nheading:: true",
  inline: "Text with `inline code` and $x^2+y_1$ math and a [link](https://example.org) and [[Page link]] and #tag and **bold** and *italic* and ~~strike~~ and ==mark== repeated for wrap: `inline code` $x^2+y_1$ [link](https://example.org) [[Page link]].",
  multiline: "first line\nsecond line\n\nfourth after empty",
  code: "```js\nconst a = 1;\nconst b = 2;\n```",
  calc: "```calc\n1+1\n2*3\n```",
  deadline: "Pay the invoice\nDEADLINE: <2026-10-31 Fri>",
  scheduled: "Write the report\nSCHEDULED: <2026-10-31 Fri>",
  repeater: "Water plants\nDEADLINE: <2026-10-31 Fri 14:00 +1w>",
  both: "Review\nSCHEDULED: <2026-10-30 Thu>\nDEADLINE: <2026-11-02 Mon .+2d>",
  sameline: "Review\nSCHEDULED: <2026-10-30 Thu> DEADLINE: <2026-11-02 Mon .+2d>",
  todo: "TODO A task with a marker that is long enough to wrap over several lines of the narrow column here",
  priority: "[#A] A high priority task text that is long enough to wrap over several lines of the narrow column",
  // Task markers (CM6-TASKMARKER): the chip group (checkbox, marker chip, priority chip) must look the same in the view and in
  // the CM6 editor with the caret away from it. Short texts, one line, so the caret at the end is away from the marker.
  doing: "DOING Write the report", now: "NOW Write the report", later: "LATER Write the report", done: "DONE Write the report",
  waiting: "WAITING Write the report", canceled: "CANCELED Write the report",
  todop: "TODO [#A] Write the report", doingp: "DOING [#B] Write the report", donep: "DONE [#C] Write the report",
  todotags: "TODO Write the report #tag [[Page]]",
  // Inline constructs between plain words, one line each, so the caret at the end is away from them (CM6-TASKMARKER census).
  i_tag: "Write #tag the report now", i_btag: "Write #[[two words]] the report now", i_page: "Write [[Page]] the report now",
  i_aliasref: "Write [alias]([[Page]]) the report now", i_bref: "Write ((12345678-1234-1234-1234-123456789abc)) the report now",
  i_link: "Write [label](https://example.org) the report now", i_url: "Write https://example.org the report now",
  i_code: "Write `code` the report now", i_hl: "Write ^^bright^^ the report now", i_strike: "Write ~~gone~~ the report now",
  i_bold: "Write **bold** the report now", i_ital: "Write *italic* the report now", i_math: "Write $x^2$ the report now",
  i_macro: "Write {{unknown arg}} the report now", i_fn: "Write[^1] the report now",
  quote: "> A quoted paragraph that is long enough to wrap over several lines of the narrow column here and again\n> second quoted line",
  list: "Intro line\n- item one\n- item two\n  - nested item",
  olist: "1. first item\n2. second item\n3. third item",
  props: "Block with properties\nfoo:: bar\nbaz:: qux",
  table: "| a | b |\n|---|---|\n| 1 | 2 |\n| 3 | 4 |",
  image: "![alt](data:image/svg+xml;base64,PHN2ZyB4bWxucz0iaHR0cDovL3d3dy53My5vcmcvMjAwMC9zdmciIHdpZHRoPSIxMjAiIGhlaWdodD0iNjAiPjxyZWN0IHdpZHRoPSIxMjAiIGhlaWdodD0iNjAiIGZpbGw9IiNjY2MiLz48L3N2Zz4=)",
  math: "$$\\int_0^1 x\\,dx$$",
  // The same constructs after a line of text, so a caret at the start is NOT inside them (when one construct covers
  // the whole block, the reveal rule shows its source, whatever the caret does).
  quote2: "Intro line\n> A quoted paragraph that is long enough to wrap over several lines of the narrow column here and again\n> second quoted line",
  table2: "Intro line\n| a | b |\n|---|---|\n| 1 | 2 |\n| 3 | 4 |",
  math2: "Intro line\n$$\\int_0^1 x\\,dx$$",
  image2: "Intro line ![alt](data:image/svg+xml;base64,PHN2ZyB4bWxucz0iaHR0cDovL3d3dy53My5vcmcvMjAwMC9zdmciIHdpZHRoPSIxMjAiIGhlaWdodD0iNjAiPjxyZWN0IHdpZHRoPSIxMjAiIGhlaWdodD0iNjAiIGZpbGw9IiNjY2MiLz48L3N2Zz4=)",
  olist2: "Intro line\n1. first item\n2. second item\n3. third item",
  props2: "Block with properties\nfoo:: bar\nbaz:: qux\nlast line after",
  embed: "{{embed [[Target]]}}",
  hr: "above the rule\n---\nbelow the rule",
  empty: "",
};
// Facets the Rust backend would seed that the in-browser fixture must supply by hand.
const DTO_EXTRA: Record<string, Record<string, unknown>> = {
  mdh1: { heading_level: 1 }, mdh1body: { heading_level: 1 }, todo: { marker: "TODO" }, priority: { priority: "A" },
  doing: { marker: "DOING" }, now: { marker: "NOW" }, later: { marker: "LATER" }, done: { marker: "DONE" }, waiting: { marker: "WAITING" },
  canceled: { marker: "CANCELED" }, todop: { marker: "TODO", priority: "A" }, doingp: { marker: "DOING", priority: "B" },
  donep: { marker: "DONE", priority: "C" }, todotags: { marker: "TODO" },
};
await initParser();
loadSingle({ name: "Target", title: "Target", kind: "page", pre_block: null, format: "md",
  blocks: [{ id: "target-block", raw: "Embedded block text, one line", collapsed: false, children: [] },
    { id: "12345678-1234-1234-1234-123456789abc", raw: "Referenced block text", collapsed: false, children: [] }] });
loadSingle({ name: "LH", title: "LH", kind: "page", pre_block: null, format: "md",
  blocks: Object.entries(CASES).map(([id, raw]) => ({ id, raw, collapsed: false, ...DTO_EXTRA[id], scheduled: raw.match(/SCHEDULED: <([^>]*)>/)?.[1], deadline: raw.match(/DEADLINE: <([^>]*)>/)?.[1], properties: raw.match(/\nheading:: (\w+)$/) ? [["heading", raw.match(/\nheading:: (\w+)$/)![1]]] : [], children: [{ id: `${id}-child`, raw: "child", collapsed: false, children: [] }] })) });
Object.assign(window, { startEditing, endEdit, setCm6EditorEnabled, CASES });
render(() => <main class="main-content" style={{ padding: "40px", width: "640px" }}>
  <For each={pageByName("LH")!.roots}>{id => <Block id={id} />}</For>
</main>, document.getElementById("root")!);

// In-page probe, so any driver (Playwright Chromium, WebKitGTK's WebKitWebDriver) runs the identical measurement.
const wait = (ms: number) => new Promise(r => setTimeout(r, ms));
type Kind = "rendered" | "textarea" | "cm6" | "cm6end";
function probe(id: string, kind: Kind) {
  const row = document.querySelector(`[data-block-id="${id}"]`)!;
  const child = document.querySelector(`[data-block-id="${id}-child"]`)!;
  const box = (kind === "rendered" ? row.querySelector(":scope > .block-main .block-content")
    : kind === "textarea" ? row.querySelector("textarea.block-editor") : row.querySelector(".cm-content")) as HTMLElement | null;
  if (!box) return { missing: true };
  const rowRect = row.getBoundingClientRect(), r = box.getBoundingClientRect();
  // The editor host is what the block lays out (the CM6 block widgets and the code card sit beside .cm-content).
  const hostRect = (kind === "cm6" || kind === "cm6end" ? row.querySelector(".cm6-block-editor") : box)!.getBoundingClientRect();
  const lines: { top: number; h: number }[] = [];
  if (kind !== "textarea") {
    const walker = document.createTreeWalker(box, NodeFilter.SHOW_TEXT);
    for (let n = walker.nextNode(); n; n = walker.nextNode()) {
      if (!n.nodeValue!.trim()) continue;
      const range = document.createRange(); range.selectNodeContents(n);
      for (const q of Array.from(range.getClientRects())) lines.push({ top: +(q.top - rowRect.top).toFixed(2), h: +q.height.toFixed(2) });
    }
  }
  const cs = getComputedStyle(box);
  // The leading chip group, relative to the content box's left edge and the row's top (CM6-TASKMARKER).
  const chips = ["block-task-checkbox", "block-marker", "block-priority"].map(cls => {
    const el = box.querySelector(`.${cls}`) as HTMLElement | null;
    if (!el) return null;
    const q = el.getBoundingClientRect(), c = getComputedStyle(el);
    return { cls, x: +(q.left - r.left).toFixed(2), y: +(q.top - rowRect.top).toFixed(2), w: +q.width.toFixed(2), h: +q.height.toFixed(2),
      text: el.textContent, border: c.borderTopWidth + " " + c.borderTopStyle + " " + c.borderTopColor, radius: c.borderTopLeftRadius,
      bg: c.backgroundColor, color: c.color, fontSize: c.fontSize, margin: c.marginLeft + "/" + c.marginRight, pad: c.paddingLeft + "/" + c.paddingRight, va: c.verticalAlign };
  }).filter(Boolean);
  // Every inline element the renderer can emit, in document order (CM6's wrappers/buffers are not elements of the view).
  const INL = "a,code,mark,del,em,strong,u,sup,sub,b,.tag,.page-ref,.block-ref,.katex,.property,.macro,.footnote-ref,.bracket";
  const inl = Array.from(box.querySelectorAll(INL)).filter(el => !el.closest(".block-marker,.block-task-checkbox,.katex-html,.katex-mathml") && !(el.parentElement?.closest(".katex"))).map(el => {
    const q = el.getBoundingClientRect(), c = getComputedStyle(el);
    return { el: el.tagName.toLowerCase() + "." + [...el.classList].filter(k => !k.startsWith("cm-")).join("."), x: +(q.left - r.left).toFixed(2), y: +(q.top - rowRect.top).toFixed(2), w: +q.width.toFixed(2), h: +q.height.toFixed(2),
      text: (el.textContent || "").slice(0, 30), color: c.color, bg: c.backgroundColor, fontSize: c.fontSize, fontWeight: c.fontWeight, fontStyle: c.fontStyle, deco: c.textDecorationLine, border: c.borderTopWidth + " " + c.borderTopColor, radius: c.borderTopLeftRadius, pad: c.paddingLeft + "/" + c.paddingTop, family: c.fontFamily.slice(0, 24), display: c.display };
  });
  // CM6's own caret of an EMPTY document (CM6-EMPTYCARET): present, shown and as tall as text, left edge on the line.
  const dc = kind === "cm6" || kind === "cm6end" ? row.querySelector(".cm6-empty-caret") as HTMLElement | null : null;
  const drawnCaret = dc ? (q => ({ x: +(q.left - r.left).toFixed(2), y: +(q.top - rowRect.top).toFixed(2), w: +q.width.toFixed(2), h: +q.height.toFixed(2),
    shown: getComputedStyle(dc.parentElement!).display !== "none", native: getComputedStyle(box).caretColor }))(dc.getBoundingClientRect()) : null;
  return {
    chips, inl, drawnCaret,
    boxH: +r.height.toFixed(2), hostH: +hostRect.height.toFixed(2), childY: +(child.getBoundingClientRect().top - rowRect.top).toFixed(2),
    lineHeight: cs.lineHeight, fontSize: cs.fontSize, cls: box.className, val: (box as HTMLTextAreaElement).value ?? box.textContent!.slice(0, 120),
    lines: [...new Map(lines.map(t => [t.top, t])).values()].sort((a, b) => a.top - b.top),
  };
}
async function until(sel: string) { for (let i = 0; i < 50 && !document.querySelector(sel); i++) await wait(100); }
Object.assign(window, {
  measureAll: async (opts: { theme: string; font: string; lh: string }) => {
    document.documentElement.setAttribute("data-theme", opts.theme);
    if (opts.font) document.documentElement.style.setProperty("--tine-font-size", `${opts.font}px`);
    if (opts.lh) document.documentElement.style.setProperty("--tine-typography-line-height", opts.lh);
    await wait(100);
    const results: Record<string, Record<string, unknown>> = {};
    for (const id of Object.keys(CASES)) {
      results[id] = {};
      endEdit("page-navigation"); await wait(50);
      // Blocks far from the viewport render a placeholder until they are near; measure the real render.
      document.querySelector(`[data-block-id="${id}"]`)!.scrollIntoView({ block: "center" }); await wait(150);
      results[id].rendered = probe(id, "rendered");
      for (const [kind, on, caret] of [["textarea", false, 0], ["cm6", true, 0], ["cm6end", true, 1e9]] as const) {
        setCm6EditorEnabled(on); await wait(50);
        startEditing(id, caret);
        await until(on ? `[data-block-id="${id}"] .cm-content` : `[data-block-id="${id}"] textarea.block-editor`);
        await wait(150);
        results[id][kind] = probe(id, kind);
        endEdit("page-navigation"); await wait(50);
      }
    }
    return results;
  },
  // Playwright-only: screenshot hooks need the editor open, so it exposes the open step separately.
  openEditor: async (id: string, on: boolean, caret = 0) => {
    endEdit("page-navigation"); setCm6EditorEnabled(on); await wait(50);
    startEditing(id, caret);
    await until(on ? `[data-block-id="${id}"] .cm-content` : `[data-block-id="${id}"] textarea.block-editor`); await wait(150);
  },
  // The caret of the open editor (CM6-EMPTYCARET): where the browser would paint it, and whether anything could hide it.
  caretProbe: (id: string) => {
    const row = document.querySelector(`[data-block-id="${id}"]`)!;
    const content = row.querySelector(".cm-content") as HTMLElement | null;
    const sel = window.getSelection();
    let rect: { x: number; y: number; w: number; h: number } | null = null;
    if (sel && sel.rangeCount) {
      const rg = sel.getRangeAt(0).cloneRange();
      const q = Array.from(rg.getClientRects())[0] ?? rg.getBoundingClientRect();
      rect = { x: +q.left.toFixed(2), y: +q.top.toFixed(2), w: +q.width.toFixed(2), h: +q.height.toFixed(2) };
    }
    const a = document.activeElement as HTMLElement | null;
    return {
      focused: !!content && (a === content || content.contains(a)), active: a ? a.tagName + "." + a.className : null,
      html: content?.outerHTML.slice(0, 400) ?? null, rect, caretColor: content ? getComputedStyle(content).caretColor : null,
      selInContent: !!(sel && sel.anchorNode && content?.contains(sel.anchorNode)), collapsed: sel?.isCollapsed ?? null,
      contentRect: content ? (({ left: x, top: y, width: w, height: h }) => ({ x, y, w, h }))(content.getBoundingClientRect()) : null,
    };
  },
  closeEditor: async () => { endEdit("page-navigation"); await wait(50); },
});
