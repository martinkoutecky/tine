import { afterEach, expect, it, vi } from "vitest";
import { backend } from "../backend";
import { mainPaneRouter, resetTabsToJournals } from "../router";
import { PageView } from "./Page";
import { deleteBlock, mergeWithNext, mergeWithPrev, pageByName } from "../document";
import { loadSingle } from "../document/workingSet";
import { doc } from "../document/model";
import { endEdit, startEditing } from "../editorController";
import { installKeybindings } from "../keybindings";
import { editorIn } from "../tests/editorDriver";
import { closeContextMenu } from "../ui";
import { ContextMenu } from "./ContextMenu";
import { Block, Editor } from "./Block";
import { blk, installBlockEditorLifecycle, mount, page } from "../tests/blockEditorTestkit";
import { BlockList } from "./BlockList";
import { FailureBoundary } from "./FailureBoundary";

installBlockEditorLifecycle();
afterEach(() => { endEdit("page-navigation"); vi.restoreAllMocks(); resetTabsToJournals(); });

it("a retiring editor cannot take down its page while its block is removed", () => {
  loadSingle(page("Deletion", [blk("before", "Before"), blk("rule", "---"), blk("after", "After")]));
  startEditing("rule", 0);
  // Retain the editor owner to exercise memo reads before its parent retires it;
  // this reconstructs the lifetime interval identified by the reporter's stack.
  const view = mount(() => <FailureBoundary region="This page"><Block id="before" /><Editor id="rule" /><Block id="after" /></FailureBoundary>);
  try {
    editorIn(view.root).keydown({ key: "Backspace", bubbles: true, cancelable: true });
    expect(doc.byId.rule).toBeUndefined();
    expect(view.root.querySelector(".region-failure")).toBeNull();
    expect(view.root.textContent).toContain("After");
  } finally { view.dispose(); }
});

it.each(["delete", "merge-backward", "merge-forward"])("a retiring rendered row preserves its page during %s", action => {
  loadSingle(page("Deletion", [blk("before", "Before"), blk("retiring", "Ordinary text"), blk("after", "After")]));
  // A window's row computations can run before its For retires the removed row.
  // Keep that owner alive to exercise this interval independently of scheduling.
  const view = mount(() => <FailureBoundary region="This page"><Block id="before" /><Block id="retiring" /><Block id="after" /></FailureBoundary>);
  try {
    if (action === "delete") deleteBlock("retiring");
    else if (action === "merge-backward") expect(mergeWithPrev("retiring")).toBe(true);
    else expect(mergeWithNext("before")).toBe(true);
    expect(doc.byId.retiring).toBeUndefined();
    expect(view.root.querySelector(".region-failure")).toBeNull();
    expect(view.root.querySelector('[data-block-id="after"]')?.textContent).toContain("After");
    startEditing("before", 0);
    expect(editorIn(view.root).blockId).toBe("before");
  } finally { view.dispose(); }
});

it.each(["backspace-start", "context-delete"])("a windowed outline stays rendered after %s", action => {
  const blocks = Array.from({ length: 120 }, (_, i) => blk(`row-${i}`, `Row ${i}`));
  blocks[1] = blk("rule", "---");
  loadSingle(page("Deletion", blocks));
  const view = mount(() => <FailureBoundary region="This page"><BlockList ids={pageByName("Deletion")!.roots} /></FailureBoundary>);
  try {
    startEditing("rule", 0);
    if (action === "context-delete") deleteBlock("rule");
    else editorIn(view.root).keydown({ key: "Backspace", bubbles: true, cancelable: true });
    expect(doc.byId.rule).toBeUndefined();
    expect(view.root.querySelector(".region-failure")).toBeNull();
    expect(view.root.textContent).toContain("Row 2");
  } finally { view.dispose(); }
});

it("Backspace on an only child keeps the page rendered", () => {
  loadSingle(page("Deletion", [blk("before", "Before"), { ...blk("parent", "Parent"), children: [blk("rule", "---")] }, blk("after", "After")]));
  const view = mount(() => <FailureBoundary region="This page"><BlockList ids={pageByName("Deletion")!.roots} /></FailureBoundary>);
  try {
    startEditing("rule", 0);
    editorIn(view.root).keydown({ key: "Backspace", bubbles: true, cancelable: true });
    expect(doc.byId.rule).toBeUndefined();
    expect(view.root.querySelector(".region-failure")).toBeNull();
    expect(view.root.textContent).toContain("After");
  } finally { view.dispose(); }
});

