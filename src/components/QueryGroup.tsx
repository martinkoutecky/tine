import { Show, createSignal, onMount, onCleanup, type JSX } from "solid-js";
import type { RefGroup, PageKind } from "../types";
import { openPageTarget, openPageTargetInNewTab } from "../router";
import { openPageInSidebar, openPageContextMenu } from "../ui";
import { openRouteInOtherPane } from "../panes";
import { internalLinkAuxClick, internalLinkDest, internalLinkMouseDown } from "../linkGesture";
import { shouldOpenTextContextMenu } from "../contextMenuPolicy";
import { observeNear, unobserveNear } from "../lazyObserve";
import { LiveRefGroup } from "./LiveRefGroup";

interface QueryGroupProps { group: () => RefGroup | undefined; flat?: boolean }

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

