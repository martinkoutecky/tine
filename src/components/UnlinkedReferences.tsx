import { For, Show, createResource, createSignal, type JSX } from "solid-js";
import { backend } from "../backend";
import { openPage } from "../router";
import { ReferenceExcerptBlocks } from "./ReferenceEvidence";
import type { RefGroup } from "../types";

// "Unlinked References" — plain-text mentions of the page, collapsed by default.
export function UnlinkedReferences(props: { name: string }): JSX.Element {
  const [open, setOpen] = createSignal(false);
  const [collapsedGroups, setCollapsedGroups] = createSignal<Set<string>>(new Set());
  const [groups] = createResource(
    () => (open() ? props.name : null),
    (n) => (n ? backend().getUnlinkedRefs(n) : Promise.resolve([]))
  );
  const count = () => groups()?.reduce((a, g) => a + g.blocks.length, 0) ?? 0;
  const groupKey = (group: RefGroup) => `${group.kind}:${group.page}`;
  const groupCollapsed = (group: RefGroup) => collapsedGroups().has(groupKey(group));
  const setGroupCollapsed = (group: RefGroup, value: boolean) => {
    setCollapsedGroups((current) => {
      const next = new Set(current);
      if (value) next.add(groupKey(group));
      else next.delete(groupKey(group));
      return next;
    });
  };
  const setAllGroups = (value: boolean) => {
    setCollapsedGroups(value ? new Set<string>((groups() ?? []).map(groupKey)) : new Set<string>());
  };

  return (
    <div class="unlinked-references">
      <div class="references-header clickable" onClick={() => setOpen(!open())}>
        {open() ? "▾" : "▸"} Unlinked References
        <Show when={open() && groups()}>
          <span class="references-count">{count()}</span>
        </Show>
      </div>
      <Show when={open()}>
        <Show when={(groups()?.length ?? 0) > 1}>
          <div class="reference-bulk-controls" aria-label="Unlinked reference page groups">
            <button type="button" onClick={() => setAllGroups(true)}>Collapse all</button>
            <button type="button" onClick={() => setAllGroups(false)}>Expand all</button>
          </div>
        </Show>
        <For each={groups()}>
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
                  {groupCollapsed(g) ? "▸" : "▾"}
                </button>
                <button type="button" class="reference-page" onClick={() => openPage(g.page, g.kind)}>
                  {g.page}
                </button>
              </div>
              <Show when={!groupCollapsed(g)}>
                <div
                  class="reference-blocks"
                  data-inpage-find-surface={`unlinked:${props.name}:${g.kind}:${g.page}`}
                >
                  <ReferenceExcerptBlocks
                    blocks={g.blocks}
                    evidence={g.evidence ?? []}
                    page={g.page}
                    kind={g.kind}
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
