import { afterEach, beforeAll, expect, it, vi } from "vitest";
import { render } from "solid-js/web";
import { Block, SurfaceContext } from "./Block";
import { RightSidebar } from "./RightSidebar";
import { editorIn, editorsIn, type EditorDriver } from "../tests/editorDriver";
import { editingId, editingOwner, endEdit } from "../editorController";
import { resetStore } from "../document";
import { doc } from "../document/model";
import { loadSingle } from "../document/workingSet";
import { backend } from "../backend";
import { applySidebarSession, setRightSidebar, rightSidebarOpen } from "../ui";
import { installKeybindings } from "../keybindings";
import { initParser } from "../render/parse";

const cleanups: (() => void)[] = [];
const flush = () => new Promise<void>(resolve => queueMicrotask(resolve));
const text = "abcdefghij";

beforeAll(() => initParser());
afterEach(() => {
  for (const dispose of cleanups.splice(0).reverse()) dispose();
  endEdit("page-navigation");
  setRightSidebar([]);
  applySidebarSession({ right: false, items: [] });
  resetStore();
  document.body.replaceChildren();
  vi.restoreAllMocks();
});

function click(el: Element) {
  el.dispatchEvent(new MouseEvent("mousedown", { bubbles: true, button: 0, cancelable: true }));
  document.dispatchEvent(new MouseEvent("mouseup", { bubbles: true, button: 0 }));
}

async function setup() {
  // A focused app window; neither editor focus nor blur is stubbed.
  vi.spyOn(document, "hasFocus").mockReturnValue(true);
  loadSingle({ name: "Late mount", title: "Late mount", kind: "page", format: "md", pre_block: null,
    blocks: ["a", "b"].map(id => ({ id, raw: text, collapsed: false, children: [] })) });
  applySidebarSession({ right: false, items: [{ kind: "page", name: "Late mount", pageKind: "page" }] });
  vi.spyOn(backend(), "getBacklinks").mockResolvedValue([]);
  vi.spyOn(backend(), "getUnlinkedRefs").mockResolvedValue([]);
  vi.spyOn(backend(), "getBlockRefCounts").mockResolvedValue({});
  const host = document.createElement("div");
  document.body.append(host);
  cleanups.push(render(() => <>
    <section data-main><SurfaceContext.Provider value="main"><Block id="a" /><Block id="b" /></SurfaceContext.Provider></section>
    <button data-outside>Outside</button>
    <RightSidebar />
  </>, host));
  cleanups.push(installKeybindings({ "ui/toggle-right-sidebar": "alt+shift+r" }));
  await flush();
  click(host.querySelector('[data-main] [data-block-id="a"] .block-content-wrapper')!);
  await flush();
  const a = editorIn(host.querySelector('[data-main] [data-block-id="a"]')!);
  a.select(2);
  a.keydown({ key: "ArrowDown", bubbles: true, cancelable: true });
  await flush();
  expect(editingId()).toBe("b");
  expect(editingOwner()).toBeNull();
  const main = editorIn(host.querySelector('[data-main] [data-block-id="b"]')!);
  expect(document.activeElement).toBe(main.element);
  return { host, main };
}

async function toggleSidebar(editor: EditorDriver) {
  editor.keydown({ key: "R", code: "KeyR", altKey: true, shiftKey: true, bubbles: true, cancelable: true });
  await flush();
  await flush();
}

function expectTextUnchanged(host: HTMLElement) {
  expect(doc.byId.a.raw).toBe(text);
  expect(doc.byId.b.raw).toBe(text);
  for (const editor of editorsIn(host)) expect(editor.value()).toBe(text);
}

it("UI-EDITOR-LATE-MOUNT: the sidebar shortcut keeps editing and the main caret when a duplicate mounts", async () => {
  const { host, main } = await setup();
  await toggleSidebar(main);
  expect(rightSidebarOpen()).toBe(true);
  expect(editingId()).toBe("b");
  expect(document.activeElement).toBe(main.element);
  expect(main.selection()).toEqual([2, 2]);
  expectTextUnchanged(host);
});

it.each(["Escape", "click-away", "page-navigation", "sidebar-close"] as const)(
  "still exits editing on %s after opening the duplicate sidebar", async reason => {
    const { host, main } = await setup();
    await toggleSidebar(main);
    expect(editingId()).toBe("b");
    if (reason === "Escape") main.keydown({ key: "Escape", bubbles: true, cancelable: true });
    if (reason === "click-away") host.querySelector<HTMLButtonElement>("[data-outside]")!.focus();
    if (reason === "page-navigation") endEdit("page-navigation");
    if (reason === "sidebar-close") {
      const side = editorIn(host.querySelector("[data-sidebar-surface]")!);
      side.focus();
      await flush();
      expect(editingId()).toBe("b");
      expect(document.activeElement).toBe(side.element);
      await toggleSidebar(side);
      expect(rightSidebarOpen()).toBe(false);
    }
    await flush();
    expect(editingId()).toBeNull();
    expect(editorsIn(host)).toHaveLength(0);
    expectTextUnchanged(host);
  },
);
