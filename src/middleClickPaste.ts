import { isElementNode, listen, onEachWindow } from "./windowRealm";

// Linux middle-click (PRIMARY) paste belongs to the editable you middle-click.
// WebKitGTK pastes on EVERY middle-button release into the focused frame's
// current selection, whatever was clicked and whether or not the page handled
// the mousedown or the click (EventHandler::handleMouseReleaseEvent ->
// handlePasteGlobalSelection, "Ignore handled"). Its only page-visible step is
// the cancelable `paste` event Editor::paste dispatches first. So a middle-click
// on a link opened the link AND pasted at the caret of the block being edited.
//
// A middle press arms a one-shot swallow when its target is outside the focused
// editable, or when a surface consumed it (link surfaces preventDefault middle
// presses, linkGesture.ts). The paste that release triggers is cancelled; the
// guard disarms right after the release. Middle-clicking into the editor's own
// text still pastes there, as everywhere on Linux.

function focusedEditable(doc: Document): Element | null {
  const active = doc.activeElement;
  if (!active || active === doc.body) return null;
  if (active.localName === "textarea" || active.localName === "input") return active;
  // isContentEditable, with the attribute as a fallback where it is missing.
  if ((active as HTMLElement).isContentEditable) return active;
  return active.closest('[contenteditable]:not([contenteditable="false"])');
}

/** Installs the guard in every window. Returns the uninstaller. */
export function installMiddleClickPasteGuard(): () => void {
  return onEachWindow((win) => {
    let armed = false;
    let disarm: ReturnType<typeof setTimeout> | undefined;
    const press = (event: MouseEvent) => {
      clearTimeout(disarm);
      armed = false;
      if (event.button !== 1) return;
      const editable = focusedEditable(win.document);
      if (!editable) return;
      const target = event.target;
      armed = event.defaultPrevented || !isElementNode(target) || !editable.contains(target);
    };
    // The paste runs in the release's default action, after its listeners.
    const release = (event: MouseEvent) => {
      if (event.button !== 1 || !armed) return;
      clearTimeout(disarm);
      disarm = setTimeout(() => { armed = false; }, 0);
    };
    const paste = (event: ClipboardEvent) => {
      if (!armed) return;
      armed = false;
      event.preventDefault();
      event.stopImmediatePropagation();
    };
    // Bubble phase on the window: runs after element and delegated (document)
    // handlers, so a link surface's preventDefault is visible here.
    const stops = [
      listen(win, "mousedown", press),
      listen(win, "mouseup", release, { capture: true }),
      listen(win, "paste", paste, { capture: true }),
    ];
    return () => {
      clearTimeout(disarm);
      for (const stop of stops) stop();
    };
  });
}
