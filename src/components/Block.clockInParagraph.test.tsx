import { afterAll, afterEach, beforeAll, describe, expect, it, vi } from "vitest";
import { For, type JSX } from "solid-js";
import { render } from "solid-js/web";
import { startEditing, endEdit, editingId } from "../editorController";
import { installKeybindings } from "../keybindings";
import { initParser } from "../render/parse";
import { pageByName, resetStore } from "../document";
import { loadSingle } from "../document/workingSet";
import { doc } from "../document/model";
import { trailingVisibleEmptyLeaf } from "../document/tree";
import { timestampText, renderedBlockText } from "../render/renderedText";
import { visibleBody } from "../render/block";
import { logbookInfo } from "../logbook";
import { toasts, setToasts, dismissToast } from "../toasts";
import * as properties from "../editor/properties";
import type { TimestampInline } from "../render/ast";
import type { BlockDto, PageDto } from "../types";
import { Block } from "./Block";

// GH #741: typing DOING makes Tine write the :LOGBOOK: clock-in immediately.
// (1) Text typed after it must stay on the task line, not land after `:END:`.
// (2) A CLOCK line that is NOT inside a recognised drawer (`:END:  test`,
//     an unterminated drawer, a pasted fragment) parses to a paragraph with a
//     `Clock` timestamp inline whose `date` is a [kind, value] pair. The
//     renderer read `.date.year` off that pair and threw, which blanked the
//     whole page behind the page-level failure boundary. OG renders it.

let disposeKeys: (() => void) | null = null;
beforeAll(async () => {
  await initParser();
  disposeKeys = installKeybindings();
});
afterAll(() => disposeKeys?.());
afterEach(() => {
  endEdit("page-navigation");
  resetStore();
  document.body.innerHTML = "";
});

let seq = 0;
function mountPage(raw: string, editing: boolean, format: "md" | "org" = "md", next = false) {
  const name = `Clock${++seq}`;
  const block: BlockDto = { id: `clock-${seq}`, raw, collapsed: false, children: [] };
  const page: PageDto = { name, kind: "page", title: name, pre_block: null, blocks: [block, ...(next ? [{id: `next-${seq}`, raw: "next", collapsed: false, children: []}] : [])], format };
  loadSingle(page);
  if (editing) startEditing(block.id, 0);
  const root = document.createElement("div");
  document.body.appendChild(root);
  const dispose = render(
    () => (<For each={pageByName(name)?.roots ?? []}>{(id): JSX.Element => <Block id={id} />}</For>),
    root,
  );
  return { root, dispose, blockId: block.id, textarea: root.querySelector("textarea.block-editor") as HTMLTextAreaElement | null };
}

function typeChar(ta: HTMLTextAreaElement, ch: string) {
  const caret = ta.selectionStart;
  ta.value = ta.value.slice(0, caret) + ch + ta.value.slice(ta.selectionEnd);
  ta.setSelectionRange(caret + ch.length, caret + ch.length);
  ta.dispatchEvent(new InputEvent("input", { bubbles: true, inputType: "insertText", data: ch }));
}

const CLOCK = "CLOCK: [2026-10-21 Wed 08:54:45]";

it('review r2 refused Org edit survives switching blocks', () => {
  const raw = `plain\n:PROPERTIES:\n:collapsed: false\n:END:\n:LOGBOOK:\n${CLOCK}\n:END:`;
  const {root, textarea, blockId, dispose} = mountPage(raw,true,'org',true);
  const errors:unknown[]=[];
  const onError=(e:ErrorEvent)=>{errors.push(e.error);e.preventDefault();};
  window.addEventListener('error',onError);
  try {
    setToasts([]);
    const typed = 'plain\n:PROPERTIES:\n:COLLAPSED: true\n:END:\nmy newly typed text';
    textarea!.value=typed;
    textarea!.dispatchEvent(new InputEvent('input',{bubbles:true,inputType:'insertFromPaste'}));
    textarea!.dispatchEvent(new InputEvent('input',{bubbles:true,inputType:'insertText'}));
    const next = root.querySelector(`[data-block-id="next-${seq}"] .block-content-wrapper`)!;
    next.dispatchEvent(new MouseEvent('mousedown',{bubbles:true,button:0,cancelable:true}));
    document.dispatchEvent(new MouseEvent('mouseup',{bubbles:true,button:0}));
    startEditing(blockId,0);
    expect(root.querySelector<HTMLTextAreaElement>('textarea.block-editor')?.value).toContain('my newly typed text');
    expect(errors).toEqual([]);
    expect(toasts()).toEqual([]);
    expect(logbookInfo(doc.byId[blockId].raw, 'org').rows).toHaveLength(1);
  } finally {window.removeEventListener('error',onError);dispose();}
});

