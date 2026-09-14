/** One editor commit/frame owns only displacement caused above that editor.
 * User scrolling and another focus/scroll owner always take precedence. */
export function captureEditorScrollAnchor(editor: HTMLTextAreaElement, scroller: HTMLElement | null) {
  if (!scroller || !editor.isConnected || document.activeElement !== editor) return null;
  const top = editor.getBoundingClientRect().top;
  const scrollTop = scroller.scrollTop;
  let canceled = false;
  const cancel = () => { canceled = true; };
  const gestures = ["wheel", "touchstart", "pointerdown"] as const;
  for (const type of gestures) scroller.addEventListener(type, cancel, { passive: true, capture: true });
  const cleanup = () => {
    for (const type of gestures) scroller.removeEventListener(type, cancel, true);
  };
  return {
    cancel() { cancel(); cleanup(); },
    restore(currentEditor: HTMLTextAreaElement | null = editor) {
      cleanup();
      if (canceled || !currentEditor?.isConnected || !scroller.isConnected || !scroller.contains(currentEditor) ||
          document.activeElement !== currentEditor || scroller.scrollTop !== scrollTop) return;
      const displacement = currentEditor.getBoundingClientRect().top - top;
      if (Math.abs(displacement) > 0.5) scroller.scrollTop += displacement;
    },
  };
}
