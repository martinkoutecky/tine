import { afterEach, expect, it, vi } from "vitest";
import { render } from "solid-js/web";

const chunk = vi.hoisted(() => ({ release: undefined as undefined | (() => void) }));
vi.mock("./Cm6BlockEditor", async original => {
  await new Promise<void>(resolve => { chunk.release = resolve; });
  return original();
});
vi.mock("../appIdentity", () => ({ APP_UPDATE_CHANNEL: "beta", APP_PRODUCT_NAME: "Tine" }));
vi.unmock("../editor/cm6Setting");

import { Block } from "./Block";
import { preloadCm6Editor } from "./EditorGate";
import { backend } from "../backend";
import { node, resetStore } from "../document";
import { loadSingle } from "../document/workingSet";
import { endEdit, startEditing } from "../editorController";
import { setCm6EditorEnabled } from "../editor/cm6Setting";
import { editorIn, maybeEditorIn } from "../tests/editorDriver";

let dispose: (() => void) | undefined;
afterEach(() => {
  dispose?.(); endEdit("page-navigation"); resetStore();
  setCm6EditorEnabled(false); document.body.replaceChildren(); vi.restoreAllMocks();
});

it("settles a composition abandoned by blur while the first CM6 editor is loading", async () => {
  vi.spyOn(backend(), "setAppBool").mockResolvedValue();
  vi.spyOn(document, "hasFocus").mockReturnValue(false);
  if (!Range.prototype.getClientRects) Range.prototype.getClientRects = () => [] as unknown as DOMRectList;
  if (!Range.prototype.getBoundingClientRect) Range.prototype.getBoundingClientRect = () => new DOMRect();
  loadSingle({ name: "Cold composition", title: "Cold composition", kind: "page", pre_block: null,
    blocks: [{ id: "cold-ime", raw: "hello", collapsed: false, children: [] }] });
  setCm6EditorEnabled(true);
  const host = document.createElement("div"); document.body.append(host);
  dispose = render(() => <Block id="cold-ime" />, host);
  startEditing("cold-ime", 5);
  await vi.waitFor(() => expect(chunk.release).toBeDefined());
  const pending = document.activeElement as HTMLTextAreaElement;
  pending.dispatchEvent(new CompositionEvent("compositionstart", { bubbles: true }));
  pending.value = "漢";
  chunk.release!();
  await preloadCm6Editor();
  expect(maybeEditorIn(host)).toBeNull();
  pending.dispatchEvent(new FocusEvent("blur"));
  await vi.waitFor(() => expect(maybeEditorIn(host)?.value()).toBe("hello漢"));
  const editor = editorIn(host);
  expect(editor.kind).toBe("cm6");
  expect(editor.selection()).toEqual([6, 6]);
  pending.value = "漢";
  pending.dispatchEvent(new CompositionEvent("compositionend", { bubbles: true, data: "漢" }));
  editor.type("!");
  expect(editor.value()).toBe("hello漢!");
  expect(node("cold-ime").raw).toBe("hello漢!");
});
