import { createEffect, onCleanup, untrack, type Accessor } from "solid-js";
import { createStore, reconcile } from "solid-js/store";
import type { QueryHit } from "./types";
import { bindingIdentity } from "./binding";
import { graphEpoch } from "./graphSession";
import { blockDtoExternalId } from "./blockIdentity";
import { adoptResults, resultTicket, resultsEpoch, resultsSettled } from "./queryResultsEpoch";

/** Runtime ids may be positional after an external reparse. Prefer authored
 * identity; unchanged anonymous rows retain their content within the exact file. */
export function searchHitKey(hit: QueryHit): string {
  if (hit.entity === "page") return JSON.stringify(["page", hit.page.kind, hit.page.path ?? hit.page.name]);
  const authored = hit.block.properties?.some(([key, value]) => key.trim().toLowerCase() === "id" && value.trim());
  return JSON.stringify(["block", hit.kind, hit.path ?? hit.page, authored
    ? ["id", blockDtoExternalId(hit.block)]
    : ["text", hit.block.raw, hit.block.breadcrumb ?? []]]);
}

function keyedHits(hits: QueryHit[]) {
  const occurrences = new Map<string, number>();
  return hits.map(hit => {
    const identity = searchHitKey(hit), occurrence = occurrences.get(identity) ?? 0;
    occurrences.set(identity, occurrence + 1);
    // Identical anonymous rows remain separate occurrences in answer order.
    return { key: `${occurrence}:${identity}`, hit };
  });
}

/** Reconcile bounded search answers by physical identity. Reactive row objects
 * survive new IPC DTOs, so unchanged rows keep DOM, focus and pointer targets.
 * A different question/binding always drops the old answer's row ownership. */
export function createSearchResultRows(
  hits: Accessor<QueryHit[]>, identity: Accessor<string>, root: Accessor<HTMLElement | undefined>,
) {
  const [rows, setRows] = createStore<{ key: string; hit: QueryHit }[]>([]);
  let scope = "";
  let alive = true;
  let pointer: { x: number; y: number } | undefined;
  onCleanup(() => { alive = false; });
  createEffect(() => {
    const element = root();
    if (!element) return;
    const move = (event: PointerEvent) => { pointer = { x: event.clientX, y: event.clientY }; };
    const leave = () => { pointer = undefined; };
    element.addEventListener("pointermove", move);
    element.addEventListener("pointerleave", leave);
    onCleanup(() => { element.removeEventListener("pointermove", move); element.removeEventListener("pointerleave", leave); });
  });
  createEffect(() => {
    resultsEpoch(); resultsSettled(); hits();
    // Reconciliation owns display DTOs, while the membership resource owns
    // the proof that an answer can be acted on. Transfer only current tickets;
    // never make a retained/coalesced answer actionable by ourselves.
    queueMicrotask(() => {
      if (!alive) return;
      const current = new Map(keyedHits(hits()).map(({ key, hit }) => [key, hit]));
      for (const row of rows) {
        const hit = current.get(row.key);
        const ticket = hit && resultTicket(hit);
        if (ticket) adoptResults(row.hit, ticket);
      }
    });
  });
  createEffect(() => {
    const nextScope = `${bindingIdentity()}\0${graphEpoch()}\0${identity()}`;
    const next = keyedHits(hits()).map(({ key, hit }) => ({ key, hit: structuredClone(hit) }));
    untrack(() => {
      const restore = nextScope === scope ? captureAnchor(root(), pointer) : undefined;
      if (nextScope !== scope) setRows([]);
      scope = nextScope;
      setRows(reconcile(next, { key: "key" }));
      queueMicrotask(() => { if (alive && scope === nextScope) restore?.(); });
    });
  });
  return () => rows.map(row => row.hit);
}

/** Hold a surviving visible result at its viewport position when another row
 * above it changes height or membership. Read/write the actual scroll owner;
 * yield if the user scrolls between capture and layout. No timers or freeze. */
function captureAnchor(root: HTMLElement | undefined, pointer: { x: number; y: number } | undefined) {
  if (!root) return;
  let scroller = root.parentElement;
  const win = root.ownerDocument.defaultView;
  while (scroller && !/(auto|scroll)/.test(win?.getComputedStyle(scroller).overflowY ?? "")) scroller = scroller.parentElement;
  if (!scroller) return;
  const bounds = scroller.getBoundingClientRect();
  const pointed = pointer && root.ownerDocument.elementFromPoint?.(pointer.x, pointer.y)?.closest<HTMLElement>("[data-query-result]");
  const visible = [...root.querySelectorAll<HTMLElement>("[data-query-result]")].filter(row => {
    const rect = row.getBoundingClientRect();
    return rect.bottom > bounds.top && rect.top < bounds.bottom;
  });
  const anchors = (pointed && root.contains(pointed) ? [pointed, ...visible.filter(row => row !== pointed)] : visible)
    .map(row => ({ row, top: row.getBoundingClientRect().top }));
  const scrollTop = scroller.scrollTop;
  return () => {
    if (!scroller.isConnected || scroller.scrollTop !== scrollTop) return;
    const anchor = anchors.find(({ row }) => row.isConnected);
    if (anchor) scroller.scrollTop += anchor.row.getBoundingClientRect().top - anchor.top;
  };
}
