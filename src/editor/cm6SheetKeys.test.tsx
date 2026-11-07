import { afterEach, beforeAll, expect, it } from "vitest";
import { render } from "solid-js/web";
import { Cm6BlockEditor } from "../components/Cm6BlockEditor";
import { SheetCellContext } from "../sheet/context";
import { cellOwner, resetCellSelectionForTests } from "../sheet/selection";
import { editingId, endEdit, startEditing } from "../editorController";
import { resetStore } from "../document";
import { loadSingle } from "../document/workingSet";
import { blockRegions, initParser } from "../render/parse";
import { editorIn } from "../tests/editorDriver";
import type { BlockDto } from "../types";

let dispose: (() => void) | undefined;
beforeAll(() => {
  if (!Range.prototype.getClientRects) Range.prototype.getClientRects = () => [] as unknown as DOMRectList;
  if (!Range.prototype.getBoundingClientRect) Range.prototype.getBoundingClientRect = () => new DOMRect();
  return initParser();
});
afterEach(() => {
  dispose?.(); dispose = undefined; endEdit("page-navigation");
  resetCellSelectionForTests(); resetStore(); document.body.replaceChildren();
});
const block = (id: string, raw: string, children: BlockDto[] = []): BlockDto => ({
  id, raw, children, collapsed: false,
  properties: blockRegions(raw, "md").properties.filter(p => p.primary).map(p => [p.key, p.value]),
});

for (const view of ["table", "board"]) it.each([
  ["host", "ArrowDown", "first"],
  ["first", "ArrowDown", "last"],
  ["last", "ArrowUp", "first"],
  ["first", "ArrowUp", "host"],
  ["middle", "ArrowDown", "middle-reply"],
  ["middle-reply", "ArrowUp", "middle"],
] as const)(`CM6 ${view}-cell navigation from %s with %s skips body threads and retains thread steps`, (id, key, target) => {
  loadSingle({ name: "Cell", title: "Cell", kind: "page", format: "md", pre_block: null, blocks: [
    block("table", `Table\ntine.view:: ${view}`, [block("row", "", [block("host", "Cell", [
      block("start", "Remark\ntine.quote:: Cell\ntine.view:: grid", [block("start-reply", "Reply")]),
      block("first", "First"),
      block("middle", "Remark\ntine.quote:: Cell", [block("middle-reply", "Reply")]),
      block("last", "Last"),
    ])])]),
  ] });
  const cell = { gridId: "table", row: 0, col: 0 };
  startEditing(id, key === "ArrowDown" ? Number.MAX_SAFE_INTEGER : 0, cellOwner(cell));
  const root = document.createElement("div"); document.body.append(root);
  dispose = render(() => <SheetCellContext.Provider value={cell}><Cm6BlockEditor id={id} surface="main" scope={null} /></SheetCellContext.Provider>, root);
  const editor = editorIn(root);
  editor.select(key === "ArrowDown" ? editor.value().length : 0);
  editor.keydown({ key });
  expect(editingId()).toBe(target);
});
