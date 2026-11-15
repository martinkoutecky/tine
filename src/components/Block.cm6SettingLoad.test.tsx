import { afterEach, beforeAll, beforeEach, expect, it, vi } from "vitest";
import { preloadCm6Editor } from "./EditorGate";
beforeAll(preloadCm6Editor);
import { For } from "solid-js";
import { render } from "solid-js/web";

// Own the real preference even in the opt-in CM6 census.
vi.mock("../appIdentity", () => ({ APP_UPDATE_CHANNEL: "beta", APP_PRODUCT_NAME: "Tine" }));
vi.unmock("../editor/cm6Setting");

import { Block, SurfaceContext } from "./Block";
import { backend } from "../backend";
import { pageByName, resetStore } from "../document";
import { loadSingle } from "../document/workingSet";
import { endEdit, startEditing } from "../editorController";
import { maybeEditorIn } from "../tests/editorDriver";
import { cm6EditorEnabled, initCm6EditorSetting, setCm6EditorEnabled } from "../editor/cm6Setting";

let dispose: (() => void) | undefined;
const flush = () => new Promise<void>(resolve => setTimeout(resolve, 0));
const kinds = (host: HTMLElement) => {
  const kind = maybeEditorIn(host)?.kind;
  return { cm6: kind === "cm6", textarea: kind === "textarea" };
};

beforeEach(() => {
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
  resetStore();
  setCm6EditorEnabled(false);
  await flush();
  document.body.replaceChildren();
  vi.restoreAllMocks();
});

it.each([false, true])("the first editor waits for the stored CM6 choice %s", async stored => {
  setCm6EditorEnabled(true);
  await flush();
  // A big graph keeps the backend busy at launch: the reply to the startup read lands after the first click.
  let reply!: (value: boolean) => void;
  vi.spyOn(backend(), "getAppBool").mockReturnValue(new Promise<boolean>(resolve => { reply = resolve; }));
  const loading = initCm6EditorSetting();
  expect(cm6EditorEnabled()).toBe(true);

  loadSingle({ name: "Slow load", title: "Slow load", kind: "page", format: "md", pre_block: null,
    blocks: [{ id: "only", raw: "hello", collapsed: false, children: [] }] });
  const host = document.createElement("div"); document.body.append(host);
  dispose = render(() => <SurfaceContext.Provider value="main">
    <For each={pageByName("Slow load")?.roots}>{id => <Block id={id} />}</For>
  </SurfaceContext.Provider>, host);
  await flush();
  startEditing("only", 0, null);
  await flush();
  // Not the default engine: the editor waits for the reply instead of mounting on the default.
  expect(kinds(host)).toEqual({ cm6: false, textarea: false });

  reply(stored);
  await loading;
  await flush();
  expect(cm6EditorEnabled()).toBe(stored);
  expect(kinds(host)).toEqual({ cm6: stored, textarea: !stored });
  expect(maybeEditorIn(host)!.value()).toBe("hello");
  expect(maybeEditorIn(host)!.selection()).toEqual([0, 0]);
});

it("opens at once with the current engine when nothing is being read", async () => {
  vi.spyOn(backend(), "getAppBool").mockResolvedValue(false);
  await initCm6EditorSetting();
  loadSingle({ name: "Quiet", title: "Quiet", kind: "page", format: "md", pre_block: null,
    blocks: [{ id: "q", raw: "hello", collapsed: false, children: [] }] });
  const host = document.createElement("div"); document.body.append(host);
  dispose = render(() => <SurfaceContext.Provider value="main">
    <For each={pageByName("Quiet")?.roots}>{id => <Block id={id} />}</For>
  </SurfaceContext.Provider>, host);
  startEditing("q", 0, null);
  await flush();
  expect(kinds(host)).toEqual({ cm6: false, textarea: true });
});
