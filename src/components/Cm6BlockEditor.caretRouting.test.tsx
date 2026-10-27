import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { For } from "solid-js";
import { render } from "solid-js/web";

// Own the real preference even in the opt-in CM6 census.
vi.mock("../appIdentity", () => ({ APP_UPDATE_CHANNEL: "beta", APP_PRODUCT_NAME: "Tine" }));
vi.unmock("../editor/cm6Setting");

import { Block, SurfaceContext } from "./Block";
import { RightSidebar } from "./RightSidebar";
import { backend } from "../backend";
import { node, pageByName, resetStore } from "../document";
import { loadSingle } from "../document/workingSet";
import { editingId, editingOwner, endEdit, noteSurfaceFocused, startEditing } from "../editorController";
import { cm6EditorEnabled, setCm6EditorEnabled } from "../editor/cm6Setting";
import { editorIn, editorsIn, type EditorKind } from "../tests/editorDriver";
import { applySidebarSession } from "../ui";

let dispose: (() => void) | undefined;
const flush = () => new Promise<void>(resolve => queueMicrotask(resolve));

beforeEach(() => {
  // A focused app window; jsdom supplies no visual-row geometry.
  vi.spyOn(document, "hasFocus").mockReturnValue(true);
  if (!Range.prototype.getClientRects) Range.prototype.getClientRects = () => [] as unknown as DOMRectList;
  if (!Range.prototype.getBoundingClientRect) Range.prototype.getBoundingClientRect = () => new DOMRect();
  vi.spyOn(backend(), "setAppBool").mockResolvedValue();
  vi.spyOn(backend(), "getBacklinks").mockResolvedValue([]);
  vi.spyOn(backend(), "getUnlinkedRefs").mockResolvedValue([]);
  vi.spyOn(backend(), "getBlockRefCounts").mockResolvedValue({});
});

afterEach(async () => {
  dispose?.(); dispose = undefined;
  endEdit("page-navigation");
  applySidebarSession({ right: false, items: [] });
  resetStore();
  setCm6EditorEnabled(false);
  await flush();
  document.body.replaceChildren();
  vi.restoreAllMocks();
});

async function mount(kind: EditorKind, targetRaw = "abcdefghij") {
  setCm6EditorEnabled(kind === "cm6");
  await flush();
  expect(cm6EditorEnabled()).toBe(kind === "cm6");
  loadSingle({ name: "Caret twins", title: "Caret twins", kind: "page", format: "md", pre_block: null,
    blocks: ["a", "b"].map(id => ({ id, raw: id === "b" ? targetRaw : "abcdefghij", collapsed: false, children: [] })) });
  applySidebarSession({ right: true, items: [{ kind: "page", name: "Caret twins", pageKind: "page" }] });
  const host = document.createElement("div"); document.body.append(host);
  // Main mounts first, matching an already open page subsequently mirrored in the sidebar.
  dispose = render(() => <>
    <section data-caret-main><SurfaceContext.Provider value="main">
      <For each={pageByName("Caret twins")?.roots}>{id => <Block id={id} />}</For>
    </SurfaceContext.Provider></section>
    <RightSidebar />
  </>, host);
  await flush();
  return host;
}

it("CM6's fallback focus applies the pending selection when the stamped surface is absent", async () => {
  setCm6EditorEnabled(true);
  await flush();
  loadSingle({ name: "Fallback", title: "Fallback", kind: "page", pre_block: null,
    blocks: [{ id: "fallback", raw: "abcdefghij", collapsed: false, children: [] }] });
  noteSurfaceFocused("sidebar");
  startEditing("fallback", { start: 2, end: 5, direction: "backward" });
  const host = document.createElement("div"); document.body.append(host);
  dispose = render(() => <Block id="fallback" />, host);
  await flush();
  const editor = editorIn(host);
  expect(editor.kind).toBe("cm6");
  expect(document.activeElement).toBe(editor.element);
  expect(editor.selection()).toEqual([2, 5]);
  expect(editor.direction()).toBe("backward");
});