it.each(['md', 'org'] as const)('review r2 refused genuine join keeps the buffer and reports recovery (%s)', format => {
  const raw = `plain\n:LOGBOOK:\n${CLOCK}\n:END:`;
  const { root, textarea, blockId, dispose } = mountPage(raw, true, format, true);
  // Exercise a release-kernel refusal independently of the casing correction.
  const join = vi.spyOn(properties, 'joinProps').mockImplementation(() => {
    throw new Error('Drawer reattachment refused: metadata ownership would change');
  });
  const onError = (e: ErrorEvent) => e.preventDefault();
  let disposed = false;
  window.addEventListener('error', onError);
  try {
    setToasts([]);
    const typed = 'my newly typed text';
    textarea!.value = typed;
    textarea!.dispatchEvent(new InputEvent('input', { bubbles: true, inputType: 'insertFromPaste' }));
    expect(doc.byId[blockId].raw).toBe(raw);
    expect(toasts()).toHaveLength(1);
    expect(toasts()[0]).toMatchObject({ kind: 'error', sticky: true, copyText: typed });
    expect(toasts()[0].message).toContain('Copy');
    const next = root.querySelector(`[data-block-id="next-${seq}"] .block-content-wrapper`)!;
    next.dispatchEvent(new MouseEvent('mousedown', { bubbles: true, button: 0, cancelable: true }));
    document.dispatchEvent(new MouseEvent('mouseup', { bubbles: true, button: 0 }));
    endEdit('blur');
    startEditing(blockId, 0);
    expect(editingId()).toBe(blockId);
    expect(textarea!.isConnected).toBe(true);
    expect(textarea!.value).toBe(typed);
    expect(doc.byId[blockId].raw).toBe(raw);
    // Correcting the buffer releases the edit hold; ordinary navigation works.
    join.mockRestore();
    textarea!.dispatchEvent(new InputEvent('input', { bubbles: true, inputType: 'insertText' }));
    expect(doc.byId[blockId].raw).toContain(typed);
    expect(logbookInfo(doc.byId[blockId].raw, format).rows).toHaveLength(1);
    startEditing(`next-${seq}`, 0);
    expect(root.querySelector<HTMLTextAreaElement>('textarea.block-editor')?.value).toBe('next');
    startEditing(blockId, 0);
    expect(root.querySelector<HTMLTextAreaElement>('textarea.block-editor')?.value).toBe(typed);
    // Dismissing an error then closing the surface still retains its last
    // refused buffer through the ordinary error toast's Copy recovery.
    const refusedAgain = vi.spyOn(properties, 'joinProps').mockImplementation(() => {
      throw new Error('metadata ownership would change');
    });
    try {
      const reopened = root.querySelector<HTMLTextAreaElement>('textarea.block-editor')!;
      const finalText = typed + ' before closing';
      reopened.value = finalText;
      reopened.dispatchEvent(new InputEvent('input', { bubbles: true, inputType: 'insertFromPaste' }));
      for (const toast of toasts()) dismissToast(toast.id);
      dispose();
      disposed = true;
      expect(toasts()).toHaveLength(1);
      expect(toasts()[0]).toMatchObject({ kind: 'error', sticky: true, copyText: finalText });
      expect(doc.byId[blockId].raw).not.toContain('before closing');
      expect(logbookInfo(doc.byId[blockId].raw, format).rows).toHaveLength(1);
      // Identical errors in another editor must retain both recovery copies.
      const other = mountPage(raw, true, format);
      try {
        other.textarea!.value = 'another refused draft';
        other.textarea!.dispatchEvent(new InputEvent('input', { bubbles: true, inputType: 'insertFromPaste' }));
      } finally { other.dispose(); }
      expect(toasts().map(toast => toast.copyText)).toEqual([finalText, 'another refused draft']);
    } finally { refusedAgain.mockRestore(); }
  } finally {
    join.mockRestore();
    window.removeEventListener('error', onError);
    if (!disposed) dispose();
    setToasts([]);
  }
});

it.each(['md','org'] as const)('review manual drawer typing preserves subsequent text (%s)', format => {
  const { textarea, blockId, dispose } = mountPage('plain', true, format);
  try {
    textarea!.setSelectionRange(5,5);
    const typed = `\n:LOGBOOK:\n${CLOCK}\n:END: suffix`;
    for (const ch of typed) typeChar(textarea!,ch);
    expect(doc.byId[blockId].raw).toBe('plain'+typed);
    expect(textarea!.value).toBe('plain'+typed);
  } finally {dispose();}
});

