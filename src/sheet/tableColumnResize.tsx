import { Show, createMemo, createSignal, onCleanup, type JSX } from "solid-js";
import { blockProperty, blockWritable, node, pageByName, readPageProperty, setBlockProperty, setPageProperty, withUndoUnit } from "../document";
import { parseTableColumnWidths, serializeTableColumnWidths, TABLE_COLUMN_MAX_WIDTH } from "./config";
import { windowOf } from "../windowRealm";

type Home = { kind: "block"; id: string; page: string; widths: ReadonlyMap<string, number> }
  | { kind: "page"; page: string; widths: ReadonlyMap<string, number> };
const minWidth = (column: string) => column === "title" ? 180 : 90;
const clamp = (column: string, width: number) => Math.min(TABLE_COLUMN_MAX_WIDTH, Math.max(minWidth(column), Math.round(width)));

/** Table width controller for one sheet owner. Reads only its width property and
 * writes one property through the document edit door on pointer release. Drag
 * preview is ephemeral; cancel/Escape publishes nothing. */
export function createTableColumnResize(ownerId: string, schemaPage?: string): {
  widths: () => ReadonlyMap<string, number>;
  resizing: () => string | null;
  handle: (column: string, label: string) => JSX.Element;
} {
  const home = createMemo<Home | null>(() => {
    const owner = node(ownerId);
    if (owner) return { kind: "block", id: ownerId, page: owner.page,
      widths: parseTableColumnWidths(blockProperty(ownerId, "tine.table-widths") ?? "") };
    if (!schemaPage) return null;
    return { kind: "page", page: schemaPage,
      widths: parseTableColumnWidths(readPageProperty(schemaPage, "tine.table-widths") ?? "") };
  });
  const writable = () => {
    const target = home();
    return target?.kind === "block" ? blockWritable(target.id)
      : target?.kind === "page" ? pageByName(target.page)?.readOnly === false : false;
  };
  const [preview, setPreview] = createSignal<ReadonlyMap<string, number> | null>(null);
  const [resizing, setResizing] = createSignal<string | null>(null);
  const widths = () => preview() ?? home()?.widths ?? new Map<string, number>();
  let cancelActive: (() => void) | undefined;
  const publish = (next: ReadonlyMap<string, number>) => {
    const target = home();
    if (!target || !writable()) return;
    const value = serializeTableColumnWidths(next) || null;
    withUndoUnit("sheet:table-column-width", [target.page], () => {
      if (target.kind === "block") setBlockProperty(target.id, "tine.table-widths", value);
      else setPageProperty(target.page, "tine.table-widths", value);
    });
  };
  const begin = (column: string, event: PointerEvent) => {
    if (event.button !== 0 || !writable()) return;
    event.preventDefault(); event.stopPropagation(); cancelActive?.();
    const measured = (event.currentTarget as HTMLElement).parentElement?.getBoundingClientRect().width ?? 0;
    const startWidth = measured || widths().get(column) || minWidth(column);
    const startX = event.clientX;
    const pointerId = event.pointerId;
    const dragWindow = windowOf(event); // the drag's own window (P1)
    let moved = false;
    setResizing(column);
    const cleanup = () => {
      dragWindow.removeEventListener("pointermove", move);
      dragWindow.removeEventListener("pointerup", up);
      dragWindow.removeEventListener("pointercancel", abort);
      dragWindow.removeEventListener("keydown", keydown);
      cancelActive = undefined;
    };
    const finish = (commit: boolean) => {
      const next = preview();
      cleanup(); setPreview(null); setResizing(null);
      if (commit && moved && next) publish(next);
    };
    const move = (next: PointerEvent) => {
      if (next.pointerId !== pointerId) return;
      next.preventDefault(); moved = true;
      const update = new Map(home()?.widths ?? []);
      update.set(column, clamp(column, startWidth + next.clientX - startX));
      setPreview(update);
    };
    const up = (next: PointerEvent) => { if (next.pointerId === pointerId) finish(true); };
    const abort = (next: PointerEvent) => { if (next.pointerId === pointerId) finish(false); };
    const keydown = (next: KeyboardEvent) => { if (next.key === "Escape") { next.preventDefault(); finish(false); } };
    cancelActive = () => finish(false);
    dragWindow.addEventListener("pointermove", move);
    dragWindow.addEventListener("pointerup", up);
    dragWindow.addEventListener("pointercancel", abort);
    dragWindow.addEventListener("keydown", keydown);
  };
  const reset = (column: string, event: MouseEvent) => {
    event.preventDefault(); event.stopPropagation();
    if (!writable()) return;
    const next = new Map(home()?.widths ?? []);
    if (next.delete(column)) publish(next);
  };
  onCleanup(() => cancelActive?.());
  const handle = (column: string, label: string) => <Show when={writable()}>
    <button type="button" class="sheet-table-column-resize-handle"
      data-sheet-resize-handle={column} aria-label={`Resize ${label} column`}
      title="Drag to resize; double-click to reset"
      onPointerDown={(event) => begin(column, event)}
      onClick={(event) => { event.preventDefault(); event.stopPropagation(); }}
      onDblClick={(event) => reset(column, event)} />
  </Show>;
  return { widths, resizing, handle };
}
