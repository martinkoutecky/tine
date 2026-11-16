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

/** `viewFoldSurface` for one pass over an outline (window sizing, height
 * estimates): read-only belongs to the page, so a pass asks it once per page,
 * not once per block of a 2000-block page (BIGLOAD-090). Make one per pass. */
export function pageFoldSurfaces(inherited: CollapseSurfaceApi | null): (id: string, page: string) => CollapseSurfaceApi | null {
  const byPage = new Map<string, CollapseSurfaceApi | null>();
  return (id, page) => {
    let surface = byPage.get(page);
    if (surface === undefined) byPage.set(page, surface = viewFoldSurface(id, inherited));
    return surface;
  };
}

export function toggleViewFold(id: string, surface: CollapseSurfaceApi | null, current: boolean) {
  if (surface) surface.toggle(id, current);
  else toggleCollapse(id);
}