for (const format of ["md", "org"] as const) {
  const forms = {
    rule: "---", empty: "",
    code: format === "md" ? "```js\nx = 1\n```" : "#+BEGIN_SRC js\nx = 1\n#+END_SRC",
    quote: format === "md" ? "> Quote" : "#+BEGIN_QUOTE\nQuote\n#+END_QUOTE",
    table: "| Name |\n|------|\n| Row |",
    embed: "{{embed [[Source]]}}", query: "{{query (page [[Source]])}}",
    heading: format === "md" ? "# " : "* ",
    properties: format === "md" ? "note:: value" : ":PROPERTIES:\n:note: value\n:END:",
  };
  it.each(Object.entries(forms))(`${format}: deleting a retiring %s editor preserves siblings and refuses stale input`, (_form, raw) => {
    loadSingle({ ...page("Deletion", [blk("before", "Before"), blk("rule", raw), blk("after", "After")]), format });
    startEditing("rule", 0);
    const view = mount(() => <FailureBoundary region="This page"><Block id="before" /><Editor id="rule" /><Block id="after" /></FailureBoundary>);
    try {
      const old = editorIn(view.root);
      deleteBlock("rule");
      expect(view.root.querySelector(".region-failure")).toBeNull();
      expect(view.root.textContent).toContain("After");
      old.input("stale input");
      old.reportBlur();
      expect(doc.byId.rule).toBeUndefined();
      expect(doc.byId.before.raw).toBe("Before");
      expect(doc.byId.after.raw).toBe("After");
      expect(pageByName("Deletion")!.roots).toEqual(["before", "after"]);
    } finally { view.dispose(); }
  });
}

it.each(["context-delete", "selected-delete", "clear-backspace", "backspace-start", "delete-forward"])("routed PageView remains visible after %s", async action => {
  const dto = { ...page("Deletion", [blk("before", "Before"), blk("rule", "---"), blk("after", "After")]), id: "pages/Deletion.md" };
  vi.spyOn(backend(), "getPage").mockResolvedValue(dto);
  mainPaneRouter.replaceActiveRoute({ kind: "page", name: dto.name, pageKind: "page" });
  const keys = installKeybindings();
  const view = mount(() => <FailureBoundary region="This page"><PageView /><ContextMenu /></FailureBoundary>);
  try {
    await vi.waitFor(() => expect(view.root.querySelector("hr")).not.toBeNull());
    if (action === "context-delete") {
      view.root.querySelector<HTMLElement>('[data-block-id="rule"] .block-main')!
        .dispatchEvent(new MouseEvent("contextmenu", { bubbles: true, cancelable: true }));
      [...document.querySelectorAll<HTMLElement>(".ctx-item")].find(el => el.textContent?.trim() === "Delete block")!.click();
    } else {
      startEditing(action === "delete-forward" ? "before" : "rule", action === "delete-forward" ? 6 : 0);
      const editor = editorIn(view.root);
      await new Promise<void>(resolve => requestAnimationFrame(() => resolve()));
      if (action === "clear-backspace") editor.input("", 0);
      if (action === "selected-delete") {
        editor.keydown({ key: "Escape", bubbles: true, cancelable: true });
        document.body.dispatchEvent(new KeyboardEvent("keydown", { key: "Delete", bubbles: true, cancelable: true }));
      } else editor.keydown({ key: action === "delete-forward" ? "Delete" : "Backspace", bubbles: true, cancelable: true });
    }
    expect(view.root.querySelector(".region-failure")).toBeNull();
    expect(view.root.textContent).toContain("After");
    expect(doc.byId.rule).toBeUndefined();
  } finally { view.dispose(); keys(); closeContextMenu(); }
});

