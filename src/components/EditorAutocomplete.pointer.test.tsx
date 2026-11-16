import { afterEach, expect, it, vi } from "vitest";
import { render } from "solid-js/web";
import { EditorAutocomplete } from "./EditorAutocomplete";
import { installPaneTracker } from "../ui";
import { focusedPaneId, resetPaneLayoutToSingle, setFocusedPaneId, splitPane } from "../panes";

afterEach(() => {
  resetPaneLayoutToSingle({ tabs: [{ history: [{ kind: "journals" }], pos: 0, pinned: false }], activeIndex: 0 });
  document.body.innerHTML = "";
});

it.each(["mouse", "touch", "pen"].flatMap(pointerType =>
  ["slash command", "page reference"].map(label => ({ pointerType, label }))))(
  "$pointerType chooses a $label once and keeps its editor's pane", ({ pointerType, label }) => {
    const pane = splitPane("main", "row")!;
    setFocusedPaneId(pane);
    const stopTracking = installPaneTracker();
    const select = vi.fn();
    const item = { label };
    const root = document.createElement("div"); document.body.append(root);
    const dispose = render(() => <EditorAutocomplete items={[item]} index={0} style={{}}
      listRef={() => {}} select={select} blockState={null} retry={() => {}} />, root);
    try {
      const row = document.querySelector(".ac-item")!;
      row.dispatchEvent(new PointerEvent("pointerdown", {
        bubbles: true, cancelable: true, pointerType, isPrimary: true, pointerId: 1, button: 0,
      }));
      row.dispatchEvent(new MouseEvent("mousedown", { bubbles: true, cancelable: true, button: 0 }));
      row.dispatchEvent(new PointerEvent("pointerup", { bubbles: true, pointerType, pointerId: 1 }));
      row.dispatchEvent(new MouseEvent("mouseup", { bubbles: true, button: 0 }));
      row.dispatchEvent(new MouseEvent("click", { bubbles: true, button: 0 }));
      expect(select).toHaveBeenCalledExactlyOnceWith(item);
      expect(focusedPaneId()).toBe(pane);
    } finally { stopTracking(); dispose(); }
  });
