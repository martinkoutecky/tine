// GH #1012: a date picked on a block without title text is written after an
// empty first line; the next character typed goes into that title line, in both
// editors, with hidden rows and with the block shown twice.
import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from "vitest";
import { render } from "solid-js/web";
import { Block, SurfaceContext } from "./Block";
import { DatePicker } from "./DatePicker";
import { resetStore } from "../document";
import { doc } from "../document/model";
import { loadSingle } from "../document/workingSet";
import { endEdit, startEditing } from "../editorController";
import { setCm6EditorEnabled } from "../editor/cm6Setting";
import { preloadCm6Editor } from "./EditorGate";
import { editorIn } from "../tests/editorDriver";
import { initParser } from "../render/parse";
import { closeDatePicker, openDatePicker } from "../ui";
import { clearTransientLayersForTest } from "../transientLayers";

beforeAll(async () => { await initParser(); await preloadCm6Editor(); });

const LB = ":LOGBOOK:\nCLOCK: [2026-10-01 Wed 10:00]--[2026-10-01 Wed 10:05] =>  00:05:00\n:END:";
let dispose: (() => void) | undefined;

function mount(raw: string, twice: boolean) {
  loadSingle({ name: "P", kind: "page", title: "P", format: "md", pre_block: null,
    blocks: [{ id: "b", raw, collapsed: false, children: [] }] });
  startEditing("b", 0);
  const root = document.createElement("div");
  document.body.append(root);
  const main = render(() => <><Block id="b" /><DatePicker /></>, root);
  let mirror: (() => void) | undefined;
  if (twice) {
    const other = document.createElement("div");
    document.body.append(other);
    mirror = render(() => <SurfaceContext.Provider value="sidebar:1012"><Block id="b" /></SurfaceContext.Provider>, other);
  }
  dispose = () => { mirror?.(); main(); };
  const editor = editorIn(root, "b");
  editor.focus();
  return editor;
}

describe.each(["textarea", "cm6"] as const)("%s: typing after a date picked on a block without title text", kind => {
  beforeEach(() => setCm6EditorEnabled(kind === "cm6"));
  afterEach(() => {
    dispose?.(); dispose = undefined;
    closeDatePicker(); endEdit("page-navigation"); resetStore(); clearTransientLayersForTest();
    document.body.innerHTML = ""; vi.restoreAllMocks();
  });

  it.each([
    ["an empty block", "", false],
    ["a hidden logbook", LB, false],
    ["a hidden id", "id:: 6679a1b2-0000-4000-8000-000000000001", false],
    ["the block shown twice", "", true],
  ] as const)("%s: the character goes into the title line", async (_name, raw, twice) => {
    vi.spyOn(document, "hasFocus").mockReturnValue(true);
    const editor = mount(raw, twice);
    openDatePicker("b", "scheduled", 0, 0);
    await vi.waitFor(() => expect(document.activeElement?.closest(".date-picker")).not.toBeNull());
    document.activeElement!.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter", bubbles: true, cancelable: true }));
    const dated = doc.byId.b.raw;
    expect(dated).toMatch(/^\nSCHEDULED: <\d{4}-\d\d-\d\d [A-Z][a-z]{2}>/);
    expect(dated.endsWith(raw)).toBe(true);
    await vi.waitFor(() => expect(document.activeElement).toBe(editor.element));
    editor.type("x");
    expect(doc.byId.b.raw).toBe(`x${dated}`);
  });
});
