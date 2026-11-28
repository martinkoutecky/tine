// A key whose target is a control (not a field) inside an open modal never acts
// on a block, blank, cell or pane selection behind it, and runs no history or
// global command; the modal's own controls and the browser get it uncancelled.
// With no modal open, and in a modal's own fields, keys behave as before.
import { afterEach, beforeAll, describe, expect, it, vi } from "vitest";
import { render } from "solid-js/web";
import type { JSX } from "solid-js";
import { Settings } from "./Settings";
import { Lightbox } from "./Toasts";
import { AudioOverlay } from "./AudioOverlay";
import { CssEditor } from "./CssEditor";
import { QuickSwitcher } from "./QuickSwitcher";
import { RenameDialogFrame } from "./RenameDialogFrame";
import { WelcomeLayer } from "./Welcome";
import { SheetGrid } from "./SheetGrid";
import { SurfaceContext } from "./Block";
import * as ui from "../ui";
import { closeSettings, closeSwitcher, openSettings, openSwitcher, setAudioPlayer, setLightbox, settingsOpen, switcherMode, switcherOpen } from "../ui";
import { initParser } from "../render/parse";
import { loadSingle } from "../document/workingSet";
import { doc, setDoc, type FeedPage, type Node as StoreNode } from "../document/model";
import { blankOutlineSelection, isDirty, node, resetStore, selectBlock, topSelected } from "../document";
import { backend } from "../backend";
import { clearClipboardPayload, peekClipboardPayload } from "../clipboard";
import { editingId, endEdit } from "../editorController";
import { installKeybindings } from "../keybindings";
import { cancelCssEditor, cssEditorSession, openCssEditor } from "../cssEditor";
import { exitPaneSelect, enterPaneSelect, paneSel } from "../paneSelect";
import { focusedPaneId, layoutPaneIds, resetPaneLayoutToSingle, setFocusedPaneId, splitPane } from "../panes";
import { mainPaneRouter, resetTabsToJournals } from "../router";
import { cellSel, resetCellSelectionForTests, setCellRangeSelection } from "../sheet/selection";
import { bindLiveOutlineSelection, bindOutlineSelection, createOutlineOwner } from "../outlineSelectionTarget";
import { clearTransientLayersForTest } from "../transientLayers";
import { MAIN_WINDOW_ID, registerWindow, setActiveWindowId } from "../windowRealm";

beforeAll(() => initParser());

const tick = () => new Promise((resolve) => setTimeout(resolve, 0));
const cleanups: (() => void)[] = [];

afterEach(async () => {
  for (const cleanup of cleanups.splice(0).reverse()) cleanup();
  closeSettings();
  closeSwitcher();
  setLightbox(null);
  setAudioPlayer(null);
  if (cssEditorSession()) await cancelCssEditor();
  exitPaneSelect();
  resetCellSelectionForTests();
  endEdit("page-navigation");
  resetStore();
  clearClipboardPayload();
  resetPaneLayoutToSingle();
  resetTabsToJournals();
  setActiveWindowId(MAIN_WINDOW_ID);
  clearTransientLayersForTest();
  vi.unstubAllGlobals();
  vi.restoreAllMocks();
  document.body.replaceChildren();
});

function mount(view: () => JSX.Element, host: HTMLElement = document.body): HTMLElement {
  const root = host.ownerDocument.createElement("div");
  host.append(root);
  cleanups.push(render(view, root), () => root.remove());
  return root;
}

function keys() {
  cleanups.push(installKeybindings());
}

function press(target: EventTarget, key: string, init: KeyboardEventInit = {}): KeyboardEvent {
  const event = new KeyboardEvent("keydown", { key, bubbles: true, cancelable: true, ...init });
  target.dispatchEvent(event);
  return event;
}

/** Page P: a, b (with children b1, b2, b3), c. All "TODO"-free plain text. */
function loadPage() {
  loadSingle({ name: "P", kind: "page", title: "P", id: "pages/P.md", rev: "r1", format: "md", pre_block: null, blocks: [
    { id: "a", raw: "one", children: [], collapsed: false },
    { id: "b", raw: "two", collapsed: false, children: [
      { id: "b1", raw: "two.one", children: [], collapsed: false },
      { id: "b2", raw: "two.two", children: [], collapsed: false },
      { id: "b3", raw: "two.three", children: [], collapsed: false },
    ] },
    { id: "c", raw: "three", children: [], collapsed: false },
  ] });
}

