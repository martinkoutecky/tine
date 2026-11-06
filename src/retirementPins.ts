import { createSignal } from "solid-js";

/** Only mounted windows register. Selection/drag changes visit that bounded
 * set, never every historical reference. Each document owns its listeners. */
const documents = new WeakMap<Document, { add: (host: HTMLElement, receive: (pinned: boolean) => void) => () => void }>();
export function trackRetirementPin(host: HTMLElement): { pinned: () => boolean; stop: () => void } {
  const doc = host.ownerDocument;
  let owner = documents.get(doc);
  if (!owner) {
    const targets = new Map<HTMLElement, (pinned: boolean) => void>();
    let dragging = false;
    const update = () => {
      const selection = doc.getSelection();
      for (const [el, receive] of targets) {
        let selected = false;
        if (selection && !selection.isCollapsed) for (let i = 0; i < selection.rangeCount; i++) {
          if (selection.getRangeAt(i).intersectsNode(el)) { selected = true; break; }
        }
        receive(dragging || selected);
      }
    };
    const down = (event: Event) => {
      if ([...targets.keys()].some(el => el.contains(event.target as Node))) dragging = true;
      update();
    };
    const up = () => { dragging = false; update(); };
    const listeners: [string, EventListener][] = [["selectionchange", update], ["pointerdown", down], ["pointerup", up],
      ["pointercancel", up], ["dragstart", down], ["dragend", up], ["drop", up]];
    owner = { add: (el, receive) => {
      if (!targets.size) for (const [name, listener] of listeners) doc.addEventListener(name, listener, true);
      targets.set(el, receive);
      update();
      return () => {
        targets.delete(el);
        if (!targets.size) {
          for (const [name, listener] of listeners) doc.removeEventListener(name, listener, true);
          documents.delete(doc);
        }
      };
    } };
    documents.set(doc, owner);
  }
  const [pinned, setPinned] = createSignal(false);
  return { pinned, stop: owner.add(host, setPinned) };
}
