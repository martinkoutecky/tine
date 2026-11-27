// GH #975: "Edit clock entries" opens a block with its LOGBOOK drawer in the text.
import { afterEach, beforeAll, expect, it, vi } from "vitest";
import { render } from "solid-js/web";
import { backend } from "../backend";
import { clearSelection, node, pageByName, resetStore, selectBlock, undo } from "../document";
import { paletteCommands } from "../keybindings";
import { ensurePageLoaded } from "../document/workingSet";
import { bindTestHost, submittedPages } from "../document/host/wiring.test.support";
import { answerOpensFromDocument } from "../document/host/documentHost.test.support";
import { editingId, endEdit, startEditing } from "../editorController";
import { clockEntriesOffered } from "../editor/clockEntries";
import { initParser } from "../render/parse";
import { closeContextMenu, openContextMenu } from "../ui";
import { editorIn, maybeEditorIn } from "../tests/editorDriver";
import { useEditorKind } from "../tests/editorKind";
import { Block } from "./Block";
import { ContextMenu } from "./ContextMenu";

useEditorKind("cm6");
beforeAll(initParser);
afterEach(() => { closeContextMenu(); endEdit("page-navigation"); resetStore(); vi.restoreAllMocks(); document.body.replaceChildren(); });

const ID = "11111111-1111-4111-8111-111111111111";
const ROW1 = "CLOCK: [2026-11-20 Fri 09:00:00]--[2026-11-20 Fri 10:00:00] =>  01:00:00";
const ROW2 = "CLOCK: [2026-11-21 Sat 09:00:00]--[2026-11-21 Sat 10:00:00] =>  01:00:00";
const FIXED = "CLOCK: [2026-11-20 Fri 09:00:00]--[2026-11-20 Fri 10:30:00] =>  01:30:00";
const drawer = `:LOGBOOK:\n${ROW1}\n${ROW2}\n:END:`;
const RAW = {
  md: `TODO write the report\nid:: ${ID}\ncollapsed:: true\n${drawer}\nnotes below`,
  org: `TODO write the report\n:PROPERTIES:\n:id: ${ID}\n:collapsed: true\n:END:\n${drawer}\nnotes below`,
};

async function mount(format: "md" | "org", raw: string, readOnly = false) {
  const host = await bindTestHost();
  answerOpensFromDocument(host);
  ensurePageLoaded({ id: `pages/Tasks.${format}`, name: "Tasks", title: "Tasks", kind: "page", format, pre_block: null, read_only: readOnly,
    blocks: [{ id: "task", raw, collapsed: false, children: [] }] });
  const submit = vi.spyOn(backend(), "pageSubmit");
  const root = document.createElement("div"); root.className = "main-content"; document.body.append(root);
  const menu = document.createElement("div"); document.body.append(menu);
  const disposers = [render(() => <Block id="task" />, root), render(() => <ContextMenu />, menu)];
  await vi.waitFor(() => expect(root.querySelector('[data-block-id="task"] .block-content')).not.toBeNull());
  return { root, submit, dispose: () => disposers.forEach(d => d()) };
}
const menuItem = (label: string) => [...document.querySelectorAll<HTMLElement>(".ctx-item")].find(el => el.textContent?.trim() === label);
function openClockEntries() {
  openContextMenu(10, 10, "task");
  const item = menuItem("Edit clock entries");
  expect(item).toBeDefined();
  item!.click();
}

it.each(["md", "org"] as const)("the menu opens the drawer at its place with the caret on the first CLOCK row; exiting untouched writes nothing (%s)", async format => {
  const { root, submit, dispose } = await mount(format, RAW[format]);
  try {
    openClockEntries();
    const editor = await vi.waitFor(() => { expect(maybeEditorIn(root, "task")).not.toBeNull(); return editorIn(root, "task"); });
    expect(editor.value()).toBe(`TODO write the report\n${drawer}\nnotes below`);
    await vi.waitFor(() => expect(editor.selection()[0]).toBe(editor.value().indexOf(ROW1)));
    editor.blur();
    endEdit("blur");
    expect(node("task").raw).toBe(RAW[format]);
    expect(submit).not.toHaveBeenCalled();
  } finally { dispose(); }
});