const outline = () => ({
  roots: [...doc.pages.find((page) => page.name === "P")!.roots],
  b: [...(node("b")?.children ?? [])],
  raws: Object.fromEntries(Object.values(doc.byId).map((n) => [n.id, n.raw])),
  parents: Object.fromEntries(Object.values(doc.byId).map((n) => [n.id, n.parent])),
});

function stubMedia() {
  vi.stubGlobal("matchMedia", () => ({ matches: false, addEventListener() {}, removeEventListener() {} }));
}

/** Settings open on a search for "theme", with focus on its first result. */
async function settingsResult(): Promise<HTMLButtonElement> {
  stubMedia();
  const root = mount(() => <Settings />);
  openSettings();
  await tick();
  const box = root.querySelector<HTMLInputElement>(".settings-search-input")!;
  box.focus();
  box.value = "theme";
  box.dispatchEvent(new InputEvent("input", { bubbles: true }));
  await tick();
  const result = root.querySelector<HTMLButtonElement>(".settings-search-result")!;
  expect(result).toBeTruthy();
  result.focus();
  return result;
}

function clipboardSpies() {
  return [vi.spyOn(backend(), "writeRich").mockResolvedValue(undefined), vi.spyOn(backend(), "writeText").mockResolvedValue(undefined)];
}

