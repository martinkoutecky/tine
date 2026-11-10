import { afterEach, expect, it, vi } from "vitest";
import { render } from "solid-js/web";
import { For } from "solid-js";

const chunk = vi.hoisted(() => ({ requested: vi.fn(), hold: false, release: undefined as undefined | (() => void) }));
vi.mock("./Cm6BlockEditor", async original => {
  chunk.requested();
  if (chunk.hold) await new Promise<void>(resolve => { chunk.release = resolve; });
  return original();
});
vi.mock("../appIdentity", () => ({ APP_UPDATE_CHANNEL: "beta", APP_PRODUCT_NAME: "Tine" }));
vi.unmock("../editor/cm6Setting");

import { Block, SurfaceContext } from "./Block";
import { backend } from "../backend";
import { node, pageByName, resetStore } from "../document";
import { loadSingle } from "../document/workingSet";
import { endEdit, noteSurfaceFocused, startEditing } from "../editorController";
import { setCm6EditorEnabled } from "../editor/cm6Setting";
import { editorIn, maybeEditorIn } from "../tests/editorDriver";
import { preloadCm6Editor } from "./EditorGate";

let dispose: (() => void) | undefined;
afterEach(() => {
  dispose?.(); endEdit("page-navigation"); resetStore();
  setCm6EditorEnabled(false); document.body.replaceChildren(); vi.restoreAllMocks();
});

it("does not request CM6 while off; the first cold edit keeps the clicked caret and early typing", async () => {
  vi.spyOn(backend(), "setAppBool").mockResolvedValue();
  vi.spyOn(document, "hasFocus").mockReturnValue(true);
  if (!Range.prototype.getClientRects) Range.prototype.getClientRects = () => [] as unknown as DOMRectList;
  if (!Range.prototype.getBoundingClientRect) Range.prototype.getBoundingClientRect = () => new DOMRect();
  loadSingle({ name: "Cold editor", title: "Cold editor", kind: "page", pre_block: null,
    blocks: [{ id: "cold", raw: "hello world", collapsed: false, children: [] }] });
  const host = document.createElement("div"); document.body.append(host);
  dispose = render(() => <>
    <section data-main><SurfaceContext.Provider value="main"><For each={pageByName("Cold editor")?.roots}>{id => <Block id={id} />}</For></SurfaceContext.Provider></section>
    <section data-sidebar><SurfaceContext.Provider value="sidebar"><For each={pageByName("Cold editor")?.roots}>{id => <Block id={id} />}</For></SurfaceContext.Provider></section>
  </>, host);
  noteSurfaceFocused("main");
  startEditing("cold", 5);
  expect(editorIn(host).kind).toBe("textarea");
  expect(chunk.requested).not.toHaveBeenCalled();
  endEdit("page-navigation");
  chunk.hold = true;
  setCm6EditorEnabled(true);
  noteSurfaceFocused("sidebar");
  startEditing("cold", 5);
  await vi.waitFor(() => expect(chunk.requested).toHaveBeenCalledTimes(1));
  expect(maybeEditorIn(host)).toBeNull();
  const pending = document.activeElement as HTMLTextAreaElement;
  expect(pending.tagName).toBe("TEXTAREA");
  pending.value = "!";
  pending.dispatchEvent(new InputEvent("input", { bubbles: true, data: "!", inputType: "insertText" }));
  pending.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter", bubbles: true, cancelable: true }));
  pending.value = "tail";
  pending.dispatchEvent(new InputEvent("input", { bubbles: true, data: "tail", inputType: "insertText" }));
  pending.dispatchEvent(new CompositionEvent("compositionstart", { bubbles: true }));
  pending.value = "【";
  chunk.release!();
  // Keep the native preedit host until composition commits, even if loading finishes.
  await preloadCm6Editor();
  expect(maybeEditorIn(host.querySelector("[data-sidebar]"))).toBeNull();
  expect(document.activeElement).toBe(pending);
  pending.dispatchEvent(new CompositionEvent("compositionend", { bubbles: true, data: "【" }));
  await vi.waitFor(() => expect(node(pageByName("Cold editor")!.roots[1])?.raw).toBe("tail【 world"));
  const editor = editorIn(host.querySelector("[data-sidebar]")!);
  expect(editor.kind).toBe("cm6");
  expect(document.activeElement).toBe(editor.element);
  expect(editor.value()).toBe("tail【 world");
  expect(editor.selection()).toEqual([5, 5]);
  expect(node("cold").raw).toBe("hello!");
  expect(chunk.requested).toHaveBeenCalledTimes(1);
});
