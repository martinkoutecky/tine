// The canonical comparison key two projections must share to be "equal", ported
// from graph-check.mjs:644-646. Uses the SAME `canonJSON` (key-sorted, drops
// span/aligns/span_map) as the differential gate, so an in-app diff and the gate
// agree on what counts as a divergence.
import { canonJSON } from "./vendor/compare.mjs";

export interface Projection {
  blocks?: unknown[];
  refs?: { page: string[]; block: string[] };
}

export function projectionKey(projection: Projection | unknown[] | null | undefined): string {
  const p = projection as Projection | undefined;
  return canonJSON({
    blocks: p?.blocks || (Array.isArray(projection) ? projection : []),
    refs: p?.refs || { page: [], block: [] },
  });
}