describe("a block selection behind Settings", () => {
  it("Backspace on a focused Settings result leaves the selected block, the page and the selection alone", async () => {
    loadPage();
    selectBlock("b");
    keys();
    const result = await settingsResult();
    const before = outline();
    const event = press(result, "Backspace");
    await tick();
    expect(outline()).toEqual(before);
    expect(isDirty("P")).toBe(false);
    expect(topSelected()).toEqual(["b"]);
    expect(event.defaultPrevented).toBe(false);
  });

  it.each<[string, string, KeyboardEventInit, string]>([
    ["Delete", "Delete", {}, "b"],
    ["Mod+X", "x", { ctrlKey: true }, "b"],
    ["Alt+Shift+Up", "ArrowUp", { altKey: true, shiftKey: true }, "b"],
    ["Alt+Shift+Down", "ArrowDown", { altKey: true, shiftKey: true }, "b"],
    ["Tab on the middle sibling", "Tab", {}, "b"],
    ["Shift+Tab on a middle child", "Tab", { shiftKey: true }, "b2"],
    ["cycle-todo", "Enter", { ctrlKey: true }, "b"],
  ])("%s from a Settings result changes nothing", async (_name, key, init, selected) => {
    loadPage();
    selectBlock(selected);
    keys();
    const result = await settingsResult();
    const before = outline();
    const event = press(result, key, init);
    await tick();
    expect(outline()).toEqual(before);
    expect(isDirty("P")).toBe(false);
    expect(topSelected()).toEqual([selected]);
    expect(event.defaultPrevented).toBe(false);
  });

  it("Mod+C from a Settings result does not replace the clipboard", async () => {
    loadPage();
    selectBlock("b");
    keys();
    const writes = clipboardSpies();
    const result = await settingsResult();
    press(result, "c", { ctrlKey: true });
    await tick();
    for (const write of writes) expect(write).not.toHaveBeenCalled();
    expect(peekClipboardPayload()).toBeNull();
  });

  it("Mod+Alt+Down from a Settings result in a vertical split moves neither the selection nor the pane focus", async () => {
    loadPage();
    const top = focusedPaneId();
    splitPane(top, "col", { focusNew: false });
    expect(layoutPaneIds()).toHaveLength(2);
    setFocusedPaneId(top);
    selectBlock("a");
    keys();
    const result = await settingsResult();
    press(result, "ArrowDown", { ctrlKey: true, altKey: true });
    await tick();
    expect(topSelected()).toEqual(["a"]);
    expect(focusedPaneId()).toBe(top);
  });

  it("Enter and Space on a Settings result still reach the result, uncancelled", async () => {
    loadPage();
    selectBlock("b");
    keys();
    for (const key of ["Enter", " "]) {
      const result = await settingsResult();
      const clicked = vi.fn();
      result.addEventListener("keydown", clicked);
      const event = press(result, key);
      expect(clicked).toHaveBeenCalledTimes(1);
      expect(event.defaultPrevented).toBe(false);
      closeSettings();
      for (const cleanup of cleanups.splice(1).reverse()) cleanup();
    }
    expect(topSelected()).toEqual(["b"]);
  });

  it("Escape clears the Settings query, the next closes Settings, and the selection survives both", async () => {
    loadPage();
    selectBlock("b");
    keys();
    const result = await settingsResult();
    const first = press(result, "Escape");
    await tick();
    expect(first.defaultPrevented).toBe(true);
    expect(settingsOpen()).toBe(true);
    expect(document.querySelector<HTMLInputElement>(".settings-search-input")!.value).toBe("");
    const second = press(document.querySelector(".settings-search-input") ?? document.body, "Escape");
    await tick();
    expect(second.defaultPrevented).toBe(true);
    expect(settingsOpen()).toBe(false);
    expect(topSelected()).toEqual(["b"]);
    expect(outline().roots).toEqual(["a", "b", "c"]);
  });

  it("Backspace in the Settings search field is the field's own", async () => {
    loadPage();
    selectBlock("b");
    keys();
    await settingsResult();
    const box = document.querySelector<HTMLInputElement>(".settings-search-input")!;
    box.focus();
    const event = press(box, "Backspace");
    expect(event.defaultPrevented).toBe(false);
    expect(outline().roots).toEqual(["a", "b", "c"]);
    expect(topSelected()).toEqual(["b"]);
  });

  it("Mod+K from a Settings result does not open Search", async () => {
    loadPage();
    selectBlock("b");
    keys();
    const result = await settingsResult();
    press(result, "k", { ctrlKey: true });
    expect(switcherOpen()).toBe(false);
  });

  it("`t o` from a Settings button with nothing selected does not fold the page", async () => {
    loadPage();
    mainPaneRouter.replaceActiveRoute({ kind: "page", name: "P", pageKind: "page" });
    keys();
    const result = await settingsResult();
    press(result, "t");
    press(result, "o");
    await tick();
    expect(node("b")!.collapsed).toBe(false);
  });

  it("Alt+Left from a Settings button does not go back", async () => {
    loadPage();
    mainPaneRouter.replaceActiveRoute({ kind: "page", name: "P", pageKind: "page" });
    mainPaneRouter.openPage("Q", "page");
    keys();
    const result = await settingsResult();
    press(result, "ArrowLeft", { altKey: true });
    await tick();
    expect(mainPaneRouter.route()).toMatchObject({ kind: "page", name: "Q" });
  });
});