it.each(["md", "org"] as const)("Backspace preserves a titleless logbook-only first block (%s)", (format) => {
  const raw = `:LOGBOOK:\n${CLOCK}\n:END:`;
  const { textarea, blockId, dispose } = mountPage(raw, true, format, true);
  try {
    expect(textarea!.value).toBe("");
    expect(trailingVisibleEmptyLeaf({roots: [blockId]})).toBeNull();
    textarea!.dispatchEvent(new KeyboardEvent("keydown", {key: "Backspace", bubbles: true, cancelable: true}));
    expect(doc.byId[blockId]?.raw).toBe(raw);
  } finally { dispose(); }
});

describe("a CLOCK line outside a recognised drawer renders (GH #741)", () => {
  it.each([
    ["text after :END:", `DOING\n:LOGBOOK:\n${CLOCK}\n:END:  test`, "test"],
    ["backtick after :END:", `\`DOING\n:LOGBOOK:\n${CLOCK}\n:END:  \``, "DOING"],
    ["unterminated drawer", `DOING x\n:LOGBOOK:\n${CLOCK}`, "x"],
    ["closed clock in a paragraph", `a\nCLOCK: [2026-10-21 Wed 08:54:45]--[2026-10-21 Wed 09:00:00] =>  00:05:15\nb`, "00:05:15"],
  ])("%s", (_name, raw, visible) => {
    const { root, dispose } = mountPage(raw, false);
    try {
      expect(root.textContent).toContain(visible);
      expect(root.textContent).toContain("2026-10-21");
    } finally {
      dispose();
    }
  });
});

// Every consumer of a parsed clock/logbook/planning date, over both literal
// samples of the report, their org spellings, and every prefix and every
// single-character deletion of them (the shapes a user passes through while
// typing, and the damaged shapes an external edit leaves behind).
describe("no reader of a clock/logbook date throws on a damaged logbook (GH #741)", () => {
  const samples = [
    `DOING\n:LOGBOOK:\n${CLOCK}\n:END:  test`,
    `\`DOING\n:LOGBOOK:\n${CLOCK}\n:END:  \``,
    `DONE x\n:LOGBOOK:\nCLOCK: [2026-10-21 Wed 08:54:45]--[2026-10-21 Wed 09:00:00] =>  00:05:15\n:END:`,
    `TODO x\nSCHEDULED: <2026-10-21 Wed>\n:LOGBOOK:\n${CLOCK}\n:END:`,
  ];
  const variants = new Set<string>();
  for (const sample of samples) {
    variants.add(sample);
    for (let i = 0; i <= sample.length; i++) variants.add(sample.slice(0, i));
    for (let i = 0; i < sample.length; i++) variants.add(sample.slice(0, i) + sample.slice(i + 1));
  }
  const readers = (raw: string, format: "md" | "org") => {
    logbookInfo(raw, format);
    visibleBody(raw, "md");
    renderedBlockText(raw, format, { typographicGlyphs: false, stripLinks: false, removeTags: false, removeProperties: false });
  };
  it("the parse-only readers never throw", () => {
    for (const raw of variants) {
      for (const format of ["md", "org"] as const) {
        try { readers(raw, format); } catch (e) { throw new Error(`${format} ${JSON.stringify(raw)}: ${String(e)}`); }
      }
    }
  });
  it("a block renders, read-only and open for editing", () => {
    for (const raw of variants) {
      for (const editing of [false, true]) {
        let mounted: ReturnType<typeof mountPage> | null = null;
        try { mounted = mountPage(raw, editing); } catch (e) { throw new Error(`${editing ? "editing" : "reading"} ${JSON.stringify(raw)}: ${String(e)}`); }
        finally { mounted?.dispose(); endEdit("page-navigation"); resetStore(); document.body.innerHTML = ""; }
      }
    }
  }, 60_000); // ~2 s here, over 5 s on the hosted Linux runner (full CI 39694278146)
});

describe("timestampText never throws on a timestamp shape it does not know (GH #741)", () => {
  const point = (hour: number, min: number) => ({ active: false, date: { year: 2026, month: 10, day: 21 }, wday: "Wed", time: { hour, min } });
  const ts = (kind: string, date: unknown) => ({ k: "timestamp", ts: kind, date, span: [0, 1] }) as unknown as TimestampInline;
  it("formats a clock start, a stopped clock, a range and a date", () => {
    expect(timestampText(ts("Clock", ["Started", point(8, 54)]))).toEqual({ text: "[2026-10-21 Wed 08:54]", active: false });
    expect(timestampText(ts("Clock", ["Stopped", { start: point(8, 54), stop: point(9, 0) }]))).toEqual({
      text: "[2026-10-21 Wed 08:54--2026-10-21 Wed 09:00]", active: false });
    expect(timestampText(ts("Date", point(8, 54))).text).toBe("[2026-10-21 Wed 08:54]");
  });
  it.each([
    ["null date", null], ["empty object", {}], ["empty array", []], ["unknown pair", ["Paused", {}]],
    ["pair with junk", ["Started", 7]], ["range missing stop", { start: point(1, 2) }], ["date without fields", { date: {} }],
    ["a string", "x"],
  ])("%s", (_name, date) => {
    for (const kind of ["Date", "Range", "Scheduled", "Deadline", "Closed", "Clock", "Other"]) {
      const out = timestampText(ts(kind, date));
      expect(typeof out.text).toBe("string");
      expect(typeof out.active).toBe("boolean");
    }
  });
});

