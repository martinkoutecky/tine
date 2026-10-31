import { afterEach, beforeAll, expect, it, vi } from "vitest";
import { render } from "solid-js/web";
import hljs from "highlight.js/lib/common";
import { Cm6BlockEditor } from "./Cm6BlockEditor";
import { initParser } from "../render/parse";
import { loadSingle } from "../document/workingSet";
import { node, resetStore } from "../document";
import { endEdit, startEditing } from "../editorController";
import { editorIn } from "../tests/editorDriver";

// jsdom has no workers. Run the actual bundled grammar asynchronously behind
// the browser's worker message boundary; the Chromium probe uses a real worker.
class HighlightWorker {
  static requests: unknown[] = [];
  static paused = false;
  static replies: (() => void)[] = [];
  static instances: HighlightWorker[] = [];
  onmessage: ((event: MessageEvent) => void) | null = null;
  terminate = vi.fn();
  constructor() { HighlightWorker.instances.push(this); }
  postMessage(request: { revision: number; regions: { from: number; to: number; code: string; lang: string }[] }) {
    HighlightWorker.requests.push(request);
    const reply = () => this.onmessage?.({ data: { revision: request.revision, regions: request.regions.map(region => ({
      ...region, html: region.lang && hljs.getLanguage(region.lang)
        ? hljs.highlight(region.code, { language: region.lang }).value : hljs.highlightAuto(region.code).value,
    })) } } as MessageEvent);
    if (HighlightWorker.paused) HighlightWorker.replies.push(reply);
    else queueMicrotask(reply);
  }
}

let dispose: (() => void) | undefined;
beforeAll(async () => {
  if (!Range.prototype.getClientRects) Range.prototype.getClientRects = () => [] as unknown as DOMRectList;
  if (!Range.prototype.getBoundingClientRect) Range.prototype.getBoundingClientRect = () => new DOMRect();
  await initParser();
});
afterEach(() => {
  dispose?.(); dispose = undefined; endEdit("page-navigation"); resetStore();
  document.body.innerHTML = ""; vi.unstubAllGlobals(); HighlightWorker.requests = [];
  HighlightWorker.paused = false; HighlightWorker.replies = []; HighlightWorker.instances = [];
});

function mount(raw: string, caret = raw.length, format: "md" | "org" = "md") {
  vi.stubGlobal("Worker", HighlightWorker);
  loadSingle({ name: "Highlight", title: "Highlight", kind: "page", format, pre_block: null,
    blocks: [{ id: "highlight", raw, collapsed: false, children: [] }] });
  startEditing("highlight", caret);
  const root = document.createElement("div"); document.body.append(root);
  dispose = render(() => <Cm6BlockEditor id="highlight" surface="main" scope={null} />, root);
  return { root, editor: editorIn(root) };
}

