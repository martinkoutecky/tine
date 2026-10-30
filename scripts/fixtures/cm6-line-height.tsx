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
  inline: "Text with `inline code` and $x^2+y_1$ math and a [link](https://example.org) and [[Page link]] and #tag and **bold** and *italic* and ~~strike~~ and ==mark== repeated for wrap: `inline code` $x^2+y_1$ [link](https://example.org) [[Page link]].",
  multiline: "first line\nsecond line\n\nfourth after empty",
  code: "```js\nconst a = 1;\nconst b = 2;\n```",
  calc: "```calc\n1+1\n2*3\n```",
  empty: "",
};
await initParser();
loadSingle({ name: "LH", title: "LH", kind: "page", pre_block: null, format: "md",
  blocks: Object.entries(CASES).map(([id, raw]) => ({ id, raw, collapsed: false, properties: raw.match(/\nheading:: (\w+)$/) ? [["heading", raw.match(/\nheading:: (\w+)$/)![1]]] : [], children: [{ id: `${id}-child`, raw: "child", collapsed: false, children: [] }] })) });
Object.assign(window, { startEditing, endEdit, setCm6EditorEnabled, CASES });
render(() => <main class="main-content" style={{ padding: "40px", width: "640px" }}>
  <For each={pageByName("LH")!.roots}>{id => <Block id={id} />}</For>
</main>, document.getElementById("root")!);

// In-page probe, so any driver (Playwright Chromium, WebKitGTK's WebKitWebDriver) runs the identical measurement.
const wait = (ms: number) => new Promise(r => setTimeout(r, ms));
function probe(id: string, kind: "rendered" | "textarea" | "cm6") {
  const row = document.querySelector(`[data-block-id="${id}"]`)!;
  const child = document.querySelector(`[data-block-id="${id}-child"]`)!;
  const box = (kind === "rendered" ? row.querySelector(":scope > .block-main .block-content")
    : kind === "textarea" ? row.querySelector("textarea.block-editor") : row.querySelector(".cm-content")) as HTMLElement | null;
  if (!box) return { missing: true };
  const rowRect = row.getBoundingClientRect(), r = box.getBoundingClientRect();
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
  return {
    boxH: +r.height.toFixed(2), childY: +(child.getBoundingClientRect().top - rowRect.top).toFixed(2),
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
      results[id].rendered = probe(id, "rendered");
      for (const [kind, on] of [["textarea", false], ["cm6", true]] as const) {
        setCm6EditorEnabled(on); await wait(50);
        startEditing(id, 0);
        await until(kind === "cm6" ? `[data-block-id="${id}"] .cm-content` : `[data-block-id="${id}"] textarea.block-editor`);
        await wait(150);
        results[id][kind] = probe(id, kind);
        endEdit("page-navigation"); await wait(50);
      }
    }
    return results;
  },
  // Playwright-only: screenshot hooks need the editor open, so it exposes the open step separately.
  openEditor: async (id: string, on: boolean) => {
    endEdit("page-navigation"); setCm6EditorEnabled(on); await wait(50);
    startEditing(id, 0);
    await until(on ? `[data-block-id="${id}"] .cm-content` : `[data-block-id="${id}"] textarea.block-editor`); await wait(150);
  },
  closeEditor: async () => { endEdit("page-navigation"); await wait(50); },
});
