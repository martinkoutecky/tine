/** Wry 0.57's WKWebView adapter synthesizes mousedown/up for NSEvent buttons
 * 3/4, with a history default on mouseup, but no auxclick. Chromium also emits
 * auxclick. Consume a completed release once, preserving middle-click links. */
export function installMouseHistory(
  win: Window,
  doc: Document,
  navigate: (direction: "back" | "forward") => void,
  source: "dom" | "gtk" | "webview2" = "dom",
) {
  const gtkOwnsPress = source === "gtk";
  // Per side button (3 Back, 4 Forward): both can be held at once, and each release consumes only its own state.
  type Press = { x: number; y: number; cancelled: boolean };
  const presses = new Map<number, Press>();
  // Release markers by button, keyed to the release's timestamp: Blink gives an auxclick its release's timestamp,
  // so a marker consumes only the auxclick of its own release (#948 review).
  const released = new Map<number, number>();
  // A DOM-seen release whose native delivery has not arrived yet; cleared by the next press.
  let pendingNative: { button: number; cancelled: boolean } | undefined;
  let primaryHeld = false;
  let dragging = false;
  const side = (button: number) => button === 3 || button === 4;
  const cancel = () => { for (const press of presses.values()) press.cancelled = true; };
  const down = (event: MouseEvent) => {
    // A release's auxclick follows it before any other press; a later press ends its eligibility (#948).
    released.clear();
    pendingNative = undefined;
    if (!side(event.button)) { primaryHeld = true; cancel(); return; }
    presses.set(event.button, { x: event.clientX, y: event.clientY,
      cancelled: event.defaultPrevented || primaryHeld || dragging || !!(event.buttons & 7) });
  };
  const move = (event: MouseEvent) => {
    // A release the webview never delivered must not leave a gesture held.
    if (!(event.buttons & 7)) { primaryHeld = false; dragging = false; }
    for (const press of presses.values())
      if (Math.hypot(event.clientX - press.x, event.clientY - press.y) > 5) press.cancelled = true;
  };
  const up = (event: MouseEvent) => {
    // Another ordinary button may still be held.
    if (!side(event.button)) { primaryHeld = !!(event.buttons & 7); return; }
    const current = presses.get(event.button);
    presses.delete(event.button);
    // Suppress Wry's window.history default even for a cancelled gesture.
    const prevented = event.defaultPrevented;
    event.preventDefault();
    const occupied = prevented || dragging || primaryHeld || !!(event.buttons & 7);
    // An unpaired WebView2 release can leave the click to auxclick.
    if (current || occupied || source !== "webview2") released.set(event.button, event.timeStamp);
    const releaseCancelled = !current || current.cancelled || occupied
      || Math.hypot(event.clientX - current.x, event.clientY - current.y) > 5;
    // Only an explicit DOM press can veto the GTK release (#948).
    pendingNative = gtkOwnsPress && current ? { button: event.button, cancelled: releaseCancelled } : undefined;
    if (!gtkOwnsPress && !releaseCancelled)
      navigate(event.button === 3 ? "back" : "forward");
  };
  const aux = (event: MouseEvent) => {
    if (!side(event.button)) return;
    const prevented = event.defaultPrevented;
    event.preventDefault();
    const marker = released.get(event.button);
    released.delete(event.button);
    if (marker !== undefined && marker === event.timeStamp) return;
    if (!gtkOwnsPress && !prevented && !dragging && !primaryHeld && !(event.buttons & 7) && !presses.get(event.button)?.cancelled)
      navigate(event.button === 3 ? "back" : "forward");
  };
  const dragStart = () => { dragging = true; cancel(); };
  const dragEnd = () => { dragging = false; };
  const blur = () => { presses.clear(); released.clear(); pendingNative = undefined; primaryHeld = false; dragging = false; };
  const listeners = { mousedown: down, mouseup: up, mousemove: move, auxclick: aux,
    dragstart: dragStart, dragend: dragEnd, blur };
  const capture = { capture: true };
  for (const [name, listener] of Object.entries(listeners)) win.addEventListener(name, listener as EventListener, capture);
  doc.addEventListener?.("selectionchange", cancel);
  const dispose = () => {
    for (const [name, listener] of Object.entries(listeners)) win.removeEventListener(name, listener as EventListener, capture);
    doc.removeEventListener?.("selectionchange", cancel);
  };
  /** Guard a native history command against a selection or drag. A GTK release
   * also consumes matching DOM gesture state. Windows commands are independent. */
  dispose.acceptNative = (direction: "back" | "forward") => {
    if (!gtkOwnsPress) return !primaryHeld && !dragging;
    const button = direction === "back" ? 3 : 4;
    // The native release ends its gesture: consume the matching press or release, so its verdict never outlives it.
    let cancelled = false;
    const press = presses.get(button);
    if (press) { cancelled = press.cancelled; presses.delete(button); }
    else if (pendingNative?.button === button) { cancelled = pendingNative.cancelled; pendingNative = undefined; }
    return !cancelled && !primaryHeld && !dragging;
  };
  return dispose;
}
