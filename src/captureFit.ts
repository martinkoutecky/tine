/**
 * The height Quick Capture's window needs for what it shows now: the capture
 * shell plus whatever floats over or below it. The window grows to this (with
 * a screen cap) on every fit, so anything that can be on screen in Capture and
 * is not counted here is clipped at the window's bottom edge.
 */
export function captureContentHeight(doc: Document, datePickerOpen: boolean): number {
  const PAD = 18; // breathing room below the content
  const shell = doc.querySelector<HTMLElement>(".capture-shell");
  let h = shell ? shell.getBoundingClientRect().bottom : 72;
  const popup = doc.querySelector<HTMLElement>(".autocomplete");
  if (popup) h = Math.max(h, popup.getBoundingClientRect().bottom);
  // The format toolbar floats just below the editing line (see capture.css) —
  // grow the window so it isn't clipped at the bottom edge.
  const toolbar = doc.querySelector<HTMLElement>(".sel-toolbar");
  if (toolbar) h = Math.max(h, toolbar.getBoundingClientRect().bottom);
  // The date picker is fixed-positioned and clamps to the *current* window
  // height, so reserve a fixed slab tall enough to show it (it then positions
  // itself once the resize takes effect — its placement is window-size-aware).
  if (datePickerOpen) h = Math.max(h, 392);
  // A consent prompt (outside file, custom scheme) is a modal sized from the
  // window itself (`.modal-overlay` pads 12vh above and 8px below,
  // `.export-modal` caps at 80vh), so its own rectangle shrinks with a small
  // window. Reserve its full content height under those two rules instead.
  const prompt = doc.querySelector<HTMLElement>(".link-scheme-prompt");
  if (prompt) {
    const content = prompt.scrollHeight;
    h = Math.max(h, content / 0.8, (content + 8) / 0.88);
  }
  return Math.ceil(h + PAD);
}
