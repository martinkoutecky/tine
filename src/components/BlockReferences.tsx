import { For, Show, createResource, createSignal, type JSX } from "solid-js";
import { backend } from "../backend";
import { dataRev, graphEpoch } from "../graphSession";
import { openPage, openPageInNewTab } from "../router";
import { openRouteInOtherPane } from "../panes";
import { internalLinkAuxClick, internalLinkDest, internalLinkMouseDown } from "../linkGesture";
import { openPageInSidebar, openPageContextMenu } from "../ui";
import { LiveRefGroup } from "./LiveRefGroup";
import { shouldOpenTextContextMenu } from "../contextMenuPolicy";
import { blockExternalId } from "../document";
import { readOr } from "../resourceRead";
import { ResourceFailure } from "./ResourceFailure";

// Block-level "linked references": the blocks that reference THIS block (via
// `((uuid))` / `[..](((uuid)))` / `{{embed ((uuid))}}`), grouped by page. Toggled
// open by the per-block reference-count badge (Block.tsx). Mirrors the page-level
// LinkedReferences, minus the co-reference filter chips (OG doesn't show those on
// the block-ref panel). Refetches when the graph generation changes.
/** Show the source locations of one block's references. Reads the bounded
 * referrer answer on graph revision changes; disclosure is local to this panel
 * and can be changed for one group or all groups without writing the graph. */
export function BlockReferences(props: { id: string }): JSX.Element {
  const [groupsResource, { refetch }] = createResource(
    () => ({ id: blockExternalId(props.id) ?? props.id, epoch: graphEpoch(), revision: dataRev() }),
    ({ id }) => backend().getBlockReferrers(id)
  );
  // Unlike the page-level panels this one has no fetcher wrapper, so it owns
  // both halves: readOr keeps a failed read out of the page's render, and the
  // row below keeps the panel from silently claiming there are no references.
  const groups = () => readOr(groupsResource, undefined, "block references");
  const count = () => (groups() ?? []).reduce((acc, g) => acc + g.blocks.length, 0);
  const [collapsedGroups, setCollapsedGroups] = createSignal<Set<string>>(new Set());
  const groupKey = (group: { page: string; kind: string; path?: string }) =>
    `${group.kind}\0${group.path ?? ""}\0${group.page}`;
  const groupCollapsed = (group: { page: string; kind: string; path?: string }) =>
    collapsedGroups().has(groupKey(group));
  const setGroupCollapsed = (group: { page: string; kind: string; path?: string }, value: boolean) => {
    setCollapsedGroups((current) => {
      const next = new Set(current);
      if (value) next.add(groupKey(group)); else next.delete(groupKey(group));
      return next;
    });
  };
  const setAllGroups = (value: boolean) =>
    setCollapsedGroups(value ? new Set((groups() ?? []).map(groupKey)) : new Set<string>());

  return (
    <>
      <ResourceFailure of={groupsResource} what="references to this block" onRetry={() => void refetch()} />
    <Show when={groups() && groups()!.length > 0}>
      <div class="block-references-inner">
        <div class="block-references-header">
          {count()} Linked Reference{count() === 1 ? "" : "s"}
        </div>
        <Show when={(groups() ?? []).length > 1}>
          <div class="reference-bulk-controls" aria-label="Reference page groups">
            <button type="button" onClick={() => setAllGroups(true)}>Collapse all</button>
            <button type="button" onClick={() => setAllGroups(false)}>Expand all</button>
          </div>
        </Show>
        <For each={groups()}>
          {(g) => (
            <div class="reference-group">
              <div class="reference-group-header">
              <button type="button" class="reference-group-disclosure"
                aria-expanded={!groupCollapsed(g)}
                aria-label={`${groupCollapsed(g) ? "Expand" : "Collapse"} references from ${g.page}`}
                onClick={() => setGroupCollapsed(g, !groupCollapsed(g))}
              >{groupCollapsed(g) ? "▸" : "▾"}</button>
              <div
                class="reference-page"
                onMouseDown={internalLinkMouseDown}
                onClick={(e) => {
                  const dest = internalLinkDest(e);
                  if (dest === "sidebar") openPageInSidebar(g.page, g.kind);
                  else if (dest === "background") openPageInNewTab(g.page, g.kind);
                  else if (dest === "pane") openRouteInOtherPane({ kind: "page", name: g.page, pageKind: g.kind });
                  else openPage(g.page, g.kind);
                }}
                onAuxClick={(e) => internalLinkAuxClick(e, () => openPageInNewTab(g.page, g.kind))}
                onContextMenu={(e) => {
                  if (!shouldOpenTextContextMenu(e.target)) return;
                  e.preventDefault();
                  openPageContextMenu(e.clientX, e.clientY, g.page, g.kind);
                }}
              >
                {g.page}
              </div>
              </div>
              <Show when={!groupCollapsed(g)}>
              <div class="reference-blocks">
                {/* OG shows each referrer's ancestor breadcrumb in the block-ref
                    panel (:breadcrumb-show? true) for "where does this live" context. */}
                <LiveRefGroup page={g.page} kind={g.kind} blocks={g.blocks} surface="ref" showBreadcrumb />
              </div>
              </Show>
            </div>
          )}
        </For>
      </div>
    </Show>
    </>
  );
}
