import { editorOffsetFromRenderedRange } from "../render/spans";
import { openSheetCellContextMenu } from "../ui";

/** O(1) paged mounting limit, preserving the no-op for already visible rows. */
export function displayLimitThrough(row: number, visible: number, total: number, pageSize: number): number | null {
  return row < 0 || row < visible ? null : Math.min(total, Math.ceil((row + 1) / pageSize) * pageSize);
}

/** Browser caret-to-source mapping; caller supplies its hidden-property policy. */
export function sheetClickOffset(e: MouseEvent, content: HTMLDivElement | undefined, raw: string, hidden: (key: string) => boolean): number | null {
  if (!content) return null;
  const d = document as Document & { caretRangeFromPoint?: (x: number, y: number) => Range | null };
  const range = d.caretRangeFromPoint?.(e.clientX, e.clientY);
  return range ? editorOffsetFromRenderedRange(content, range, raw, hidden) : null;
}

/** Select before opening a cell menu; handle geometry is shared, editability
 * and removal policy belong to the surface. O(1), no additional effects. */
export function sheetCellMenu(e: MouseEvent, select: () => void, blockId: string,
  remove?: Parameters<typeof openSheetCellContextMenu>[3], handle = false): void {
  e.preventDefault();
  e.stopPropagation();
  select();
  if (handle) {
    const rect = (e.currentTarget as HTMLElement).getBoundingClientRect();
    openSheetCellContextMenu(rect.right, rect.bottom + 2, blockId, remove);
  } else openSheetCellContextMenu(e.clientX, e.clientY, blockId, remove);
}
