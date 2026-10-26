import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { render } from "solid-js/web";
import { Cm6BlockEditor } from "./Cm6BlockEditor";
import { CaptureCtx, Editor, type CaptureApi } from "./Block";
import { backend } from "../backend";
import { isDirty, node, pageByName, resetStore } from "../document";
import { loadSingle } from "../document/workingSet";
import { endEdit, startEditing } from "../editorController";
import { editorIn, type EditorKind } from "../tests/editorDriver";
import { commandDefaults, installKeybindings } from "../keybindings";
import { setSpellcheckEnabled, spellcheckEnabled } from "../spellcheckSettings";
import { blockRegions } from "../render/parse";
import { setAutoPairing, setTypographyMode } from "../ui";
import * as cm6Commands from "../editor/cm6Commands";

// These paired scenarios originated in the independent milestone review. Both
// editors are real mounts; CaptureCtx and configured key dispatch stay live.
let dispose: (() => void) | undefined;
let uninstall: (() => void) | undefined;
beforeEach(() => {
  if (!Range.prototype.getClientRects) Range.prototype.getClientRects = () => [] as unknown as DOMRectList;
  if (!Range.prototype.getBoundingClientRect) Range.prototype.getBoundingClientRect = () => new DOMRect();
  setAutoPairing(false);
  setTypographyMode("render");
  vi.spyOn(backend(), "applySpellcheck").mockResolvedValue(undefined);
  vi.spyOn(backend(), "setAppBool").mockResolvedValue(undefined);
  setSpellcheckEnabled(true);
  uninstall = installKeybindings();
});
afterEach(async () => {
  dispose?.(); dispose = undefined;
  uninstall?.(); uninstall = undefined;
  endEdit("page-navigation"); resetStore(); document.body.innerHTML = "";
  setSpellcheckEnabled(true);
  // Let preference writes finish before restoring the fake backend boundary.
  await Promise.resolve(); await Promise.resolve();
  vi.restoreAllMocks();
});

function remap(id: string) {
  uninstall?.();
  uninstall = installKeybindings({ [id]: "alt+shift+k" });
}

function mount(kind: EditorKind, raw = "captured thought", options: { capture?: boolean; enterFiles?: boolean; neighbors?: boolean } = {}) {
  loadSingle({ name: "Parity", title: "Parity", kind: "page", pre_block: null, blocks: [
    ...(options.neighbors ? [{ id: "previous", raw: "previous", collapsed: false, children: [] }] : []),
    { id: "parity", raw, collapsed: false, children: [],
      properties: blockRegions(raw, "md").properties.filter(p => p.primary).map(p => [p.key, p.value] as [string, string]) },
    ...(options.neighbors ? [{ id: "next", raw: "next", collapsed: false, children: [] }] : []),
  ] });
  startEditing("parity", raw.length);
  const root = document.createElement("div"); document.body.append(root);
  const api = { submit: vi.fn(), cancel: vi.fn(), enterFiles: () => options.enterFiles ?? true, quickSwitch: vi.fn<CaptureApi["quickSwitch"]>(async () => []) };
  dispose = render(() => <CaptureCtx.Provider value={options.capture ? api : null}>
    {kind === "cm6" ? <Cm6BlockEditor id="parity" surface="main" scope={null} /> : <Editor id="parity" />}
  </CaptureCtx.Provider>, root);
  return { editor: editorIn(root), api };
}

const highlight = "Highlighted text\nid:: 12345678-1234-1234-1234-123456789abc\nls-type:: annotation\nhl-page:: 2\nhl-color:: yellow";
const motions = [
  { id: "editor/clear-block", key: "l", text: "", caret: 0 },
  { id: "editor/kill-line-before", key: "u", text: "second", caret: 0 },
  { id: "editor/kill-line-after", key: "k", text: "first ", caret: 6 },
  { id: "editor/backward-word", key: "b", text: "first second", caret: 0 },
  { id: "editor/forward-word", key: "f", text: "first second", caret: 12 },
  { id: "editor/backward-kill-word", key: "w", text: "second", caret: 0 },
  { id: "editor/forward-kill-word", key: "d", text: "first ", caret: 6 },
];