for (const format of ["md", "org"] as const) for (const position of [0, 1, 2]) for (const children of [false, true]) {
  const actions = ["clear-backspace", "backspace-start", "delete-forward", "selected-backspace", "selected-delete", "context-delete", "cut"]
    .filter(action => action !== "delete-forward" || position > 0);
  it.each(actions)(
    `${format} position=${position} children=${children} keeps the page rendered after %s`, async action => {
      const target = { ...blk("rule", "---"), children: children ? [blk("child", "Child")] : [] };
      const blocks = [blk("before", "Before"), blk("after", "After")];
      blocks.splice(position, 0, target);
      loadSingle({ ...page("Deletion", blocks), format });
      const keys = installKeybindings();
      const view = mount(() => <FailureBoundary region="This page"><BlockList ids={pageByName("Deletion")!.roots} /><ContextMenu /></FailureBoundary>);
      try {
        if (action === "context-delete") {
          view.root.querySelector<HTMLElement>('[data-block-id="rule"] .block-main')!
            .dispatchEvent(new MouseEvent("contextmenu", { bubbles: true, cancelable: true }));
          [...document.querySelectorAll<HTMLElement>(".ctx-item")].find(el => el.textContent?.trim() === "Delete block")!.click();
        } else {
          const forward = action === "delete-forward";
          const previous = blocks[position - 1];
          startEditing(forward ? previous!.id : "rule", forward ? previous!.raw.length : 0);
          const editor = editorIn(view.root);
          await new Promise<void>(resolve => requestAnimationFrame(() => resolve()));
          if (action === "clear-backspace") editor.input("", 0);
          if (action.startsWith("selected") || action === "cut") {
            editor.keydown({ key: "Escape", bubbles: true, cancelable: true });
            document.body.dispatchEvent(new KeyboardEvent("keydown", { key: action === "cut" ? "x" : action === "selected-delete" ? "Delete" : "Backspace", ctrlKey: action === "cut", bubbles: true, cancelable: true }));
            if (action === "cut") await vi.waitFor(() => expect(doc.byId.rule).toBeUndefined());
          } else editor.keydown({ key: forward ? "Delete" : "Backspace", bubbles: true, cancelable: true });
        }
        expect(view.root.querySelector(".region-failure")).toBeNull();
        expect(view.root.querySelector('[data-block-id="after"]')).not.toBeNull();
        expect(view.root.querySelector('[data-block-id="before"]')).not.toBeNull();
        const retained = position === 0 && (action === "backspace-start" || action === "clear-backspace" && children);
        if (retained) expect(doc.byId.rule.raw).toBe(action === "clear-backspace" ? "" : "---");
        else expect(doc.byId.rule).toBeUndefined();
        if (children) {
          const removesSubtree = action === "context-delete" || action === "cut" || action.startsWith("selected");
          if (removesSubtree) expect(doc.byId.child).toBeUndefined();
          else expect(doc.byId.child.raw).toBe("Child");
        }
      } finally { view.dispose(); keys(); closeContextMenu(); }
    });
}

it.each(["delete", "merge-backward", "merge-forward"])("keeps the page rendered after %s of a horizontal-rule block", action => {
  loadSingle(page("Deletion", [blk("before", "Before"), blk("rule", "---"), blk("after", "After")]));
  const view = mount(() => <FailureBoundary region="This page"><BlockList ids={pageByName("Deletion")!.roots} /></FailureBoundary>);
  try {
    expect(view.root.querySelector("hr")).not.toBeNull();
    if (action === "delete") deleteBlock("rule");
    else if (action === "merge-backward") expect(mergeWithPrev("rule")).toBe(true);
    else expect(mergeWithNext("before")).toBe(true);
    expect(doc.byId.rule).toBeUndefined();
    expect(view.root.querySelector(".region-failure")).toBeNull();
    expect(view.root.textContent).toContain("After");
    expect(view.root.querySelector('[data-block-id="before"]')).not.toBeNull();
  } finally { view.dispose(); }
});

it.each(["existing", "typed", "cleared"])("backspace merges a %s rule after native input and selection settle", async state => {
  loadSingle(page("Deletion", [blk("before", "Before"), blk("rule", state === "typed" ? "" : "---"), blk("after", "After")]));
  const view = mount(() => <FailureBoundary region="This page"><BlockList ids={pageByName("Deletion")!.roots} /></FailureBoundary>);
  try {
    startEditing("rule", 3);
    const editor = editorIn(view.root);
    await new Promise<void>(resolve => requestAnimationFrame(() => resolve()));
    if (state === "typed") for (const raw of ["-", "--", "---"]) editor.input(raw);
    if (state === "cleared") for (const raw of ["--", "-", ""]) editor.input(raw, raw.length, { inputType: "deleteContentBackward" });
    editor.selectByUser(0, 0);
    await new Promise<void>(resolve => requestAnimationFrame(() => resolve()));
    editor.keydown({ key: "Backspace", bubbles: true, cancelable: true });
    expect(doc.byId.rule).toBeUndefined();
    expect(view.root.querySelector(".region-failure")).toBeNull();
    expect(view.root.textContent).toContain("After");
  } finally { view.dispose(); }
});

// Undo/redo restores a page's node snapshot in place without unmounting the open
// editor (history.ts: page views keep their lifetime). The mounted editor must
// keep writing to the restored live node; only a removed slot is retired (GH #906).
it("typing after an undo that restores the edited block in place still saves", async () => {
  const { undo } = await import("../document");
  loadSingle(page("Deletion", [blk("a", "Alpha"), blk("b", "Bravo")]));
  const view = mount(() => <FailureBoundary region="This page"><BlockList ids={pageByName("Deletion")!.roots} /></FailureBoundary>);
  try {
    startEditing("b", 5);
    deleteBlock("a");
    expect(doc.byId.a).toBeUndefined();
    expect(undo()).toBe(true);
    expect(doc.byId.a.raw).toBe("Alpha");
    const ed = editorIn(view.root);
    ed.select(5);
    ed.type("!");
    expect(doc.byId.b.raw).toBe("Bravo!");
    expect(view.root.querySelector(".region-failure")).toBeNull();
  } finally { view.dispose(); }
});
