import { afterEach, beforeAll, expect, it, vi } from "vitest";
import { render } from "solid-js/web";
import { EditorView } from "@codemirror/view";
import { Cm6BlockEditor } from "./Cm6BlockEditor";
import { initParser } from "../render/parse";
import { node, resetStore } from "../document";
import { loadSingle } from "../document/workingSet";
import { startEditing, endEdit } from "../editorController";
import { editorIn } from "../tests/editorDriver";
import { cm6PreviewCases } from "../tests/cm6PreviewCases";
import { previewSpans } from "../editor/cm6Preview";

let dispose: (() => void) | undefined;
beforeAll(() => {
  if (!Range.prototype.getClientRects) Range.prototype.getClientRects = () => [] as unknown as DOMRectList;
  if (!Range.prototype.getBoundingClientRect) Range.prototype.getBoundingClientRect = () => new DOMRect();
  return initParser();
});
afterEach(() => { dispose?.(); dispose = undefined; endEdit("page-navigation"); resetStore(); document.body.innerHTML = ""; });

function mount(raw: string, format: "md" | "org" = "md") {
  loadSingle({ name: "Reveal", title: "Reveal", kind: "page", format, pre_block: null,
    blocks: [{ id: "reveal", raw, collapsed: false, children: [] }] });
  startEditing("reveal", raw.length);
  const root = document.createElement("div"); document.body.append(root);
  dispose = render(() => <Cm6BlockEditor id="reveal" surface="main" scope={null} />, root);
  return { root, editor: editorIn(root) };
}

it("plain typing preserves editor configuration while publishing the draft", () => {
  const { editor } = mount("Typing target");
  const updates = vi.spyOn(EditorView.prototype, "update");
  try {
    editor.type("x");
    expect(node("reveal").raw).toBe("Typing targetx");
    expect(updates.mock.calls.flatMap(([transactions]) => transactions).filter(tr => tr.reconfigured)).toHaveLength(0);
  } finally { updates.mockRestore(); }
});

it.each(cm6PreviewCases)("$name renders outside, reveals inside/spanning, and preserves boundary bytes", async fixture => {
  const raw = fixture.raw ?? `before ${fixture.source} after`;
  const { root, editor } = mount(raw, fixture.format);
  const span = previewSpans(raw, fixture.format).find(s => s.kind === fixture.kind);
  expect(span, "lsdoc owns this construct").toBeDefined();
  const { from, to } = span!;
  const widget = () => root.querySelector(`[data-cm6-construct="${fixture.kind}"]`);
  editor.select(from > 0 ? 0 : raw.length);
  expect(widget(), "caret elsewhere renders").not.toBeNull();
  if (fixture.selector) await vi.waitFor(() => expect(widget()!.querySelector(fixture.selector!), "uses the read renderer").not.toBeNull());
  expect(editor.value()).toBe(raw);
  expect(node("reveal").raw).toBe(raw);
  editor.select(Math.min(from + 1, to));
  expect(widget(), "caret inside reveals").toBeNull();
  editor.select(Math.max(0, from - 1), Math.min(raw.length, to + 1));
  expect(widget(), "selection spanning reveals").toBeNull();
  for (const boundary of [from, to]) {
    editor.select(boundary);
    expect(widget(), "boundary exposes source before typing").toBeNull();
    editor.type("文😀");
    const expected = raw.slice(0, boundary) + "文😀" + raw.slice(boundary);
    expect(editor.value()).toBe(expected);
    expect(node("reveal").raw).toBe(expected);
    editor.input(raw, raw.length);
    expect(node("reveal").raw).toBe(raw);
  }
});