describe("other modals", () => {
  it("the rename dialog's button", async () => {
    loadPage();
    selectBlock("b");
    keys();
    mount(() => <RenameDialogFrame id="rename-test" label="Rename" dismiss={() => {}}><button data-choice>Keep</button></RenameDialogFrame>);
    await tick();
    const button = document.querySelector<HTMLButtonElement>("button[data-choice]")!;
    button.focus();
    press(button, "Backspace");
    expect(outline().roots).toEqual(["a", "b", "c"]);
  });

  it("Welcome's Close", async () => {
    loadPage();
    selectBlock("b");
    keys();
    mount(() => <WelcomeLayer mandatory={false} optionalOpen onClose={() => {}} />);
    await tick();
    const close = document.querySelector<HTMLButtonElement>(".welcome-close")!;
    expect(close).toBeTruthy();
    close.focus();
    press(close, "Backspace");
    expect(outline().roots).toEqual(["a", "b", "c"]);
  });

  it("the image viewer's Close", async () => {
    loadPage();
    selectBlock("b");
    keys();
    mount(() => <Lightbox />);
    setLightbox("blob:image");
    await tick();
    const close = document.querySelector<HTMLButtonElement>(".lightbox-close")!;
    close.focus();
    press(close, "Backspace");
    expect(outline().roots).toEqual(["a", "b", "c"]);
  });

  it("expanded audio's player button", async () => {
    loadPage();
    selectBlock("b");
    keys();
    mount(() => <AudioOverlay />);
    setAudioPlayer({ url: "blob:audio", name: "clip.mp3" });
    await tick();
    const button = document.querySelector<HTMLButtonElement>(".audio-btn")!;
    button.focus();
    press(button, "Backspace");
    expect(outline().roots).toEqual(["a", "b", "c"]);
  });

  it("CSS Editor open with the focus left on the page", async () => {
    loadPage();
    selectBlock("b");
    keys();
    mount(() => <CssEditor />);
    await openCssEditor(true);
    await tick();
    expect(cssEditorSession()).not.toBeNull();
    (document.activeElement as HTMLElement | null)?.blur();
    press(document.body, "Backspace");
    expect(outline().roots).toEqual(["a", "b", "c"]);
  });

  it("Mandatory Welcome: Escape on its action button is not cancelled and does not enter pane select", async () => {
    keys();
    mount(() => <WelcomeLayer mandatory optionalOpen={false} onClose={() => {}} />);
    await tick();
    const button = document.querySelector<HTMLButtonElement>(".welcome-choice")!;
    button.focus();
    const event = press(button, "Escape");
    expect(event.defaultPrevented).toBe(false);
    expect(paneSel()).toBeNull();
  });
});

describe("a sheet cell range behind a modal", () => {
  const sheetNode = (id: string, raw: string, parent: string | null, children: string[] = []): StoreNode =>
    ({ id, raw, collapsed: false, parent, page: "Sheet", children });
  const sheetPage: FeedPage = { name: "Sheet", kind: "page", title: "Sheet", preBlock: null, roots: ["grid"], format: "md", readOnly: false, guide: false };
  function loadSheet() {
    setDoc({ byId: {
      grid: sheetNode("grid", "Grid\ntine.view:: grid", null, ["r1", "r2"]),
      r1: sheetNode("r1", "", "grid", ["c1", "c2", "c5"]), c1: sheetNode("c1", "Alpha", "r1"), c2: sheetNode("c2", "Beta", "r1"),
      c5: sheetNode("c5", "Epsilon", "r1"),
      r2: sheetNode("r2", "", "grid", ["c3", "c4", "c6"]), c3: sheetNode("c3", "Gamma", "r2"), c4: sheetNode("c4", "Delta", "r2"),
      c6: sheetNode("c6", "Zeta", "r2"),
    }, pages: [sheetPage], feed: ["Sheet"], loaded: true });
    mount(() => <SurfaceContext.Provider value="main"><SheetGrid id="grid" /></SurfaceContext.Provider>);
    expect(setCellRangeSelection("grid", { row: 0, col: 0 }, { row: 0, col: 1 }, "main")).toBe(true);
  }
  function stubClipboard() {
    const writeText = vi.fn(async () => {});
    const previous = Object.getOwnPropertyDescriptor(navigator, "clipboard");
    Object.defineProperty(navigator, "clipboard", { configurable: true, value: { writeText } });
    cleanups.push(() => {
      if (previous) Object.defineProperty(navigator, "clipboard", previous);
      else Reflect.deleteProperty(navigator, "clipboard");
    });
    return writeText;
  }
  const cells = () => ["c1", "c2", "c3", "c4"].map((id) => doc.byId[id]?.raw);
  function paste(target: EventTarget, text: string): Event {
    const event = new Event("paste", { bubbles: true, cancelable: true });
    Object.defineProperty(event, "clipboardData", { value: { getData: (type: string) => (type === "text/plain" ? text : "") } });
    target.dispatchEvent(event);
    return event;
  }

  it.each<[string, string, KeyboardEventInit]>([
    ["Delete", "Delete", {}], ["Backspace", "Backspace", {}], ["Mod+X", "x", { ctrlKey: true }],
    ["Mod+C", "c", { ctrlKey: true }], ["a printable key", "z", {}], ["an arrow", "ArrowRight", {}], ["F2", "F2", {}],
  ])("%s from a Settings result leaves the cells, the range and the clipboard alone", async (_name, key, init) => {
    loadSheet();
    keys();
    const writeText = stubClipboard();
    const range = cellSel();
    const result = await settingsResult();
    press(result, key, init);
    await tick();
    expect(cells()).toEqual(["Alpha", "Beta", "Gamma", "Delta"]);
    expect(cellSel()).toEqual(range);
    expect(editingId()).toBeNull();
    expect(writeText).not.toHaveBeenCalled();
  });

  it("control: with no modal open, Delete clears the range", () => {
    loadSheet();
    keys();
    press(document.body, "Delete");
    expect(cells()).toEqual(["", "", "Gamma", "Delta"]);
  });

  it("a paste on the page with CSS Editor open leaves the cells alone", async () => {
    loadSheet();
    keys();
    mount(() => <CssEditor />);
    await openCssEditor(true);
    await tick();
    (document.activeElement as HTMLElement | null)?.blur();
    const event = paste(document.body, "X\tY\nZ\tW");
    await tick();
    expect(cells()).toEqual(["Alpha", "Beta", "Gamma", "Delta"]);
    expect(event.defaultPrevented).toBe(false);
  });

  it("a paste in a second window while Settings is open in the first leaves the cells alone", async () => {
    loadSheet();
    const frame = document.createElement("iframe");
    document.body.append(frame);
    cleanups.push(registerWindow("ws-paste", frame.contentWindow!));
    keys();
    await settingsResult();
    const event = paste(frame.contentDocument!.body, "X\tY\nZ\tW");
    await tick();
    expect(cells()).toEqual(["Alpha", "Beta", "Gamma", "Delta"]);
    expect(event.defaultPrevented).toBe(false);
  });
});

