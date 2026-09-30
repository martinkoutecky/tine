import { For, Show, createEffect, createMemo, createSignal, onMount, onCleanup, untrack, type JSX } from "solid-js";
import type { RefGroup, PageKind } from "../types";
import { openPageTarget, openPageTargetInNewTab } from "../router";
import { openPageInSidebar, openPageContextMenu } from "../ui";
import { openRouteInOtherPane } from "../panes";
import { internalLinkAuxClick, internalLinkDest, internalLinkMouseDown } from "../linkGesture";
import { shouldOpenTextContextMenu } from "../contextMenuPolicy";
import { observeNear, unobserveNear } from "../lazyObserve";
import { LiveRefGroup } from "./LiveRefGroup";

interface QueryGroupProps { group: () => RefGroup | undefined; flat?: boolean }

/** Present the complete keyed List without one graph-sized DOM commit. Existing
 * groups survive membership refreshes; at most 32 new shells mount per frame.
 * Pending groups reserve the same approximate height as their eventual shells.
 * Without browser layout all groups mount immediately, like observeNear. */
export function QueryGroups(props: { groups: () => Map<string, RefGroup>; flat?: boolean }): JSX.Element {
  const [keys, setKeys] = createSignal<string[]>([]);
  let frame: number | undefined;
  let generation = 0;
  const cancel = () => {
    generation += 1;
    if (frame !== undefined) cancelAnimationFrame(frame);
    frame = undefined;
  };
  onCleanup(cancel);
  createEffect(() => {
    const all = [...props.groups().keys()];
    cancel();
    if (typeof IntersectionObserver === "undefined" || typeof requestAnimationFrame === "undefined") {
      setKeys(all);
      return;
    }
    const current = generation;
    const retained = new Set(untrack(keys));
    const pending = all.filter((key) => !retained.has(key));
    let cursor = 0;
    const advance = () => {
      if (generation !== current) return;
      frame = undefined;
      const end = Math.min(cursor + 32, pending.length);
      while (cursor < end) retained.add(pending[cursor++]);
      setKeys(all.filter((key) => retained.has(key)));
      if (cursor < pending.length) frame = requestAnimationFrame(advance);
    };
    advance();
  });
  const pendingHeight = createMemo(() => {
    const mounted = new Set(keys());
    let rows = 0;
    for (const [key, group] of props.groups()) if (!mounted.has(key)) rows += 1 + group.blocks.length;
    return rows * 1.9;
  });
  return <>
    <For each={keys()}>{(key) => <QueryGroup group={() => props.groups().get(key)} flat={props.flat} />}</For>
    <Show when={pendingHeight() > 0}>
      <div class="query-pending-groups" aria-hidden="true" style={{ "min-height": `${pendingHeight()}em` }} />
    </Show>
  </>;
}

// Keep the keyed group shell and approximate scroll height. The header and
// live result subtree start together on first viewport approach, then persist.
export function QueryGroup(props: QueryGroupProps): JSX.Element {
  const [near, setNear] = createSignal(false);
  let element: HTMLDivElement | undefined;
  onMount(() => {
    if (!element) return;
    const node = element;
    observeNear(node, () => setNear(true));
    onCleanup(() => unobserveNear(node));
  });
  return (
    <div ref={element} class="query-group" classList={{ "query-group-flat": props.flat }}
      style={!near() ? { "min-height": `${(1 + (props.group()?.blocks.length ?? 0)) * 1.9}em` } : undefined}>
      <Show when={near()}><MountedQueryGroup {...props} /></Show>
    </div>
  );
}

// One page's query results, rendered as LIVE editable blocks. The result page
// is loaded into the shared working set on demand; each result is the same
// <Block> the main view uses (so editing a result edits the real block and
// saves to its page). Until the page is loaded, a read-only block stands in.
//
// Keyed by page name (outer <For>) and block uuid (inner <For>) so a reactive
// re-query that returns the same membership reuses the existing rows — it never
// re-mounts a block you're editing in a result and yanks the caret out.
function MountedQueryGroup(props: QueryGroupProps): JSX.Element {
  const kind = (): PageKind => props.group()?.kind ?? "page";
  const page = () => props.group()?.page ?? "";
  const target = () => ({ name: page(), pageKind: kind(), ...(props.group()?.path ? { path: props.group()!.path } : {}) });
  return (
    <Show when={props.group()}>
      {(g) => (
        <>
          <div
            class={props.flat ? "query-crumb" : "query-page"}
            onClick={(e) => {
              e.stopPropagation();
              const dest = internalLinkDest(e);
              if (dest === "sidebar") openPageInSidebar(target());
              else if (dest === "background") openPageTargetInNewTab(target());
              else if (dest === "pane") openRouteInOtherPane({ kind: "page", ...target() });
              else openPageTarget(target());
            }}
            onMouseDown={internalLinkMouseDown}
            onAuxClick={(e) => {
              e.stopPropagation();
              internalLinkAuxClick(e, () => openPageTargetInNewTab(target()));
            }}
            onContextMenu={(e) => {
              if (!shouldOpenTextContextMenu(e.target)) return;
              e.preventDefault();
              e.stopPropagation();
              openPageContextMenu(e.clientX, e.clientY, target());
            }}
          >
            {page()}
          </div>
          <LiveRefGroup page={page()} kind={kind()} path={g().path} blocks={g().blocks} surface="query" showBreadcrumb />
        </>
      )}
    </Show>
  );
}
