import { For, Show, createMemo, createResource, createSignal, type JSX } from "solid-js";
import { backend } from "../backend";
import { openPage } from "../router";
import { ReferenceExcerptBlocks } from "./ReferenceEvidence";
import type { RefGroup } from "../types";

const pageIdentity = (name: string) => {
  const lowered = name.trim().toLowerCase();
  const withoutLeading = lowered.startsWith("/") ? lowered.slice(1) : lowered;
  const withoutBoundaries = withoutLeading.endsWith("/") ? withoutLeading.slice(0, -1) : withoutLeading;
  return withoutBoundaries.normalize("NFC");
};

type BoundedEvidence = NonNullable<RefGroup["evidence"]>[number] & {
  total?: number;
  truncated?: boolean;
};

function mergeReferenceGroups(groups: RefGroup[]): RefGroup[] {
  const merged = new Map<string, RefGroup>();
  for (const group of groups) {
    const key = pageIdentity(group.page);
    const existing = merged.get(key);
    if (existing) {
      existing.blocks.push(...group.blocks);
      existing.evidence = [...(existing.evidence ?? []), ...(group.evidence ?? [])];
    } else {
      merged.set(key, { ...group, blocks: [...group.blocks], evidence: [...(group.evidence ?? [])] });
    }
  }
  return [...merged.values()];
}

type ReferenceLoadError = "bounded" | "backend";

function classifyReferenceLoadError(error: unknown): ReferenceLoadError {
  const message = error instanceof Error ? error.message : String(error);
  return message.startsWith("result-too-large:") ? "bounded" : "backend";
}

// "Unlinked References" — plain-text mentions of the page, collapsed by default.
export function UnlinkedReferences(props: { name: string }): JSX.Element {
  const [open, setOpen] = createSignal(false);
  const [loadError, setLoadError] = createSignal<ReferenceLoadError | null>(null);
  const [collapsedGroups, setCollapsedGroups] = createSignal<Set<string>>(new Set());
  const [groups] = createResource(
    () => props.name,
    async (n) => {
      setLoadError(null);
      try {
        return await backend().getUnlinkedRefs(n);
      } catch (error) {
        setLoadError(classifyReferenceLoadError(error));
        return [];
      }
    }
  );
  const mergedGroups = createMemo(() => mergeReferenceGroups(groups() ?? []));
  const count = () => mergedGroups().reduce((a, g) => a + g.blocks.length, 0);
  const groupKey = (group: RefGroup) => pageIdentity(group.page);
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
    setCollapsedGroups(value ? new Set<string>(mergedGroups().map(groupKey)) : new Set<string>());
  };
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
        {open() ? "▾" : "▸"} Unlinked References
        <Show when={groups()}>
          <span class="references-count">{count()}</span>
        </Show>
        <Show when={groups.loading}><span class="references-loading"> Loading…</span></Show>
      </div>
      <Show when={open()}>
        <Show when={loadError()}>
          <div class="reference-filter-error reference-error" role="alert">
            {loadError() === "bounded"
              ? "Couldn’t load references: the bounded result limit was exceeded."
              : "Couldn’t load references because the backend request failed."}
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
                    path={g.path}
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
