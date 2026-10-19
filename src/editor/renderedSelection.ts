// Formatting a native text selection inside a RENDERED block, without entering
// edit mode (GH #375, D-20261018-LP2). On a touch-first device a long press in
// a block that is not being edited selects rendered text; the platform's own
// selection handles and menu belong to that rendered text, so Tine leaves the
// selection where it is and offers its formatting actions over it.
//
// - `renderedSelectionTarget(selection)` answers which block a selection lies
//   wholly inside, and its editor-coordinate range (the parser-owned span
//   mapping the click-to-caret path uses), or null: a selection that spans
//   blocks, lies in a read-only page, an annotation, a macro host, a code card or
//   anything else without byte-exact span data gets no toolbar.
// - `applyRenderedSelectionAction(target, action)` runs the editor's own text
//   transformation on the block's editor-visible text and commits it through
//   `setRaw`, the normal block-edit intent (undo, dirty page, guarded save).
// - `selectRenderedRange(...)` re-selects the result after the re-render.
// No Solid state here; the toolbar component owns visibility.
import { blockPageReadOnly, blockWritable, node as docNode, pageByName, setRaw } from "../document";
import { exactEditorOffsetFromRenderedRange, renderedPointFromEditorOffset } from "../render/spans";
import { BUILTIN_HIDE, joinProps, splitProps } from "./properties";
import { calcSource } from "./calc";
import type { Edit } from "./format";
import { SELECTION_ACTIONS, type SelectionAction } from "./selectionActions";

/** The attribute `Rendered` puts on a block's own rendered content (never on a
 *  macro host or an annotation swatch). Its value is the block's runtime id. */
export const RENDERED_BLOCK_ATTR = "data-rendered-block";

/** The editor's selection actions that finish without typing. "Link" leaves a
 *  caret inside `(...)` for the URL, which needs the editor, so it stays there. */
export const renderedSelectionActions: readonly SelectionAction[] = SELECTION_ACTIONS.filter((action) => action.id !== "link");

export interface RenderedSelectionTarget {
  blockId: string;
  /** The block's raw text the offsets were computed against. */
  raw: string;
  format: "md" | "org";
  /** Editor-visible (BUILTIN_HIDE) offsets, start < end. */
  start: number;
  end: number;
  direction: "forward" | "backward";
  root: HTMLElement;
}

function renderedRoot(node: Node): HTMLElement | null {
  const element = node.nodeType === 1 ? node as Element : node.parentElement;
  return element?.closest<HTMLElement>(`[${RENDERED_BLOCK_ATTR}]`) ?? null;
}

/** The rendered block a non-collapsed `selection` lies wholly inside, whether
 *  or not that block offers formatting there (read-only, a code card, a macro's
 *  output). D-20261018-LP2 (b) keys on this: selecting text in block B ends an
 *  edit in A even when B then shows no actions. */
export function renderedSelectionOwner(selection: Selection | null): HTMLElement | null {
  if (!selection || selection.isCollapsed || selection.rangeCount !== 1) return null;
  const range = selection.getRangeAt(0);
  const root = renderedRoot(range.startContainer);
  return root && renderedRoot(range.endContainer) === root ? root : null;
}

/** Which rendered block `selection` lies wholly inside, with its editor range.
 *  One block's span lookup and two offset mappings; O(block bytes). */
export function renderedSelectionTarget(selection: Selection | null): RenderedSelectionTarget | null {
  if (!selection || selection.isCollapsed || selection.rangeCount !== 1) return null;
  const range = selection.getRangeAt(0);
  const root = renderedRoot(range.startContainer);
  // The nearest rendered block owns both ends: an embedded child block is its
  // own owner, so a selection crossing into or out of it belongs to neither.
  if (!root || renderedRoot(range.endContainer) !== root) return null;
  const blockId = root.getAttribute(RENDERED_BLOCK_ATTR);
  const node = blockId ? docNode(blockId) : undefined;
  if (!blockId || !node || blockPageReadOnly(blockId) || !blockWritable(blockId)) return null;
  const format = pageByName(node.page)?.format === "org" ? "org" : "md";
  const raw = node.raw;
  // A calc block's editor shows the fence-stripped expressions, not this text
  // (same refusal as deleteRenderedTextSelection).
  if (calcSource(raw) !== null) return null;
  const start = exactEditorOffsetFromRenderedRange(root, range, raw, BUILTIN_HIDE, format, "start");
  const end = exactEditorOffsetFromRenderedRange(root, { startContainer: range.endContainer, startOffset: range.endOffset }, raw, BUILTIN_HIDE, format, "end");
  if (start === null || end === null || start >= end) return null;
  const direction = selection.anchorNode === range.startContainer && selection.anchorOffset === range.startOffset ? "forward" : "backward";
  return { blockId, raw, format, start, end, direction, root };
}

/** Apply `action` to the target's block text through the normal edit intent.
 *  Returns the resulting editor-visible selection, or null when nothing was
 *  written: the block changed since the selection was read (another surface,
 *  an external reload), it is no longer writable, or the action is a no-op. */
export function applyRenderedSelectionAction(target: RenderedSelectionTarget, action: SelectionAction): Edit | null {
  const node = docNode(target.blockId);
  if (!node || node.raw !== target.raw || !blockWritable(target.blockId)) return null;
  const { visible, hidden } = splitProps(node.raw, BUILTIN_HIDE, target.format);
  const edit = action.apply(visible, target.start, target.end, target.format, target.direction);
  if (edit.text === visible) return null;
  const next = joinProps(edit.text, hidden, target.format);
  if (next === node.raw) return null;
  setRaw(target.blockId, next);
  return edit;
}

/** Select editor range [start, end) in a rendered block's content. False when
 *  either end has no byte-exact rendered position (e.g. it now sits inside a
 *  page link); the caller then drops the selection. */
export function selectRenderedRange(root: HTMLElement, blockId: string, start: number, end: number, direction: "forward" | "backward" | "none" | undefined): boolean {
  const node = docNode(blockId);
  if (!node || !root.isConnected || start >= end) return false;
  const format = pageByName(node.page)?.format === "org" ? "org" : "md";
  const from = renderedPointFromEditorOffset(root, node.raw, BUILTIN_HIDE, format, start, "start");
  const to = renderedPointFromEditorOffset(root, node.raw, BUILTIN_HIDE, format, end, "end");
  const selection = root.ownerDocument.getSelection();
  if (!from || !to || !selection) return false;
  if (direction === "backward") selection.setBaseAndExtent(to.node, to.offset, from.node, from.offset);
  else selection.setBaseAndExtent(from.node, from.offset, to.node, to.offset);
  return true;
}
