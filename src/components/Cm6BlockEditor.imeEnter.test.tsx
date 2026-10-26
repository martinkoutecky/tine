import { afterEach, expect, it, vi } from "vitest";
import { render } from "solid-js/web";
import { EditorView } from "@codemirror/view";

// Exercise CM6's actual desktop Safari/WebKit post-composition key suppression.
vi.hoisted(() => Object.defineProperty(navigator, "vendor", { configurable: true, value: "Apple Computer, Inc." }));

import { Cm6BlockEditor } from "./Cm6BlockEditor";
import { endEdit, startEditing } from "../editorController";
import { pageByName, resetStore } from "../document";
import { loadSingle } from "../document/workingSet";
import { doc } from "../document/model";

afterEach(() => {
  endEdit("page-navigation");
  resetStore();
  document.body.innerHTML = "";
});

function mount() {
  loadSingle({ name: "IME Enter", title: "IME Enter", kind: "page", pre_block: null,
    blocks: [{ id: "ime-enter", raw: "enter 한 tail", collapsed: false, children: [] }] });
  startEditing("ime-enter", 7);
  const root = document.createElement("div");
  document.body.append(root);
  const dispose = render(() => <Cm6BlockEditor id="ime-enter" surface="main" scope={null} />, root);
  const view = EditorView.findFromDOM(root.querySelector('[data-block-editor="cm6"]') as HTMLElement)!;
  return { view, dispose };
}

function enter(view: EditorView, init: KeyboardEventInit = {}) {
  const event = new KeyboardEvent("keydown", { key: "Enter", code: "Enter", keyCode: 13,
    bubbles: true, cancelable: true, ...init });
  view.contentDOM.dispatchEvent(event);
  return event;
}

it("splits once when an ordinary Enter follows compositionend immediately, without source newlines", () => {
  const { view, dispose } = mount();
  try {
    view.contentDOM.dispatchEvent(new CompositionEvent("compositionstart", { bubbles: true }));
    view.contentDOM.dispatchEvent(new CompositionEvent("compositionend", { bubbles: true, data: "한" }));
    expect(enter(view).defaultPrevented).toBe(true);
    const roots = pageByName("IME Enter")!.roots;
    expect(roots).toHaveLength(2);
    expect(roots.map(id => doc.byId[id].raw)).toEqual(["enter 한", " tail"]);
  } finally { dispose(); }
});

it.each([
  { isComposing: true },
  { keyCode: 229 },
])("leaves a confirming Enter consumed by the IME intact: %j", init => {
  const { view, dispose } = mount();
  try {
    view.contentDOM.dispatchEvent(new CompositionEvent("compositionstart", { bubbles: true }));
    expect(enter(view, init).defaultPrevented).toBe(false);
    view.contentDOM.dispatchEvent(new CompositionEvent("compositionend", { bubbles: true, data: "한" }));
    expect(pageByName("IME Enter")!.roots).toEqual(["ime-enter"]);
    expect(doc.byId["ime-enter"].raw).toBe("enter 한 tail");
    expect(view.state.doc.toString()).toBe("enter 한 tail");
  } finally { dispose(); }
});

it("preserves legacy 229 protection even when compositionend arrives before keydown", () => {
  const { view, dispose } = mount();
  try {
    view.contentDOM.dispatchEvent(new CompositionEvent("compositionstart", { bubbles: true }));
    view.contentDOM.dispatchEvent(new CompositionEvent("compositionend", { bubbles: true, data: "한" }));
    expect(enter(view, { keyCode: 229 }).defaultPrevented).toBe(false);
    expect(pageByName("IME Enter")!.roots).toEqual(["ime-enter"]);
    expect(view.state.doc.toString()).toBe("enter 한 tail");
  } finally { dispose(); }
});
