// @vitest-environment jsdom
import { afterEach, expect, it, vi } from "vitest";
import { EditorSelection, EditorState, StateEffect } from "@codemirror/state";
import { EditorView } from "@codemirror/view";
import { cm6CaretReveal } from "./cm6CaretReveal";

// CM calls scroll handlers while it writes a measurement, when reading layout throws
// ("Reading the editor layout isn't allowed during an update"). The reveal then
// failed on every caret scroll: CM logged the error and fell back to its own
// scrolling, bypassing the pane / sticky-notice / pin policy. jsdom has no layout:
// the caret rect and the pane geometry are given.
afterEach(() => { vi.restoreAllMocks(); document.body.innerHTML = ""; });

it("reveals a CM-requested caret scroll through the pane policy without reading layout during CM's update", () => {
  const frames: FrameRequestCallback[] = [];
  vi.spyOn(window, "requestAnimationFrame").mockImplementation(cb => { frames.push(cb); return frames.length; });
  vi.spyOn(window, "cancelAnimationFrame").mockImplementation(() => {});
  const flush = () => { for (let i = 0; i < 8; i++) frames.splice(0).forEach(cb => cb(0)); };
  const errors = vi.spyOn(console, "error").mockImplementation(() => {});
  // jsdom Ranges have no geometry; CM's real caret read needs these to exist.
  Object.assign(Range.prototype, { getClientRects: () => [], getBoundingClientRect: () => new DOMRect() });

  const pane = document.createElement("main");
  pane.className = "main-content";
  pane.style.overflowY = "auto";
  document.body.append(pane);
  Object.defineProperty(pane, "clientHeight", { value: 400 });
  Object.defineProperty(pane, "scrollHeight", { value: 4000 });
  pane.getBoundingClientRect = () => ({ top: 0, bottom: 400, left: 0, right: 800, height: 400, width: 800 } as DOMRect);
  const view = new EditorView({ parent: pane, state: EditorState.create({ doc: "one\ntwo\nthree" }) });
  const coordsAtPos = view.coordsAtPos.bind(view);
  // The real read (it throws during CM's update), then a caret below the pane.
  vi.spyOn(view, "coordsAtPos").mockImplementation(pos => { coordsAtPos(pos); return { top: 900, bottom: 920, left: 10, right: 11 }; });
  // Focused after mount: the settling watcher owns only a focused mount's first reveal.
  const reveal = cm6CaretReveal(view);
  view.contentDOM.focus();
  vi.spyOn(view, "hasFocus", "get").mockReturnValue(true);
  view.dispatch({ effects: StateEffect.appendConfig.of(reveal.extension) });
  flush();
  pane.scrollTop = 0;

  // CM runs scroll handlers in its measure cycle's write phase (jsdom has no
  // editor height, so drive that phase directly with the public measure API).
  const [handler] = view.state.facet(EditorView.scrollHandler);
  let handled: boolean | undefined;
  view.requestMeasure({ read: () => null, write: () => { handled = handler(view, EditorSelection.cursor(9), {} as never); } });
  flush();

  expect(handled).toBe(true);
  expect(errors).not.toHaveBeenCalled();
  expect(pane.scrollTop).toBeGreaterThan(0);
  reveal.cancel();
  view.destroy();
});
