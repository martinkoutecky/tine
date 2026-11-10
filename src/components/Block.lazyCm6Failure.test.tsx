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
import { resetStore } from "../document";
import { loadSingle } from "../document/workingSet";
import { endEdit, startEditing } from "../editorController";
import { setCm6EditorEnabled } from "../editor/cm6Setting";
import { editorIn, maybeEditorIn } from "../tests/editorDriver";

let dispose: (() => void) | undefined;
afterEach(() => {
  dispose?.(); endEdit("page-navigation"); resetStore();
  setCm6EditorEnabled(false); document.body.replaceChildren(); vi.restoreAllMocks();
});

it("reports a failed engine import and keeps the block editable with its buffered input", async () => {
  vi.spyOn(backend(), "setAppBool").mockResolvedValue();
  vi.spyOn(document, "hasFocus").mockReturnValue(true);
  loadSingle({ name: "Failed engine", title: "Failed engine", kind: "page", pre_block: null,
    blocks: [{ id: "failed", raw: "hello", collapsed: false, children: [] }] });
  setCm6EditorEnabled(true);
  const host = document.createElement("div"); document.body.append(host);
  dispose = render(() => <Block id="failed" />, host);
  startEditing("failed", 2);
  await vi.waitFor(() => expect(chunk.reject).toBeDefined());
  // This is the native input buffer, not a mounted textarea editor.
  const pending = document.activeElement as HTMLTextAreaElement;
  pending.value = "!";
  pending.dispatchEvent(new InputEvent("input", { bubbles: true, inputType: "insertText", data: "!" }));
  const data = { types: ["text/plain"], files: [], items: [], getData: vi.fn(() => "?") };
  const paste = new Event("paste", { bubbles: true, cancelable: true });
  Object.defineProperty(paste, "clipboardData", { value: data });
  pending.dispatchEvent(paste);
  expect(data.getData).toHaveBeenCalledExactlyOnceWith("text/plain");
  // Native clipboard event data is readable during the event, not after it.
  data.getData.mockReturnValue("");
  const error = new Error("asset unavailable");
  chunk.reject!(error);
  await vi.waitFor(() => expect(maybeEditorIn(host)?.value()).toBe("he!?llo"));
  // Vitest wraps a rejected module factory, retaining the asset error as cause.
  expect(chunk.report).toHaveBeenCalledExactlyOnceWith("editor-load", expect.objectContaining({ cause: error }));
  const editor = editorIn(host);
  expect(editor.kind).toBe("textarea");
  expect(editor.value()).toBe("he!?llo");
  expect(editor.selection()).toEqual([4, 4]);
  editor.type(".");
  expect(editor.value()).toBe("he!?.llo");
});
