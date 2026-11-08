import { createSignal, onCleanup } from "solid-js";
import { clearOnBindingInvalidated } from "./binding";

function newDraft() {
  const [draft, setDraft] = createSignal("");
  const [expanded, setExpanded] = createSignal(false);
  return { draft, setDraft, expanded, setExpanded, leases: 0 };
}

// One state per mounted thread/surface, plus states retired in this microtask.
// Regrouping or changing width remounts composers synchronously. Transfer their
// unsent text across that render; navigation still releases it, with no file or
// long-lived draft store. No DOM nodes or document nodes are retained.
const drafts = new Map<string, ReturnType<typeof newDraft>>();
clearOnBindingInvalidated(() => drafts.clear());

export function commentDraftState(id: string, surface: string) {
  const key = JSON.stringify([id, surface]);
  let state = drafts.get(key);
  if (!state) { state = newDraft(); drafts.set(key, state); }
  const held = state;
  held.leases++;
  onCleanup(() => {
    held.leases--;
    queueMicrotask(() => {
      if (!held.leases && drafts.get(key) === held) drafts.delete(key);
    });
  });
  return held;
}