describe("pane select behind Settings", () => {
  it.each<[string, string[]]>([["ArrowDown", ["ArrowDown"]], ["Delete", ["Delete"]], ["t o", ["t", "o"]]])(
    "%s from a Settings result leaves the panes, the pane selection and the folds alone", async (_name, sequence) => {
      loadPage();
      mainPaneRouter.replaceActiveRoute({ kind: "page", name: "P", pageKind: "page" });
      const top = focusedPaneId();
      splitPane(top, "col", { focusNew: false });
      const panes = layoutPaneIds();
      enterPaneSelect(top);
      keys();
      const result = await settingsResult();
      const selected = paneSel();
      expect(selected).not.toBeNull();
      for (const key of sequence) press(result, key);
      await tick();
      expect(layoutPaneIds()).toEqual(panes);
      expect(paneSel()).toEqual(selected);
      expect(node("b")!.collapsed).toBe(false);
    });
});

describe("a blank outline selection", () => {
  /** A minimal routed-outline shell: a blank click on it then Mod+A selects every root. */
  function Outline(): JSX.Element {
    const target = { scope: { roots: ["a", "b", "c"] }, surface: "main", page: "P" };
    const owner = createOutlineOwner(target);
    bindLiveOutlineSelection(owner, target);
    return <div class="test-outline" ref={(element) => bindOutlineSelection(element, owner)} />;
  }

  function armBlank(host: HTMLElement) {
    const shell = host.querySelector<HTMLElement>(".test-outline")!;
    shell.click();
    expect(press(shell, "a", { ctrlKey: true }).defaultPrevented).toBe(true);
    expect(blankOutlineSelection()).not.toBeNull();
    return shell;
  }

  it.each<[string, string, KeyboardEventInit]>([["Backspace", "Backspace", {}], ["Mod+X", "x", { ctrlKey: true }], ["Mod+C", "c", { ctrlKey: true }]])(
    "an armed marker is retired when a modal opens; %s afterwards changes nothing", async (_name, key, init) => {
      loadPage();
      keys();
      const writes = clipboardSpies();
      const host = mount(() => <><Outline /><Lightbox /></>);
      const shell = armBlank(host);
      setLightbox("blob:image");
      await tick();
      expect(blankOutlineSelection()).toBeNull();
      press(shell, key, init);
      await tick();
      expect(outline().roots).toEqual(["a", "b", "c"]);
      for (const write of writes) expect(write).not.toHaveBeenCalled();
    });

  it("F8, then Escape that closes Settings, then Backspace: the outline stays", async () => {
    loadPage();
    keys();
    const host = mount(() => <Outline />);
    const shell = armBlank(host);
    const result = await settingsResult();
    press(result, "F8");
    press(result, "Escape");
    press(document.querySelector(".settings-search-input") ?? result, "Escape");
    await tick();
    expect(settingsOpen()).toBe(false);
    press(shell, "Backspace");
    expect(outline().roots).toEqual(["a", "b", "c"]);
  });

  it("with Settings open in main, a blank click and Mod+A in a second window, then Backspace, changes nothing", async () => {
    loadPage();
    const frame = document.createElement("iframe");
    document.body.append(frame);
    cleanups.push(registerWindow("ws-blank", frame.contentWindow!));
    keys();
    const host = mount(() => <Outline />, frame.contentDocument!.body);
    await settingsResult();
    const shell = host.querySelector<HTMLElement>(".test-outline")!;
    shell.click();
    press(shell, "a", { ctrlKey: true });
    press(shell, "Backspace");
    expect(outline().roots).toEqual(["a", "b", "c"]);
  });
});

