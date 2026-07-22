import { mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { afterEach, beforeAll, describe, expect, it, vi } from "vitest";
import { render } from "solid-js/web";
import { backend } from "../backend";
import { editingId, endEdit } from "../editorController";
import { installKeybindings } from "../keybindings";
import { installMobileDrawerMode } from "../mobileDrawers";
import { initParser } from "../render/parse";
import { doc, flushAll, isDirty, loadSingle, pageToDto, resetStore } from "../store";
import { clearTransientLayersForTest } from "../transientLayers";
import type { PageDto } from "../types";
import {
  applySidebarSession,
  closeRightSidebarSafely,
  dismissMobileDrawer,
  rightSidebarOpen,
  setLeftSidebarOpen,
  setRightSidebar,
  setRightSidebarOpen,
  toggleRightSidebar,
} from "../ui";
import { MobileDrawerController } from "./MobileDrawerShell";
import { RightSidebar } from "./RightSidebar";

const PAGE_NAME = "R1 close safety";
const BLOCK_ID = "r1-sidebar-block";

function page(raw = "Original right-sidebar text"): PageDto {
  return {
    name: PAGE_NAME,
    kind: "page",
    title: PAGE_NAME,
    pre_block: null,
    blocks: [{ id: BLOCK_ID, raw, collapsed: false, children: [] }],
  };
}

function mediaQuery(matches: boolean): MediaQueryList {
  return {
    matches,
    media: "(max-width: 639px)",
    onchange: null,
    addListener: vi.fn(),
    removeListener: vi.fn(),
    addEventListener: vi.fn(),
    removeEventListener: vi.fn(),
    dispatchEvent: vi.fn(() => true),
  };
}

let disposeMode = () => {};
const cleanups: Array<() => void> = [];
const tempDirs: string[] = [];

function forceMobileMode(matches: boolean) {
  disposeMode();
  vi.stubGlobal("matchMedia", vi.fn(() => mediaQuery(matches)));
  disposeMode = installMobileDrawerMode();
}

function mountRightSidebar(mobile = false) {
  forceMobileMode(mobile);
  loadSingle(page());
  applySidebarSession({
    left: false,
    right: true,
    items: [{ kind: "block", uuid: BLOCK_ID, page: PAGE_NAME, pageKind: "page" }],
  });
  vi.spyOn(backend(), "getBacklinks").mockResolvedValue([]);
  vi.spyOn(backend(), "getUnlinkedRefs").mockResolvedValue([]);
  vi.spyOn(backend(), "getBlockRefCounts").mockResolvedValue({});

  const root = document.createElement("div");
  document.body.append(root);
  const dispose = render(() => (
    <>
      <button data-r1-toolbar-toggle onClick={() => toggleRightSidebar()}>Toggle right</button>
      <RightSidebar />
      <MobileDrawerController />
    </>
  ), root);
  cleanups.push(dispose);
  return root;
}

async function beginEdit(root: HTMLElement, text: string) {
  const content = await vi.waitFor(() => {
    const found = root.querySelector<HTMLElement>(
      `[data-block-id="${BLOCK_ID}"] .block-content-wrapper`
    );
    expect(found).not.toBeNull();
    return found!;
  });
  content.dispatchEvent(new MouseEvent("mousedown", { bubbles: true, button: 0 }));
  content.dispatchEvent(new MouseEvent("mouseup", { bubbles: true, button: 0 }));
  content.click();
  const editor = await vi.waitFor(() => {
    const found = root.querySelector<HTMLTextAreaElement>(".rs-item-body textarea.block-editor");
    expect(found).not.toBeNull();
    return found!;
  });
  editor.focus();
  editor.value = text;
  editor.setSelectionRange(text.length, text.length);
  editor.dispatchEvent(new InputEvent("input", { bubbles: true, inputType: "insertText" }));
  expect(editingId()).toBe(BLOCK_ID);
  expect(doc.byId[BLOCK_ID].raw).toBe(text);
  return editor;
}

function escape(target: HTMLElement, init: { composing?: boolean; keyCode?: number } = {}) {
  const event = new KeyboardEvent("keydown", { key: "Escape", bubbles: true, cancelable: true });
  if (init.composing) Object.defineProperty(event, "isComposing", { value: true });
  if (init.keyCode != null) Object.defineProperty(event, "keyCode", { value: init.keyCode });
  target.dispatchEvent(event);
  return event;
}

beforeAll(async () => {
  await initParser();
});

afterEach(() => {
  while (cleanups.length) cleanups.pop()!();
  forceMobileMode(false);
  disposeMode();
  disposeMode = () => {};
  endEdit("page-navigation");
  clearTransientLayersForTest();
  setRightSidebar([]);
  applySidebarSession({ left: false, right: false, items: [] });
  resetStore();
  document.body.innerHTML = "";
  localStorage.clear();
  for (const dir of tempDirs.splice(0)) rmSync(dir, { recursive: true, force: true });
  vi.restoreAllMocks();
  vi.unstubAllGlobals();
});

type CloseCase = {
  name: string;
  mobile?: boolean;
  collectionOnly?: boolean;
  close: (root: HTMLElement, editor: HTMLTextAreaElement) => void;
};

const closeCases: CloseCase[] = [
  {
    name: "desktop explicit close",
    close: (root) => root.querySelector<HTMLButtonElement>(".rs-close")!.click(),
  },
  {
    name: "toolbar toggle",
    close: (root) => root.querySelector<HTMLButtonElement>("[data-r1-toolbar-toggle]")!.click(),
  },
  {
    name: "mobile explicit close",
    mobile: true,
    close: (root) => root.querySelector<HTMLButtonElement>(".rs-close")!.click(),
  },
  {
    name: "mobile scrim",
    mobile: true,
    close: (root) => root.querySelector<HTMLElement>("[data-mobile-drawer-scrim]")!.click(),
  },
  {
    name: "plain Escape",
    mobile: true,
    close: (_root, editor) => {
      const uninstall = installKeybindings();
      cleanups.push(uninstall);
      escape(editor);
    },
  },
  {
    name: "Android Back call seam",
    mobile: true,
    close: () => { expect(dismissMobileDrawer("back")).toBe(true); },
  },
  {
    name: "mobile left-drawer replacement",
    mobile: true,
    close: () => setLeftSidebarOpen(true),
  },
  {
    name: "session application",
    close: () => applySidebarSession({ right: false, items: [] }),
  },
  {
    name: "graph-transition collection clear",
    collectionOnly: true,
    close: () => setRightSidebar([]),
  },
  {
    name: "public visibility setter",
    close: () => setRightSidebarOpen(false),
  },
];

describe("GH #161 R1 right-sidebar close safety", () => {
  it.each(closeCases)("prepares the mounted editor before $name unmounts it", async (entry) => {
    const root = mountRightSidebar(!!entry.mobile);
    const text = `Pending edit via ${entry.name}`;
    const editor = await beginEdit(root, text);

    entry.close(root, editor);

    await vi.waitFor(() => expect(root.querySelector(".rs-item-body textarea.block-editor")).toBeNull());
    expect(editingId()).toBeNull();
    expect(doc.byId[BLOCK_ID].raw).toBe(text);
    expect(rightSidebarOpen()).toBe(entry.collectionOnly ? true : false);
  });

  it("peels completion before the drawer, then commits the plain Escape close", async () => {
    const root = mountRightSidebar(true);
    const editor = await beginEdit(root, "/");
    const uninstall = installKeybindings();
    cleanups.push(uninstall);
    await vi.waitFor(() => expect(document.body.querySelector(".autocomplete .ac-item")).not.toBeNull());

    escape(editor);
    await vi.waitFor(() => expect(document.body.querySelector(".autocomplete")).toBeNull());
    expect(rightSidebarOpen()).toBe(true);
    expect(editingId()).toBe(BLOCK_ID);

    escape(editor);
    await vi.waitFor(() => expect(root.querySelector(".right-sidebar")).toBeNull());
    expect(editingId()).toBeNull();
    expect(doc.byId[BLOCK_ID].raw).toBe("/");
  });

  it("leaves composing and legacy keyCode 229 Escape entirely to the IME", async () => {
    const root = mountRightSidebar(true);
    const editor = await beginEdit(root, "Pending IME edit");
    const uninstall = installKeybindings();
    cleanups.push(uninstall);

    expect(escape(editor, { composing: true }).defaultPrevented).toBe(false);
    expect(escape(editor, { keyCode: 229 }).defaultPrevented).toBe(false);
    expect(rightSidebarOpen()).toBe(true);
    expect(editingId()).toBe(BLOCK_ID);
    expect(doc.byId[BLOCK_ID].raw).toBe("Pending IME edit");

    escape(editor);
    await vi.waitFor(() => expect(root.querySelector(".right-sidebar")).toBeNull());
    expect(editingId()).toBeNull();
  });

  it("is idempotent and does not dirty or save a page when the editor did not change", async () => {
    const root = mountRightSidebar(false);
    await beginEdit(root, "Original right-sidebar text");
    const before = pageToDto(PAGE_NAME);
    const save = vi.spyOn(backend(), "savePage");

    root.querySelector<HTMLButtonElement>(".rs-close")!.click();
    expect(closeRightSidebarSafely()).toBe(false);
    expect(isDirty(PAGE_NAME)).toBe(false);
    expect(pageToDto(PAGE_NAME)).toEqual(before);
    await expect(flushAll()).resolves.toBe(true);
    expect(save).not.toHaveBeenCalled();
  });

  it("survives an actual file save and reload after the close boundary", async () => {
    const root = mountRightSidebar(false);
    const dir = mkdtempSync(join(tmpdir(), "tine-r1-close-"));
    tempDirs.push(dir);
    const diskPage = join(dir, "page.json");
    vi.spyOn(backend(), "savePage").mockImplementation(async (dto) => {
      writeFileSync(diskPage, JSON.stringify(dto));
      return "r1-disk-rev";
    });
    const text = "Pending edit persisted across right close";
    await beginEdit(root, text);

    root.querySelector<HTMLButtonElement>("[data-r1-toolbar-toggle]")!.click();
    expect(editingId()).toBeNull();
    await expect(flushAll()).resolves.toBe(true);

    const persisted = JSON.parse(readFileSync(diskPage, "utf8")) as PageDto;
    expect(persisted.blocks[0].raw).toBe(text);
    resetStore();
    loadSingle(persisted);
    expect(doc.byId[BLOCK_ID].raw).toBe(text);
  });
});
