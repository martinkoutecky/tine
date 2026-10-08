// Interactive targets that must NOT enter edit on mousedown (mirrors OG's
// target-forbidden-edit?). Their own handlers run on click, so entering edit
// first would swap the DOM out from under them.
export const FORBID_EDIT_SELECTOR = [
  "a",
  "button",
  "input",
  "textarea",
  "select",
  "video",
  "audio",
  "iframe",
  "details",
  "summary",
  ".block-ref",
  // A live block nested inside a macro host (notably {{embed}}) owns its control
  // column. Letting the outer host arm an edit gesture on this mousedown removes
  // the control before the browser can deliver its click.
  ".block-controls",
  // The embed owns its whole surface, including gaps between nested rows.
  // Nested editors remain allowed: their own host does not contain this ancestor.
  ".embed-block",
  ".block-marker",
  ".clock-badge",
  ".date-chip",
  ".hl-prefix",
  ".inline-image-wrap",
  ".media-embed-wrap",
  ".embed-iframe-wrap",
  // Query-macro controls: they run their own action on CLICK (toggle collapse,
  // rename title, navigate to a result page, sort a column).
  ".query-collapse",
  ".query-title-editable",
  ".query-page",
  ".query-crumb",
  ".qt-page",
  ".query-table th",
].join(", ");

// A native scrollbar's hit target is its scroll container, not an interactive
// DOM child, so a press on a code block's horizontal scrollbar would otherwise
// arm edit entry and unmount the node whose scrollbar the browser is dragging.
// Geometry is in the element's own unscaled offset units so a zoomed page (CSS
// transform / browser zoom) still hits the gutter, not the text above it.
function pressIsOnHorizontalScrollbar(node: HTMLElement, e: MouseEvent): boolean {
  if (node.scrollWidth <= node.clientWidth) return false;
  if (!node.offsetWidth || !node.offsetHeight) return false;
  const rect = node.getBoundingClientRect();
  if (!rect.width || !rect.height) return false;
  const x = ((e.clientX - rect.left) * node.offsetWidth) / rect.width;
  const y = ((e.clientY - rect.top) * node.offsetHeight) / rect.height;
  const style = getComputedStyle(node);
  const right = node.offsetWidth - (parseFloat(style.borderRightWidth) || 0);
  const bottom = node.offsetHeight - (parseFloat(style.borderBottomWidth) || 0);
  return y >= node.clientTop + node.clientHeight && y < bottom && x >= node.clientLeft && x < right;
}

export function forbidsEditEntry(e: MouseEvent): boolean {
  const target = e.target as Element | null;
  const host = e.currentTarget as Element;
  // A press that is not in this host's DOM at all still arrives here when a
  // surface was floated through a bare `<Portal>` (Solid walks `_$host`, the
  // LOGICAL parent): the query sheet is exactly that. It landed on the floating
  // surface, never on this block's text, and entering the editor would unmount
  // the surface before its own click could run (GH #619). `FloatingPortal`
  // severs that chain; this is the second line, ported from master 2617ff194.
  if (target && !host.contains(target)) return true;
  for (let node = target; node && host.contains(node); node = node.parentElement) {
    if (node instanceof HTMLElement && pressIsOnHorizontalScrollbar(node, e)) return true;
    if (node === host) break;
  }
  const hit = target?.closest?.(FORBID_EDIT_SELECTOR);
  return !!hit && host.contains(hit);
}
