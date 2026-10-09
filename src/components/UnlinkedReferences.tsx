import { createReferenceGroupCollapse } from "../referenceGroupCollapse";
import { For, Show, createEffect, createMemo, createResource, createSignal, onCleanup, type JSX } from "solid-js";
import { backend } from "../backend";
import { classifyReferenceLoadError, referenceLoadErrorMessage, type ReferenceLoadError } from "../referenceLoadError";
import { graphOwner, latestOwner, readOwned } from "../owned";
import { openPage, openPageInNewTab } from "../router";
import { openRouteInOtherPane } from "../panes";
import { internalLinkAuxClick, internalLinkDest, internalLinkMouseDown } from "../linkGesture";
import { openPageContextMenu, openPageInSidebar } from "../ui";
import { shouldOpenTextContextMenu } from "../contextMenuPolicy";
import { LiveRefGroup } from "./LiveRefGroup";
import { ReferenceExportChooser } from "./ReferenceExportChooser";
import type { RefGroup } from "../types";
import { mergeReferenceGroups } from "../referenceGroups";
import { sectionOverride, setSectionOverride } from "../referenceSectionState";
import { readOr } from "../resourceRead";
import { ReferenceDisclosure } from "./ReferenceDisclosure";

type BoundedEvidence = NonNullable<RefGroup["evidence"]>[number] & {
  total?: number;
  truncated?: boolean;
};

// "Unlinked References" — plain-text mentions of the page, collapsed by default.
/** Show bounded plain-text mentions for one page. Expansion survives remounts
 * within the graph session, and batch export snapshots the current results.
 * One backend read per target; only the fixed result-limit token selects the
 * bounded failure alert. */
export function UnlinkedReferences(props: { name: string }): JSX.Element {
  const readScope = {};
  let alive = true;
  onCleanup(() => { alive = false; });
  const [open, setOpenSignal] = createSignal(sectionOverride("unlinked", props.name) ?? false);
  const setOpen = (value: boolean) => {
    setSectionOverride("unlinked", props.name, value);
    setOpenSignal(value);
  };
  const [loadError, setLoadError] = createSignal<ReferenceLoadError | null>(null);
  const [exportChooserOpen, setExportChooserOpen] = createSignal(false);
  const { groupCollapsed, setGroupCollapsed, setAll, reload: reloadGroupCollapse } = createReferenceGroupCollapse("unlinked", () => props.name);
  createEffect(() => {
    const page = props.name;
    reloadGroupCollapse();
    setOpenSignal(sectionOverride("unlinked", page) ?? false);
  });
  const [groupsResource] = createResource(
    () => props.name,
    async (n) => {
      const owner = latestOwner(readScope, "unlinked", graphOwner(() => alive && props.name === n));
      setLoadError(null);
      try {
        const result = await readOwned(owner, backend().getUnlinkedRefs(n));
        return result.kind === "current" ? result.value : [];
      } catch (error) {
        if (owner()) setLoadError(classifyReferenceLoadError(error));
        return [];
      }
    }
  );
  // The resource loader reports failures through loadError; readOr covers reads.
  const groups = () => readOr(groupsResource, undefined, "unlinked references");
  const mergedGroups = createMemo(() => mergeReferenceGroups(groups() ?? []));
  const count = () => mergedGroups().reduce((a, g) => a + g.blocks.length, 0);
  const setAllGroups = (value: boolean) => setAll(mergedGroups(), value);
  const occurrenceLimit = createMemo(() => {
    let shown = 0;
    let total = 0;
    for (const group of mergedGroups()) {
      for (const evidence of (group.evidence ?? []) as BoundedEvidence[]) {
        shown += evidence.occurrences.length;
        total += evidence.total ?? evidence.occurrences.length;
      }
    }
    return { shown, total, truncated: total > shown };
  });

  return (
    <div class="unlinked-references">
      <div class="references-header clickable" onClick={() => setOpen(!open())}>
        <ReferenceDisclosure collapsed={!open()} /> Unlinked References
        <Show when={groups()}>
          <span class="references-count">{count()}</span>
        </Show>
        <Show when={groupsResource.loading}><span class="references-loading"> Loading…</span></Show>
        <button type="button" class="reference-export-toggle"
          aria-label="Copy / export unlinked references" title="Copy / export selected unlinked references"
          disabled={!count()}
          onClick={(event) => { event.stopPropagation(); setExportChooserOpen(true); }}
        ><svg viewBox="0 0 24 24" aria-hidden="true"><path d="M16 1H4a2 2 0 0 0-2 2v14h2V3h12V1zm3 4H8a2 2 0 0 0-2 2v14a2 2 0 0 0 2 2h11a2 2 0 0 0 2-2V7a2 2 0 0 0-2-2zm0 16H8V7h11v14z" /></svg></button>
      </div>
      <Show when={exportChooserOpen()}>
        <ReferenceExportChooser subject="Unlinked References" groups={mergedGroups()} onClose={() => setExportChooserOpen(false)} />
      </Show>
      <Show when={open()}>
        <Show when={loadError()}>
          <div class="reference-filter-error reference-error" role="alert">
            {referenceLoadErrorMessage(loadError()!)}
          </div>
        </Show>
        <Show when={occurrenceLimit().truncated}>
          <div class="reference-truncation" role="status">
            Showing {occurrenceLimit().shown} of {occurrenceLimit().total} matching occurrences.
          </div>
        </Show>
        <Show when={mergedGroups().length > 1}>
          <div class="reference-bulk-controls" aria-label="Unlinked reference page groups">
            <button type="button" onClick={() => setAllGroups(true)}>Collapse all</button>
            <button type="button" onClick={() => setAllGroups(false)}>Expand all</button>
          </div>
        </Show>
        <For each={mergedGroups()}>
          {(g) => (
            <div class="reference-group">
              <div class="reference-group-header">
                <button
                  type="button"
                  class="reference-group-disclosure"
                  aria-expanded={!groupCollapsed(g)}
                  aria-label={`${groupCollapsed(g) ? "Expand" : "Collapse"} references from ${g.page}`}
                  onClick={() => setGroupCollapsed(g, !groupCollapsed(g))}
                >
                  <ReferenceDisclosure collapsed={groupCollapsed(g)} />
                </button>
                <button
                  type="button"
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
                </button>
              </div>
              <Show when={!groupCollapsed(g)}>
                <div
                  class="reference-blocks"
                  data-inpage-find-surface={`unlinked:${props.name}:${g.kind}:${g.page}`}
                >
                  <LiveRefGroup
                    blocks={g.blocks}
                    evidence={g.evidence ?? []}
                    page={g.page}
                    kind={g.kind}
                    path={g.path}
                    surface="ref"
                    showBreadcrumb
                    excerpt
                  />
                </div>
              </Show>
            </div>
          )}
        </For>
      </Show>
    </div>
  );
}