it.each(["md", "org"] as const)("correcting one row changes exactly that row; undo restores it; normal re-entry hides the drawer again (%s)", async format => {
  const { root, submit, dispose } = await mount(format, RAW[format]);
  try {
    openClockEntries();
    const editor = await vi.waitFor(() => editorIn(root, "task"));
    editor.focus();
    editor.input(editor.value().replace(ROW1, FIXED));
    const fixed = RAW[format].replace(ROW1, FIXED);
    expect(node("task").raw).toBe(fixed);
    await vi.waitFor(() => expect(submittedPages(submit, "Tasks").at(-1)?.blocks[0].raw).toBe(fixed));
    endEdit("blur");
    // Normal editing is unchanged: the drawer is hidden again.
    startEditing("task");
    const again = await vi.waitFor(() => editorIn(root, "task"));
    expect(again.value()).toBe("TODO write the report\nnotes below");
    endEdit("blur");
    undo();
    expect(node("task").raw).toBe(RAW[format]);
  } finally { dispose(); }
});

it("Enter in the session is a line break, not a block split", async () => {
  const { root, dispose } = await mount("md", RAW.md);
  try {
    openClockEntries();
    const editor = await vi.waitFor(() => editorIn(root, "task"));
    editor.focus();
    await vi.waitFor(() => expect(editor.selection()[0]).toBe(editor.value().indexOf(ROW1)));
    editor.keydown({ key: "Enter" });
    expect(pageByName("Tasks")!.roots).toEqual(["task"]);
    expect(editor.value()).toBe(`TODO write the report\n:LOGBOOK:\n\n${ROW1}\n${ROW2}\n:END:\nnotes below`);
    expect(editingId()).toBe("task");
  } finally { dispose(); }
});

it("the command is offered only for a writable outline block with a LOGBOOK", async () => {
  const plain = await mount("md", `TODO no history\nid:: ${ID}`);
  try {
    openContextMenu(10, 10, "task");
    expect(menuItem("Properties…")).toBeDefined();
    expect(menuItem("Edit clock entries")).toBeUndefined();
  } finally { closeContextMenu(); plain.dispose(); resetStore(); document.body.replaceChildren(); }
  const readOnly = await mount("md", RAW.md, true);
  try {
    openContextMenu(10, 10, "task");
    expect(menuItem("Edit clock entries")).toBeUndefined();
    expect(clockEntriesOffered("task")).toBe(false);
  } finally { closeContextMenu(); readOnly.dispose(); resetStore(); }
  ensurePageLoaded({ id: "pages/Sheet.md", name: "Sheet", title: "Sheet", kind: "page", format: "md", pre_block: null,
    blocks: [{ id: "grid", raw: "Grid\ntine.view:: grid", properties: [["tine.view", "grid"]], collapsed: false, children: [
      { id: "row", raw: "Row", collapsed: false, children: [{ id: "cell", raw: RAW.md, collapsed: false, children: [] }] }] }] });
  expect(clockEntriesOffered("cell")).toBe(false);
  expect(clockEntriesOffered("grid")).toBe(false);
});

it("the palette offers it for the block edited as it opened, or the selected block", async () => {
  const { root, dispose } = await mount("md", RAW.md);
  try {
    const listed = () => paletteCommands().find(c => c.id === "editor/edit-clock-entries");
    expect(listed()).toBeUndefined();
    selectBlock("task");
    expect(listed()?.label).toBe("Edit clock entries");
    clearSelection();
    startEditing("task");
    await vi.waitFor(() => editorIn(root, "task"));
    const command = listed();
    endEdit("blur"); // opening the palette ends the edit
    command!.run();
    const editor = await vi.waitFor(() => editorIn(root, "task"));
    expect(editor.value()).toContain(ROW1);
  } finally { dispose(); }
});
