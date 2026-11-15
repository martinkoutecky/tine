/** Wry 0.57's WKWebView adapter synthesizes mousedown/up for NSEvent buttons
 * 3/4, with a history default on mouseup, but no auxclick. Chromium also emits
 * auxclick. Consume a completed release once, preserving middle-click links. */
export function installMouseHistory(win: Window, doc: Document, navigate: (direction: "back" | "forward") => void) {
  let press: { button: number; x: number; y: number; cancelled: boolean } | undefined;
  let released: number | undefined;
  // A DOM-seen release whose native delivery has not arrived yet; cleared by the next press.
  let pendingNative: { button: number; cancelled: boolean } | undefined;
  let primaryHeld = false;
  let dragging = false;
  const side = (button: number) => button === 3 || button === 4;
  const cancel = () => { if (press) press.cancelled = true; };
  const down = (event: MouseEvent) => {
    released = undefined;
    pendingNative = undefined;
    if (!side(event.button)) { primaryHeld = true; cancel(); return; }
    press = { button: event.button, x: event.clientX, y: event.clientY,
      cancelled: event.defaultPrevented || primaryHeld || dragging || !!(event.buttons & 7) };
  };
  const move = (event: MouseEvent) => {
    // A release the webview never delivered must not leave a gesture held.
    if (!(event.buttons & 7)) { primaryHeld = false; dragging = false; }
    if (press && Math.hypot(event.clientX - press.x, event.clientY - press.y) > 5) cancel();
  };
  const up = (event: MouseEvent) => {
    if (!side(event.button)) { primaryHeld = false; return; }
    const current = press;
    press = undefined;
    released = event.button;
    // Suppress Wry's window.history default even for a cancelled gesture.
    const prevented = event.defaultPrevented;
    event.preventDefault();
    const releaseCancelled = current?.button !== event.button || !!current?.cancelled || prevented || dragging || primaryHeld
      || (!!current && Math.hypot(event.clientX - current.x, event.clientY - current.y) > 5);
    // Only an explicit DOM press can veto the native release; an absent one never does (#948).
    pendingNative = current?.button === event.button ? { button: event.button, cancelled: releaseCancelled } : undefined;
    if (!releaseCancelled)
      navigate(event.button === 3 ? "back" : "forward");
  };
  const aux = (event: MouseEvent) => {
    if (!side(event.button)) return;
    const prevented = event.defaultPrevented;
    event.preventDefault();
    if (released === event.button) { released = undefined; return; }
    if (!prevented && !dragging && !primaryHeld && !press?.cancelled)
      navigate(event.button === 3 ? "back" : "forward");
  };
  const dragStart = () => { dragging = true; cancel(); };
  const dragEnd = () => { dragging = false; };
  const blur = () => { press = undefined; released = undefined; pendingNative = undefined; primaryHeld = false; dragging = false; };
  const listeners = { mousedown: down, mouseup: up, mousemove: move, auxclick: aux,
    dragstart: dragStart, dragend: dragEnd, blur };
  const capture = { capture: true };
  for (const [name, listener] of Object.entries(listeners)) win.addEventListener(name, listener as EventListener, capture);
  doc.addEventListener?.("selectionchange", cancel);
  const dispose = () => {
    for (const [name, listener] of Object.entries(listeners)) win.removeEventListener(name, listener as EventListener, capture);
    doc.removeEventListener?.("selectionchange", cancel);
  };
  /** Whether a native (WM_APPCOMMAND / X11) side-button release should navigate, in O(1). This recognizer is the one
   * owner of the gesture: a selection, drag or cancelled press vetoes it; with no DOM press seen, native decides. */
  dispose.acceptNative = (direction: "back" | "forward") => {
    const button = direction === "back" ? 3 : 4;
    // The native release ends its gesture: consume the matching press or release, so its verdict never outlives it.
    let cancelled = false;
    if (press?.button === button) { cancelled = press.cancelled; press = undefined; }
    else if (pendingNative?.button === button) { cancelled = pendingNative.cancelled; pendingNative = undefined; }
    return !cancelled && !primaryHeld && !dragging;
  };
  return dispose;
}