describe("typing DOING keeps the caret on the task line (GH #741)", () => {
  it("text typed after the auto-filled clock-in lands after DOING, not after :END:", () => {
    const { textarea, blockId, dispose } = mountPage("", true);
    try {
      for (const ch of "DOING test") typeChar(textarea!, ch);
      const raw = doc.byId[blockId].raw;
      expect(raw).toMatch(/^DOING test\n:LOGBOOK:\nCLOCK: \[\d{4}-\d\d-\d\d \w{3} \d\d:\d\d:\d\d\]\n:END:$/);
      expect(textarea!.value).toBe("DOING test");
      expect(textarea!.selectionStart).toBe("DOING test".length);
      expect(textarea!.selectionEnd).toBe("DOING test".length);
    } finally {
      dispose();
    }
  });

  it("the same in an Org page", () => {
    const { textarea, blockId, dispose } = mountPage("", true, "org");
    try {
      for (const ch of "DOING test") typeChar(textarea!, ch);
      expect(doc.byId[blockId].raw).toMatch(/^DOING test\n:LOGBOOK:\nCLOCK: \[[^\]]+\]\n:END:$/);
      expect(textarea!.selectionStart).toBe("DOING test".length);
    } finally {
      dispose();
    }
  });

  it("a caret inside the title survives the clock-in of a longer task", () => {
    const { textarea, blockId, dispose } = mountPage("TODO task", true);
    try {
      textarea!.setSelectionRange(0, 4);
      for (const ch of "DOING") typeChar(textarea!, ch); // select "TODO", type over it
      expect(doc.byId[blockId].raw).toMatch(/^DOING task\n:LOGBOOK:\nCLOCK: \[[^\]]+\]\n:END:$/);
      expect(textarea!.selectionStart).toBe(5);
      typeChar(textarea!, "!");
      expect(doc.byId[blockId].raw).toMatch(/^DOING! task\n:LOGBOOK:/);
    } finally {
      dispose();
    }
  });
});

// A marker change made by a command (Ctrl+Enter here; the slash marker, paste and
// format edits share the path) writes the clock-in/out into the store. The
// textarea must take the store's text, or the next keystroke commits the
// pre-transition text and drops the clock change (review R1 of GH #741).
describe("a command marker change keeps its clock change when you keep typing (GH #741)", () => {
  const ctrlEnter = (ta: HTMLTextAreaElement) =>
    ta.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter", ctrlKey: true, bubbles: true, cancelable: true }));
  const flush = () => new Promise<void>((r) => queueMicrotask(r));

  it("DOING to DONE clocks out and stays clocked out", async () => {
    const { textarea, blockId, dispose } = mountPage(`DOING taskA\n:LOGBOOK:\nCLOCK: [2026-10-21 Wed 08:00:00]\n:END:`, true);
    try {
      textarea!.setSelectionRange(11, 11);
      ctrlEnter(textarea!);
      await flush();
      expect(textarea!.value).toBe("DONE taskA");
      typeChar(textarea!, "x");
      expect(doc.byId[blockId].raw).toMatch(/^DONE taskAx\n:LOGBOOK:\nCLOCK: \[2026-10-21 Wed 08:00:00\]--\[[^\]]+\] =>\s+\d\d:\d\d:\d\d\n:END:$/);
    } finally {
      dispose();
    }
  });

  it("TODO to DOING keeps the new clock-in next to an old one", async () => {
    const closed = "CLOCK: [2026-10-20 Tue 08:00:00]--[2026-10-20 Tue 09:00:00] =>  01:00:00";
    const { textarea, blockId, dispose } = mountPage(`TODO taskB\n:LOGBOOK:\n${closed}\n:END:`, true);
    try {
      textarea!.setSelectionRange(10, 10);
      ctrlEnter(textarea!);
      await flush();
      expect(doc.byId[blockId].raw.match(/CLOCK:/g)).toHaveLength(2);
      typeChar(textarea!, "x");
      const raw = doc.byId[blockId].raw;
      expect(raw).toMatch(/^DOING taskBx\n:LOGBOOK:\n/);
      expect(raw.match(/CLOCK:/g)).toHaveLength(2);
    } finally {
      dispose();
    }
  });
});