describe.each<EditorKind>(["textarea", "cm6"])("editor parity: %s", kind => {
  it.each([false, true])("Enter preserves highlight text, hidden metadata and identity (selection=%s)", selected => {
    const { editor } = mount(kind, highlight, { neighbors: true });
    expect(editor.value()).toBe("Highlighted text");
    editor.select(5, selected ? 10 : 5);
    editor.keydown({ key: "Enter" });
    const roots = pageByName("Parity")!.roots;
    expect(roots.slice(0, 2)).toEqual(["previous", "parity"]);
    expect(roots[3]).toBe("next");
    expect(roots).toHaveLength(4);
    expect(node("parity").raw).toBe(highlight);
    expect(node(roots[2]).raw).toBe("");
    expect(node("previous").raw).toBe("previous");
    expect(node("next").raw).toBe("next");
  });

  it.each(["Backspace", "Delete"])("%s at a highlight boundary preserves all adjacent blocks", key => {
    const { editor } = mount(kind, highlight, { neighbors: true });
    editor.select(key === "Backspace" ? 0 : editor.value().length);
    editor.keydown({ key });
    expect(pageByName("Parity")!.roots).toEqual(["previous", "parity", "next"]);
    expect(node("parity").raw).toBe(highlight);
    expect(node("previous").raw).toBe("previous");
    expect(node("next").raw).toBe("next");
    expect(isDirty("Parity")).toBe(false);
  });

  it("files on plain Enter in Enter-files mode", () => {
    const { editor, api } = mount(kind, undefined, { capture: true });
    editor.input("file this thought");
    editor.keydown({ key: "Enter" });
    expect(api.submit).toHaveBeenCalledOnce();
    expect(node("parity").raw).toBe("file this thought");
    expect(pageByName("Parity")!.roots).toEqual(["parity"]);
  });

  it("keeps ordinary Enter splitting in new-block mode", () => {
    const { editor, api } = mount(kind, "first second", { capture: true, enterFiles: false });
    editor.select(6);
    editor.keydown({ key: "Enter" });
    expect(api.submit).not.toHaveBeenCalled();
    const roots = pageByName("Parity")!.roots;
    expect(roots).toHaveLength(2);
    expect(roots.map(id => node(id).raw)).toEqual(["first ", "second"]);
  });

  it.each([false, true])("files with the default chord in either Enter mode (enterFiles=%s)", enterFiles => {
    const { editor, api } = mount(kind, undefined, { capture: true, enterFiles });
    editor.keydown({ key: "Enter", ctrlKey: true, shiftKey: true });
    expect(api.submit).toHaveBeenCalledOnce();
    expect(pageByName("Parity")!.roots).toEqual(["parity"]);
  });

  it.each([false, true])("honors a remapped file chord in either Enter mode (enterFiles=%s)", enterFiles => {
    remap("editor/quick-capture-file");
    const { editor, api } = mount(kind, undefined, { capture: true, enterFiles });
    editor.keydown({ key: "K", altKey: true, shiftKey: true });
    expect(api.submit).toHaveBeenCalledOnce();
    expect(pageByName("Parity")!.roots).toEqual(["parity"]);
  });

  it("cancels the capture on Escape", () => {
    const { editor, api } = mount(kind, undefined, { capture: true });
    editor.keydown({ key: "Escape" });
    expect(api.cancel).toHaveBeenCalledOnce();
  });

  it.each(["Enter", "Escape"])("gives completion ownership of %s before capture intent", async key => {
    const { editor, api } = mount(kind, "", { capture: true });
    api.quickSwitch.mockResolvedValue([{ name: "Target", kind: "page", date_key: null, path: "pages/Target.md" }]);
    editor.input("[[Tar]]", 5);
    await vi.waitFor(() => expect(document.querySelector(".autocomplete .ac-item")).not.toBeNull());
    editor.keydown({ key });
    expect(api.submit).not.toHaveBeenCalled();
    expect(api.cancel).not.toHaveBeenCalled();
    if (key === "Enter") expect(editor.value().trim()).toBe("[[Target]]");
    else expect(document.querySelector(".autocomplete")).toBeNull();
  });

  it.each(["Enter", "Escape"])("leaves composing/229 %s to the IME", key => {
    const { editor, api } = mount(kind, undefined, { capture: true });
    editor.keydown({ key }, { isComposing: true });
    editor.keydown({ key }, { keyCode: 229 });
    editor.keydown({ key: "Enter", ctrlKey: true, shiftKey: true }, { isComposing: true });
    expect(api.submit).not.toHaveBeenCalled();
    expect(api.cancel).not.toHaveBeenCalled();
    expect(node("parity").raw).toBe("captured thought");
    expect(pageByName("Parity")!.roots).toEqual(["parity"]);
  });

  it.each(motions.flatMap(motion => [false, true].map(remapped => ({ ...motion, remapped }))))(
    "dispatches $id with its configured binding (remapped=$remapped)", async ({ id, key, text, caret, remapped }) => {
      if (remapped) remap(id);
      const { editor } = mount(kind, "first second");
      editor.select(6);
      editor.keydown(remapped ? { key: "K", altKey: true, shiftKey: true } : { key, altKey: true });
      await Promise.resolve(); // textarea restores its buffer/selection after commit
      expect(editor.value()).toBe(text);
      expect(node("parity").raw).toBe(text);
      expect(editor.selection()).toEqual([caret, caret]);
      if (id === "editor/backward-word" || id === "editor/forward-word") expect(isDirty("Parity")).toBe(false);
    },
  );

  it("binds spellcheck initially and reacts to preference changes on the same editor", async () => {
    const { editor } = mount(kind);
    expect(spellcheckEnabled()).toBe(true);
    expect(editor.element.getAttribute("spellcheck")).toBe("true");
    setSpellcheckEnabled(false);
    expect(editor.element.getAttribute("spellcheck")).toBe("false");
    setSpellcheckEnabled(true);
    expect(editor.element.getAttribute("spellcheck")).toBe("true");
    expect(editor.value()).toBe("captured thought");
  });
});

it("registers a CM6 handler for every editor-scoped command", () => {
  const registration = vi.spyOn(cm6Commands, "cm6CommandHandlers");
  mount("cm6");
  const registry = registration.mock.results[0].value as ReturnType<typeof cm6Commands.cm6CommandHandlers>;
  const missing = commandDefaults().filter(command => command.scope === "editor" && typeof registry[command.id] !== "function");
  expect(missing.map(command => command.id)).toEqual([]);
});
