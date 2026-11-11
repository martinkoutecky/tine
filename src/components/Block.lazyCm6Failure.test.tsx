import { afterEach, expect, it, vi } from "vitest";
import { render } from "solid-js/web";

const chunk = vi.hoisted(() => ({ reject: undefined as undefined | ((error: Error) => void), report: vi.fn() }));
vi.mock("./Cm6BlockEditor", async () => {
  await new Promise((_, reject) => { chunk.reject = reject; });
  throw new Error("unreachable");
});
vi.mock("../appIdentity", () => ({ APP_UPDATE_CHANNEL: "beta", APP_PRODUCT_NAME: "Tine" }));
vi.unmock("../editor/cm6Setting");
vi.mock("../uiFailure", () => ({ reportUiFailure: chunk.report }));

import { Block } from "./Block";
import { backend } from "../backend";
import { node, resetStore } from "../document";
import { loadSingle } from "../document/workingSet";
import { endEdit, startEditing } from "../editorController";
import { setCm6EditorEnabled } from "../editor/cm6Setting";
import { editorIn } from "../tests/editorDriver";
import { preloadCm6Editor } from "./EditorGate";

let dispose: (() => void) | undefined;
afterEach(() => {
  dispose?.(); endEdit("page-navigation"); resetStore();
  setCm6EditorEnabled(false); document.body.replaceChildren(); vi.restoreAllMocks();
});

it("reports a failed import once and preserves native caret edits, paste and subsequent classic activations", async () => {
  vi.spyOn(backend(), "setAppBool").mockResolvedValue();
  vi.spyOn(document, "hasFocus").mockReturnValue(true);
  loadSingle({ name: "Failed engine", title: "Failed engine", kind: "page", pre_block: null,
    blocks: [{ id: "failed", raw: "hello", collapsed: false, children: [] }] });
  setCm6EditorEnabled(true);
  const host = document.createElement("div"); document.body.append(host);
  dispose = render(() => <Block id="failed" />, host);
  startEditing("failed", 2);
  await vi.waitFor(() => expect(chunk.reject).toBeDefined());
  const editor = editorIn(host);
  expect(editor.kind).toBe("textarea");
  expect(editor.selection()).toEqual([2, 2]);
  expect(editor.keydown({ key: "ArrowLeft" }).defaultPrevented).toBe(false);
  // jsdom lacks the browser's native caret movement.
  editor.selectByUser(1);
  editor.type("!");
  const paste = editor.paste({ text: "?" });
  expect(paste.defaultPrevented).toBe(false);
  // jsdom does not perform the browser's default plain-text paste.
  editor.input("h!?ello", 3, { inputType: "insertFromPaste", data: "?" });
  expect(editor.value()).toBe("h!?ello");
  const error = new Error("asset unavailable");
  chunk.reject!(error);
  await preloadCm6Editor();
  // Vitest wraps a rejected module factory, retaining the asset error as cause.
  expect(chunk.report).toHaveBeenCalledExactlyOnceWith("editor-load", expect.objectContaining({ cause: error }));
  expect(editorIn(host).element).toBe(editor.element);
  expect(editor.value()).toBe("h!?ello");
  expect(editor.selection()).toEqual([3, 3]);
  editor.type(".");
  expect(editor.value()).toBe("h!?.ello");
  expect(node("failed").raw).toBe("h!?.ello");
  endEdit("page-navigation");
  startEditing("failed", 4);
  const next = editorIn(host);
  expect(next.kind).toBe("textarea");
  expect(next.value()).toBe("h!?.ello");
  expect(next.selection()).toEqual([4, 4]);
  next.type("again");
  await preloadCm6Editor();
  expect(node("failed").raw).toBe("h!?.againello");
  expect(chunk.report).toHaveBeenCalledTimes(1);
});