it.each(["numeric", "range"] as const)("CM6 maps a %s caret into a mixed code body only in the focused occurrence", async kind => {
  const raw = "before\n```js\ncode\n```\nafter";
  const host = await mount("cm6", raw);
  const sidebar = host.querySelector("[data-sidebar-surface]")!;
  await clickBlock(sidebar, "a");
  const start = raw.indexOf("code") + 1;
  startEditing("b", kind === "numeric" ? start : { start, end: start + 2, direction: "backward" });
  await flush();
  const focused = editorIn(sidebar.querySelector('[data-block-id="b"]')!);
  const main = editorIn(host.querySelector('[data-caret-main] [data-block-id="b"]')!);
  expect(document.activeElement).toBe(focused.element);
  expect(focused.value()).toBe("code");
  expect(focused.selection()).toEqual(kind === "numeric" ? [1, 1] : [1, 3]);
  if (kind === "range") expect(focused.direction()).toBe("backward");
  expect(main.value()).toBe(raw);
  expect(node("b").raw).toBe(raw);
});

async function clickBlock(surface: Element, id: string) {
  surface.querySelector(`[data-block-id="${id}"] .block-content-wrapper`)!
    .dispatchEvent(new MouseEvent("mousedown", { bubbles: true, button: 0, cancelable: true }));
  document.dispatchEvent(new MouseEvent("mouseup", { bubbles: true, button: 0 }));
  await flush();
  return editorIn(surface.querySelector(`[data-block-id="${id}"]`)!);
}

for (const kind of ["cm6", "textarea"] as const) {
  it.each([
    { key: "ArrowDown", from: "a", to: "b" },
    { key: "ArrowUp", from: "b", to: "a" },
  ])(`${kind}: $key keeps the requested column in the focused sidebar occurrence (UI-EDITOR-C3-CM6)`, async ({ key, from, to }) => {
    const host = await mount(kind);
    const sidebar = host.querySelector("[data-sidebar-surface]")!;
    const source = await clickBlock(sidebar, from);
    expect(source.kind).toBe(kind);
    expect(document.activeElement).toBe(source.element);
    source.select(2);
    expect(source.keydown({ key }).defaultPrevented).toBe(true);
    await flush();

    expect(editingId()).toBe(to);
    expect(editingOwner()).toBeNull();
    const focused = editorIn(sidebar.querySelector(`[data-block-id="${to}"]`)!);
    const main = editorIn(host.querySelector(`[data-caret-main] [data-block-id="${to}"]`)!);
    expect(editorsIn(host)).toHaveLength(2);
    expect(document.activeElement).toBe(focused.element);
    expect(main.element).not.toBe(document.activeElement);
    expect(focused.selection()).toEqual([2, 2]);
    expect(main.selection()).not.toEqual([2, 2]);
    expect(node("a").raw).toBe("abcdefghij");
    expect(node("b").raw).toBe("abcdefghij");
  });

  it.each([
    { key: "ArrowLeft", from: "b", at: 0, targetIndex: 0, caret: 10, text: "abcdefghij" },
    { key: "ArrowRight", from: "a", at: 10, targetIndex: 1, caret: 0, text: "abcdefghij" },
    { key: "Enter", from: "a", at: 2, targetIndex: 1, caret: 0, text: "cdefghij" },
    { key: "Backspace", from: "b", at: 0, targetIndex: 0, caret: 10, text: "abcdefghijabcdefghij" },
  ])(`${kind}: $key routes a boundary or structural caret to the sidebar`, async ({ key, from, at, targetIndex, caret, text }) => {
    const host = await mount(kind);
    const sidebar = host.querySelector("[data-sidebar-surface]")!;
    const source = await clickBlock(sidebar, from);
    expect(source.kind).toBe(kind);
    source.select(at);
    expect(source.keydown({ key }).defaultPrevented).toBe(true);
    await flush();

    const target = pageByName("Caret twins")!.roots[targetIndex];
    expect(editingId()).toBe(target);
    const focused = editorIn(sidebar.querySelector(`[data-block-id="${target}"]`)!);
    expect(document.activeElement).toBe(focused.element);
    expect(focused.value()).toBe(text);
    expect(focused.selection()).toEqual([caret, caret]);
    expect(node(target).raw).toBe(text);
    expect(editorsIn(host)).toHaveLength(2);
  });
}
