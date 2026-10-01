import { afterEach, expect, it, vi } from "vitest";
import { render } from "solid-js/web";
import { createSignal } from "solid-js";
import { AstBody } from "../render/body";
import { DeferredStandaloneMacro } from "./DeferredStandaloneMacro";
import { renderedBlocks, resetNearObserverForTests } from "../lazyObserve";
import { boardGroupField, formulaReferenceName } from "../sheet/boardColumns";
import { stepMonth, optionsUpdater } from "./primitives";

let callback: IntersectionObserverCallback;
const observe = vi.fn();
const unobserve = vi.fn();
function fakeObserver() {
  vi.stubGlobal("IntersectionObserver", class {
    constructor(cb: IntersectionObserverCallback) { callback = cb; }
    observe = observe; unobserve = unobserve; disconnect = vi.fn();
  });
}
afterEach(() => { resetNearObserverForTests(); vi.unstubAllGlobals(); vi.clearAllMocks(); });
it("both render adapters defer, latch and remount eagerly; pending placeholders unobserve on disposal", () => {
  fakeObserver();
  const root = document.createElement("div");
  const dispose = render(() => <><AstBody raw="body" blockId="body" /><DeferredStandaloneMacro blockId="macro" raw="query">mounted query</DeferredStandaloneMacro></>, root);
  const placeholders = [...root.querySelectorAll(".ast-deferred")];
  expect(placeholders).toHaveLength(2);
  expect(observe).toHaveBeenCalledTimes(2);
  callback(placeholders.map((target) => ({ target, isIntersecting: true }) as IntersectionObserverEntry), {} as IntersectionObserver);
  expect(root.querySelectorAll(".ast-deferred")).toHaveLength(0);
  expect(root.textContent).toContain("mounted query");
  expect([...renderedBlocks]).toEqual(["body", "macro"]);
  dispose();
  const again = render(() => <AstBody raw="body" blockId="body" />, root);
  expect(root.querySelector(".ast-deferred")).toBeNull();
  expect(observe).toHaveBeenCalledTimes(2);
  again();
  const pending = render(() => <AstBody raw="other" blockId="other" />, root);
  const target = root.querySelector(".ast-deferred");
  pending();
  expect(unobserve).toHaveBeenCalledWith(target);
});
it("body ids stay captured while standalone macros latch current props when a pending component is reused", () => {
  fakeObserver();
  const [id, setId] = createSignal("initial");
  const root = document.createElement("div");
  const dispose = render(() => <><AstBody raw="body" blockId={`body-${id()}`} /><DeferredStandaloneMacro raw="query" blockId={`macro-${id()}`}>query</DeferredStandaloneMacro></>, root);
  const targets = [...root.querySelectorAll(".ast-deferred")];
  setId("current");
  callback(targets.map((target) => ({ target, isIntersecting: true }) as IntersectionObserverEntry), {} as IntersectionObserver);
  expect([...renderedBlocks]).toEqual(["body-initial", "macro-current"]);
  dispose();
});
it("a body without an id stays eager without observer registration", () => {
  fakeObserver();
  const root = document.createElement("div");
  const dispose = render(() => <AstBody raw="eager" />, root);
  expect(root.querySelector(".ast-deferred")).toBeNull();
  expect(observe).not.toHaveBeenCalled();
  dispose();
});
it("field codecs preserve builtins, custom refs, legacy formulas and invalid-token fallback", () => {
  expect([undefined, "", "invalid", "formula.x", "prop:cost"].map(boardGroupField)).toEqual(["state", "state", "state", "formula:x", "prop:cost"]);
  expect([formulaReferenceName("state"), formulaReferenceName("prop:cost"), formulaReferenceName("formula:x")]).toEqual(["state", "cost", null]);
});
it("month stepping retains Euclidean rollover and options update publishes exactly the persisted object", () => {
  expect(stepMonth({ y: 2026, m: 0 }, -1)).toEqual({ y: 2025, m: 11 });
  expect(stepMonth({ y: 0, m: 0 }, -1)).toEqual({ y: -1, m: 11 });
  let options = { font: 16, margin: 10 };
  const save = vi.fn();
  const update = optionsUpdater(() => options, (next) => { options = next; }, save);
  update({ font: 19 });
  expect(options).toEqual({ font: 19, margin: 10 });
  expect(save).toHaveBeenCalledExactlyOnceWith(options);
});
