import { afterEach, expect, it, vi } from "vitest";
import { render } from "solid-js/web";
import { For } from "solid-js";

const chunk = vi.hoisted(() => ({ requested: vi.fn(), release: undefined as undefined | (() => void) }));
vi.mock("./Cm6BlockEditor", async original => {
  chunk.requested();
  await new Promise<void>(resolve => { chunk.release = resolve; });
  return original();
});
vi.mock("../appIdentity", () => ({ APP_UPDATE_CHANNEL: "beta", APP_PRODUCT_NAME: "Tine" }));
// This case owns both engines even in the forced-CM6 suite, starting classic
// without preloading the deliberately held-back chunk in the driver's hook.
vi.mock("../editor/cm6Setting", async original => {
  const setting = await original<typeof import("../editor/cm6Setting")>();
  setting.setCm6EditorEnabled(false);
  return setting;
});

import { Block, SurfaceContext } from "./Block";
import { backend } from "../backend";
import { node, pageByName, resetStore } from "../document";
import { loadSingle } from "../document/workingSet";
import { endEdit, noteSurfaceFocused, startEditing } from "../editorController";
import { setCm6EditorEnabled } from "../editor/cm6Setting";
import { editorIn } from "../tests/editorDriver";
import { preloadCm6Editor } from "./EditorGate";

let dispose: (() => void) | undefined;
afterEach(() => {
  dispose?.(); endEdit("page-navigation"); resetStore();
  setCm6EditorEnabled(false); document.body.replaceChildren(); vi.restoreAllMocks(); vi.unstubAllGlobals();
});

it("keeps cold typing, input-only Backspace and native caret movement across block switches; the next activation uses CM6", async () => {
  // Keep the large page's offscreen bodies deferred, as on a real first open.
  vi.stubGlobal("IntersectionObserver", class {
    observe() {}
    unobserve() {}
    disconnect() {}
  });
  vi.spyOn(backend(), "setAppBool").mockResolvedValue();
  vi.spyOn(document, "hasFocus").mockReturnValue(true);
  if (!Range.prototype.getClientRects) Range.prototype.getClientRects = () => [] as unknown as DOMRectList;
  if (!Range.prototype.getBoundingClientRect) Range.prototype.getBoundingClientRect = () => new DOMRect();
  loadSingle({ name: "Cold editor", title: "Cold editor", kind: "page", pre_block: null,
    blocks: [...["typed", "deleted", "moved"], ...Array.from({ length: 1497 }, (_, i) => `filler-${i}`)]
      .map(id => ({ id, raw: "hello", collapsed: false, children: [] })) });
  const host = document.createElement("div"); document.body.append(host);
  dispose = render(() => <>
    <section data-main><SurfaceContext.Provider value="main"><For each={pageByName("Cold editor")?.roots}>{id => <Block id={id} />}</For></SurfaceContext.Provider></section>
    <section data-sidebar><SurfaceContext.Provider value="sidebar"><For each={pageByName("Cold editor")?.roots}>{id => <Block id={id} />}</For></SurfaceContext.Provider></section>
  </>, host);
  noteSurfaceFocused("main");
  startEditing("typed", 5);
  expect(editorIn(host).kind).toBe("textarea");
  expect(chunk.requested).not.toHaveBeenCalled();
  endEdit("page-navigation");
  setCm6EditorEnabled(true);
  noteSurfaceFocused("sidebar");
  const sidebar = host.querySelector<HTMLElement>("[data-sidebar]")!;
  const coldSamples: number[] = [];
  for (let i = 0; i < 9; i++) {
    const at = performance.now();
    startEditing("typed", 5);
    coldSamples.push(performance.now() - at);
    // The driver's DOM-wide editor lookup is an assertion, not activation work.
    expect(editorIn(sidebar).kind).toBe("textarea");
    if (i < 8) endEdit("page-navigation");
  }
  // The asset is deliberately still pending. Budget the usable first-edit
  // path, including real Block mounting, rather than a mocked import's delay.
  console.log(`cold first edit ms=${coldSamples[0]}; pending-import activations ms=${JSON.stringify(coldSamples)}`);
  expect([...coldSamples].sort((a, b) => a - b)[4]).toBeLessThan(50);
  await vi.waitFor(() => expect(chunk.requested).toHaveBeenCalledTimes(1));
  const typed = editorIn(sidebar);
  expect(typed.kind).toBe("textarea");
  expect(typed.selection()).toEqual([5, 5]);
  expect(document.activeElement).toBe(typed.element);
  typed.type("!");
  startEditing("deleted", 5);
  expect(node("typed").raw).toBe("hello!");
  const deleted = editorIn(sidebar);
  expect(deleted.kind).toBe("textarea");
  deleted.input("hell", 4, { inputType: "deleteContentBackward", data: null });
  startEditing("moved", 2);
  expect(node("deleted").raw).toBe("hell");
  const moved = editorIn(sidebar);
  expect(moved.kind).toBe("textarea");
  expect(moved.keydown({ key: "ArrowLeft" }).defaultPrevented).toBe(false);
  // jsdom has no native caret movement: report the browser's resulting selection.
  moved.selectByUser(1);
  moved.type("!");
  expect(moved.value()).toBe("h!ello");
  expect(moved.selection()).toEqual([2, 2]);
  chunk.release!();
  await preloadCm6Editor();
  expect(editorIn(sidebar).element).toBe(moved.element);
  moved.type("?");
  expect(node("moved").raw).toBe("h!?ello");
  startEditing("typed", 3);
  const next = editorIn(sidebar);
  expect(next.kind).toBe("cm6");
  expect(next.value()).toBe("hello!");
  expect(next.selection()).toEqual([3, 3]);
  expect(document.activeElement).toBe(next.element);
  expect(node("deleted").raw).toBe("hell");
  expect(node("moved").raw).toBe("h!?ello");
  expect(chunk.requested).toHaveBeenCalledTimes(1);
});