describe("the switcher's footer chord", () => {
  it("Mod+Shift+P from the Search syntax button switches to Commands; from the input it switches exactly once", async () => {
    keys();
    mount(() => <QuickSwitcher />);
    openSwitcher();
    await tick();
    const syntax = [...document.querySelectorAll<HTMLButtonElement>(".switcher-syntax-toggle")][0];
    syntax.focus();
    const event = press(syntax, "p", { ctrlKey: true, shiftKey: true });
    expect(switcherMode()).toBe("commands");
    expect(event.defaultPrevented).toBe(true);

    closeSwitcher();
    openSwitcher();
    await tick();
    const palette = vi.spyOn(ui, "openCommandPalette");
    const input = document.querySelector<HTMLInputElement>(".switcher-input")!;
    input.focus();
    press(input, "p", { ctrlKey: true, shiftKey: true });
    expect(switcherMode()).toBe("commands");
    expect(palette).toHaveBeenCalledTimes(1);
  });
});

describe("with no modal open, keys behave as before", () => {
  it("Backspace deletes a selected block", () => {
    loadPage();
    selectBlock("b");
    keys();
    press(document.body, "Backspace");
    expect(outline().roots).toEqual(["a", "c"]);
  });

  it("Alt+Shift+Down moves it", () => {
    loadPage();
    selectBlock("a");
    keys();
    press(document.body, "ArrowDown", { altKey: true, shiftKey: true });
    expect(outline().roots).toEqual(["b", "a", "c"]);
  });

  it("Escape climbs from the block selection to pane select", () => {
    loadPage();
    selectBlock("b");
    keys();
    expect(press(document.body, "Escape").defaultPrevented).toBe(true);
    expect(topSelected()).toEqual([]);
    expect(paneSel()).not.toBeNull();
  });

  it("Mod+K opens Search and `t o` folds the page", () => {
    loadPage();
    mainPaneRouter.replaceActiveRoute({ kind: "page", name: "P", pageKind: "page" });
    keys();
    press(document.body, "t");
    press(document.body, "o");
    expect(node("b")!.collapsed).toBe(true);
    press(document.body, "k", { ctrlKey: true });
    expect(switcherOpen()).toBe(true);
  });

  it("Backspace on a block selection with a non-modal layer (a context menu) open deletes it", async () => {
    loadPage();
    selectBlock("b");
    keys();
    const { ContextMenu } = await import("./ContextMenu");
    mount(() => <ContextMenu />);
    ui.openContextMenu(1, 1, "a");
    await tick();
    expect(document.querySelector(".ctx-menu, [role=menu]")).not.toBeNull();
    press(document.body, "Backspace");
    expect(outline().roots).toEqual(["a", "c"]);
  });
});