it.each([
  ["md", "```js\nconst greeting = '文😀';\n```", "const", "'文😀'"],
  ["md", "```python\ndef greet():\n    return '文😀'\n```", "def", "'文😀'"],
  ["org", "#+BEGIN_SRC python\ndef greet():\n    return '文😀'\n#+END_SRC", "def", "'文😀'"],
] as const)("keeps %s code token classes while editing and after typing", async (format, raw, keyword, string) => {
  const { root, editor } = mount(raw, raw.length, format);
  await vi.waitFor(() => expect(root.querySelector(".cm-content .hljs-keyword")?.textContent).toBe(keyword));
  expect(root.querySelector(".cm-content .hljs-string")?.textContent).toBe(string);
  expect(editor.value()).not.toMatch(/```|#\+(BEGIN|END)/);
  const before = editor.value();
  editor.select(before.indexOf(string) + 1); editor.type("hello ");
  await vi.waitFor(() => expect(root.querySelector(".cm-content .hljs-string")?.textContent).toBe("'hello 文😀'"));
  expect(node("highlight").raw).toBe(raw.replace(string, "'hello 文😀'"));
  expect(editor.kind).toBe("cm6");
});

it("highlights mixed raw and projected fences without changing the reveal boundary", async () => {
  const raw = "before\n```js\nconst answer = 42;\n```\nafter";
  const { root, editor } = mount(raw, raw.indexOf("```"));
  await vi.waitFor(() => expect(root.querySelector(".cm-content .hljs-keyword")?.textContent).toBe("const"));
  expect(editor.value()).toBe(raw);
  expect(root.querySelector('[data-cm6-construct="src"]')).toBeNull();
  editor.selectByUser(raw.indexOf("answer"));
  await vi.waitFor(() => expect(root.querySelector(".cm-content .hljs-keyword")?.textContent).toBe("const"));
  expect(editor.value()).toBe("const answer = 42;");
  editor.select(0); editor.keydown({ key: "ArrowLeft" });
  expect(editor.value()).toBe(raw);
  editor.select(0);
  expect(root.querySelector('[data-cm6-construct="src"]')).not.toBeNull();
  expect(node("highlight").raw).toBe(raw);
  expect(editor.kind).toBe("cm6");
});

it("coalesces rapid typing and never rehighlights on caret movement", async () => {
  const { root, editor } = mount("```js\nconst answer = 42;\n```");
  await vi.waitFor(() => expect(root.querySelector(".cm-content .hljs-keyword")).not.toBeNull());
  const requests = HighlightWorker.requests.length;
  editor.select(1); editor.select(2); editor.select(3);
  expect(HighlightWorker.requests).toHaveLength(requests);
  editor.select(editor.value().length);
  for (const char of " // typing") editor.type(char);
  expect(HighlightWorker.requests).toHaveLength(requests);
  await vi.waitFor(() => expect(root.querySelector(".cm-content .hljs-comment")?.textContent).toBe("// typing"));
  expect(HighlightWorker.requests).toHaveLength(requests + 1);
});

it("discards stale replies, coalesces work in flight, and terminates on disposal", async () => {
  HighlightWorker.paused = true;
  const { root, editor } = mount("```js\nconst answer = 42;\n```");
  await vi.waitFor(() => expect(HighlightWorker.requests).toHaveLength(1));
  editor.select(0, editor.value().length); editor.type("return 'new';");
  await new Promise(resolve => setTimeout(resolve, 100));
  editor.select(editor.value().length); editor.type(" // newest");
  await new Promise(resolve => setTimeout(resolve, 100));
  expect(HighlightWorker.requests, "only one in-flight request").toHaveLength(1);
  HighlightWorker.replies.shift()!();
  expect(root.querySelector(".cm-content .hljs-keyword"), "stale response is ignored").toBeNull();
  expect(HighlightWorker.requests).toHaveLength(2);
  HighlightWorker.replies.shift()!();
  expect(root.querySelector(".cm-content .hljs-keyword")?.textContent).toBe("return");
  expect(root.querySelector(".cm-content .hljs-comment")?.textContent).toBe("// newest");
  const worker = HighlightWorker.instances[0];
  editor.type(" more");
  await vi.waitFor(() => expect(HighlightWorker.requests).toHaveLength(3));
  dispose!(); dispose = undefined;
  expect(worker.terminate).toHaveBeenCalledOnce();
  expect(() => HighlightWorker.replies.shift()!()).not.toThrow();
});

it("keeps CM6 editing usable when the optional worker cannot start", async () => {
  const { root, editor } = mount("```js\nconst answer = 42;\n```");
  vi.stubGlobal("Worker", class { constructor() { throw new Error("worker unavailable"); } });
  await new Promise(resolve => setTimeout(resolve, 100));
  editor.select(editor.value().length); editor.type(" // still editable");
  expect(node("highlight").raw).toBe("```js\nconst answer = 42; // still editable\n```");
  expect(root.querySelector(".cm-content")).not.toBeNull();
  expect(editor.kind).toBe("cm6");
});
