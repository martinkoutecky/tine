import { createStore } from "solid-js/store";
import { clearOnBindingInvalidated } from "../../binding";
import { graphEpoch } from "../../graphSession";
import { blockPageReadOnly, node, toggleCollapse } from "../../document";
import type { CollapseSurfaceApi } from "../Block";

// Presentation only: never change a document node, property, undo unit or save.
// Keep choices across page remounts, and discard them on a graph session change.
// Deliberate difference from editable folds: reloading starts from collapsed::.
const [view, setView] = createStore<{ epoch: number; folds: Record<string, boolean> }>({ epoch: -1, folds: {} });
clearOnBindingInvalidated(() => setView({ epoch: -1, folds: {} }));
const key = (id: string) => JSON.stringify([node(id)?.page, id]);
export const readOnlyFold: CollapseSurfaceApi = {
  collapsed: (id, stored) => view.epoch === graphEpoch() ? view.folds[key(id)] ?? stored : stored,
  toggle: (id, current) => readOnlyFold.setMany([id], !current),
  setMany: (ids, collapsed) => {
    if (view.epoch !== graphEpoch()) setView({ epoch: graphEpoch(), folds: {} });
    for (const id of ids) setView("folds", key(id), collapsed);
  },
};

// Read-only sources always use presentation folds, including inside embeds
// whose inherited surface may otherwise write a host collapse property.
export const viewFoldSurface = (id: string, inherited: CollapseSurfaceApi | null) =>
  blockPageReadOnly(id) ? readOnlyFold : inherited;

export function toggleViewFold(id: string, surface: CollapseSurfaceApi | null, current: boolean) {
  if (surface) surface.toggle(id, current);
  else toggleCollapse(id);
}