it("reveals only the selected nested constructs and keeps a sibling rendered", () => {
  const raw = "中文😀 **bold *inner* tail** and `sibling` end";
  const { root, editor } = mount(raw);
  editor.select(raw.indexOf("bold") + 1);
  expect(root.querySelector('[data-cm6-construct="bold"]')).toBeNull();
  expect(root.querySelector('[data-cm6-construct="italic"]')).not.toBeNull();
  expect(root.querySelector('[data-cm6-construct="code"]')).not.toBeNull();
  editor.select(raw.indexOf("inner") + 1);
  expect(root.querySelector('[data-cm6-construct="italic"]')).toBeNull();
  expect(root.querySelector('[data-cm6-construct="code"]')).not.toBeNull();
  expect(node("reveal").raw).toBe(raw);
});

it("clicking rendered markup reveals its host source without following a link", () => {
  const raw = "before [[Target]] after";
  const { root, editor } = mount(raw);
  editor.select(0);
  root.querySelector('[data-cm6-construct="page"]')!.dispatchEvent(new MouseEvent("mousedown", { bubbles: true, cancelable: true }));
  expect(root.querySelector('[data-cm6-construct="page"]')).toBeNull();
  expect(editor.selection()).toEqual([7, 7]);
  expect(node("reveal").raw).toBe(raw);
});

it.each([
  ["Markdown code", "md", "```js\ncode\n```", "```js\n", "\n```"],
  ["Org source", "org", "#+BEGIN_SRC js\ncode\n#+END_SRC", "#+BEGIN_SRC js\n", "\n#+END_SRC"],
  ["calculator", "md", "```calc\n1+1\n```", "```calc\n", "```"],
] as const)("reveals exact wrappers around a projected %s body", (_name, format, raw, open, close) => {
  const { root, editor } = mount(raw, format);
  expect([...root.querySelectorAll(".cm6-projected-source")].map(el => el.textContent)).toEqual([open, close]);
  const body = editor.value();
  editor.select(0); editor.type("文😀");
  expect(node("reveal").raw).toBe(raw.replace(body, "文😀" + body));
});

it("removes projected source wrappers when the caret leaves a mixed fence", () => {
  const raw = "before\n```js\ncode\n```\nafter";
  const { root, editor } = mount(raw);
  editor.selectByUser(raw.indexOf("code"));
  expect(editor.value()).toBe("code");
  expect([...root.querySelectorAll(".cm6-projected-source")].map(el => el.textContent)).toEqual(["```js\n", "\n```"]);
  editor.select(0); editor.keydown({ key: "ArrowLeft" });
  expect(root.querySelector(".cm6-projected-source")).toBeNull();
  expect(editor.value()).toBe(raw);
  expect(root.querySelector('[data-cm6-construct="src"]')).toBeNull();
  editor.select(0);
  expect(root.querySelector('[data-cm6-construct="src"]')).not.toBeNull();
  expect(node("reveal").raw).toBe(raw);
});

// CM6-DEADLINE: the planning line is the view's date chip while the caret is elsewhere, and its source otherwise.
it("renders a DEADLINE line as the view's date chip and reveals its source only inside the line", async () => {
  const raw = "Pay the invoice\nDEADLINE: <2026-10-31 Fri +1w>";
  const { root, editor } = mount(raw);
  const chip = () => root.querySelector('[data-cm6-construct="planning"] .date-chip.deadline');
  editor.select(0);
  await vi.waitFor(() => expect(chip()).not.toBeNull());
  expect(chip()!.textContent!.trim()).toBe("2026-10-31 Fri +1w"); // the text Rendered.tsx puts in its chip
  expect(root.querySelectorAll(".cm-line")).toHaveLength(1); // the chip joins the header line, as in the view
  expect(editor.value()).toBe(raw);
  editor.select(15); // end of the header line: the usual place to keep typing a title
  expect(chip(), "the end of the header line is not inside the planning line").not.toBeNull();
  editor.select(16);
  expect(chip(), "the start of the planning line reveals its source").toBeNull();
  expect(root.querySelectorAll(".cm-line")).toHaveLength(2);
  editor.select(raw.length);
  expect(chip()).toBeNull();
  expect(node("reveal").raw).toBe(raw);
});
