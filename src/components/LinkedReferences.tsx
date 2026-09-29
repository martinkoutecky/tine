import { For, Show, createResource, createSignal, createMemo, createEffect, onCleanup, type JSX } from "solid-js";
import { backend } from "../backend";
import { errorFamily } from "../errorFamily";
import { graphOwner, latestOwner, readOwned } from "../owned";
import { openPage, openPageInNewTab } from "../router";
import { openPageInSidebar, openPageContextMenu, searchRemoveAccents } from "../ui";
import { graphMeta } from "../graphSession";
import { LiveRefGroup } from "./LiveRefGroup";
import type { BacklinkFilterEntry, BacklinkFilterTarget, BlockDto, RefGroup } from "../types";
import { shouldOpenTextContextMenu } from "../contextMenuPolicy";
import { createLongPress } from "../render/longPress";
import { matcherMatches, parseSearchQuery } from "../editor/searchQuery";
import { searchFold } from "../editor/searchFold";
import { ReferenceExportChooser } from "./ReferenceExportChooser";
import { collapsedGroupsFor, sectionOverride, setCollapsedGroupsFor, setSectionOverride } from "../referenceSectionState";

const norm = (s: string) => s.trim().toLowerCase();
const pageIdentity = (s: string) => {
  const lowered = s.trim().toLowerCase();
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
  return errorFamily(error) === "result-too-large" ? "bounded" : "backend";
}

// Persist the per-page include/exclude reference filter so it survives reload.
type FilterMap = Record<string, "in" | "out">;
const RF_KEY = "logseq-claude.refFilters";
function loadFilters(page: string): FilterMap {
  try {
    return JSON.parse(localStorage.getItem(RF_KEY) ?? "{}")[page] ?? {};
  } catch {
    return {};
  }
}
function saveFilters(page: string, f: FilterMap) {
  try {
    const m = JSON.parse(localStorage.getItem(RF_KEY) ?? "{}");
    if (Object.keys(f).length) m[page] = f;
    else delete m[page];
    localStorage.setItem(RF_KEY, JSON.stringify(m));
  } catch {
    // ignore
  }
}
const filterKey = (page: string, kind: string, blockId: string) => `${kind}\0${norm(page)}\0${blockId}`;

type SearchableFilterEntry = Pick<BacklinkFilterEntry, "text" | "facets"> & {
  normalizedText: string;
};

function searchableFilterEntry(
  entry: Pick<BacklinkFilterEntry, "text" | "facets">
): SearchableFilterEntry {
  return {
    text: entry.text,
    facets: entry.facets,
    normalizedText: searchFold(entry.text, searchRemoveAccents()),
  };
}

/** A bounded fallback while native context is loading or stale. It intentionally
 *  uses only DTO-owned semantic facets (never a raw reference regex); the native
 *  context replaces it with parser-owned descendant refs as soon as it arrives. */
function fallbackFilterEntry(block: BlockDto): SearchableFilterEntry {
  const text: string[] = [];
  const facets = new Map<string, string>();
  const visit = (current: BlockDto) => {
    text.push(current.raw);
    for (const tag of current.tags ?? []) if (!facets.has(norm(tag))) facets.set(norm(tag), tag);
    if (current.marker) {
      const key = norm(current.marker);
      if (!facets.has(key)) facets.set(key, current.marker);
    }
    for (const child of current.children) visit(child);
  };
  visit(block);
  return searchableFilterEntry({ text: text.join("\n"), facets: [...facets.values()] });
}

// The "Linked References" section (backlinks). Live, editable, collapsible, and
// filterable by co-referenced page (click a chip: include → exclude → off),
// mirroring OG's reference filter.
// GH #479: the graph's `:ref/linked-references-collapsed-threshold` decides;
// 100 is OG's fallback when the key is absent. Zero is a real setting
// ("always collapsed"), so this never treats a falsy threshold as unset.
const OG_REFERENCE_COLLAPSE_THRESHOLD = 100;
const referenceCollapseThreshold = () =>
  graphMeta()?.linked_references_collapsed_threshold ?? OG_REFERENCE_COLLAPSE_THRESHOLD;

/** Show bounded backlinks for one page. Text filters and OR include / cumulative
 * exclude chips use the same source-root context; export snapshots visible rows.
 * One backend read per target; a fixed result-limit token selects the bounded
 * alert, other failures a generic alert. */
export function LinkedReferences(props: { name: string }): JSX.Element {
  const readScope = {};
  let alive = true;
  onCleanup(() => { alive = false; });
  const [loadError, setLoadError] = createSignal<ReferenceLoadError | null>(null);
  const [groups] = createResource(
    () => props.name,
    async (n) => {
      const owner = latestOwner(readScope, "backlinks", graphOwner(() => alive && props.name === n));
      setLoadError(null);
      try {
        const result = await readOwned(owner, backend().getBacklinks(n));
        return result.kind === "current" ? result.value : [];
      } catch (error) {
        if (owner()) setLoadError(classifyReferenceLoadError(error));
        return [];
      }
    }
  );
  const mergedGroups = createMemo(() => mergeReferenceGroups(groups() ?? []));
  const [collapsedOverride, setCollapsedOverrideSignal] = createSignal<boolean | null>(sectionOverride("linked", props.name) ?? null);
  const setCollapsedOverride = (value: boolean) => {
    setSectionOverride("linked", props.name, value);
    setCollapsedOverrideSignal(value);
  };
  const [collapsedGroups, setCollapsedGroupsSignal] = createSignal<Set<string>>(collapsedGroupsFor("linked", props.name));
  const setCollapsedGroups = (update: Set<string> | ((current: Set<string>) => Set<string>)) => {
    setCollapsedGroupsSignal((current) => {
      const next = typeof update === "function" ? update(current) : update;
      setCollapsedGroupsFor("linked", props.name, next);
      return next;
    });
  };
  const [filterOpen, setFilterOpen] = createSignal(false);
  const [exportChooserOpen, setExportChooserOpen] = createSignal(false);
  const [searchDraft, setSearchDraft] = createSignal("");
  const [searchQuery, setSearchQuery] = createSignal("");
  let searchTimer: ReturnType<typeof setTimeout> | undefined;
  onCleanup(() => {
    if (searchTimer !== undefined) clearTimeout(searchTimer);
  });
  // page name -> "in" (must also reference) | "out" (must not reference).
  const [filters, setFilters] = createSignal<FilterMap>(loadFilters(props.name));
  // Reload the saved filter when the page changes.
  createEffect(() => {
    const page = props.name;
    setCollapsedOverrideSignal(sectionOverride("linked", page) ?? null);
    setCollapsedGroupsSignal(collapsedGroupsFor("linked", page));
    setFilters(loadFilters(props.name));
    setFilterOpen(false);
    setSearchDraft("");
    setSearchQuery("");
  });

  const targets = createMemo<BacklinkFilterTarget[]>(() =>
    mergedGroups().flatMap((group) =>
      group.blocks.map((block) => ({ page: group.page, kind: group.kind, block_id: block.id }))
    )
  );
  const needsNativeContext = () => filterOpen() || Object.keys(filters()).length > 0;
  const [nativeContext] = createResource(
    () => {
      if (!needsNativeContext() || !groups()) return null;
      return { name: props.name, targets: targets() };
    },
    ({ name, targets }) => backend().getBacklinkFilterContext(name, targets)
  );
  const fallbackByRoot = createMemo(() =>
    new Map(
      mergedGroups().flatMap((group) =>
        group.blocks.map((block) => [
          filterKey(group.page, group.kind, block.id),
          fallbackFilterEntry(block),
        ] as const)
      )
    )
  );
  const nativeByRoot = createMemo(() =>
    new Map(
      (nativeContext()?.entries ?? []).map((entry) => [
        filterKey(entry.page, entry.kind, entry.block_id),
        searchableFilterEntry(entry),
      ] as const)
    )
  );
  const rootEntry = (group: RefGroup, block: BlockDto) =>
    nativeByRoot().get(filterKey(group.page, group.kind, block.id))
      ?? fallbackByRoot().get(filterKey(group.page, group.kind, block.id))!;

  const parsedSearch = createMemo(() => parseSearchQuery(searchQuery(), searchRemoveAccents()));
  const searchError = createMemo(() => {
    const parsed = parsedSearch();
    return parsed.kind === "invalid" ? parsed.error : null;
  });
  const filterGroups = (source: RefGroup[], keep: (group: RefGroup, block: BlockDto) => boolean): RefGroup[] =>
    source.map((group) => ({ ...group, blocks: group.blocks.filter((block) => keep(group, block)) }))
      .map((group) => {
        const ids = new Set(group.blocks.map((block) => block.id));
        return { ...group, evidence: group.evidence?.filter((item) => ids.has(item.block_id)) };
      }).filter((group) => group.blocks.length > 0);
  const textMatchedGroups = createMemo<RefGroup[]>(() => {
    const parsed = parsedSearch();
    if (nativeContext.loading || parsed.kind === "empty" || parsed.kind === "invalid") return mergedGroups();
    return filterGroups(mergedGroups(), (group, block) => {
      const entry = rootEntry(group, block);
      return matcherMatches(parsed, entry.normalizedText, entry.text);
    });
  });

  // Co-referenced pages/tags and task states in each backlink tree, with counts.
  const coRefs = createMemo(() => {
    const counts = new Map<string, { name: string; count: number }>();
    for (const g of textMatchedGroups()) {
      for (const b of g.blocks) {
        for (const name of rootEntry(g, b).facets) {
          const key = norm(name);
          const previous = counts.get(key);
          counts.set(key, { name: previous?.name ?? name, count: (previous?.count ?? 0) + 1 });
        }
      }
    }
    return [...counts.values()]
      .map(({ name, count }) => [name, count] as const)
      .sort((a, b) => b[1] - a[1] || a[0].localeCompare(b[0]));
  });

  const orphanFilters = createMemo(() => {
    const present = new Set(coRefs().map(([name]) => norm(name)));
    return Object.keys(filters()).filter((name) => !present.has(norm(name)));
  });
  const filterState = (name: string): "in" | "out" | undefined => {
    const key = norm(name);
    return Object.entries(filters()).find(([candidate]) => norm(candidate) === key)?.[1];
  };

  const shown = createMemo<RefGroup[]>(() => {
    const f = filters();
    const ins = Object.keys(f).filter((k) => f[k] === "in").map(norm);
    const outs = Object.keys(f).filter((k) => f[k] === "out").map(norm);
    const parsed = parsedSearch();
    const searching = parsed.kind !== "empty" && parsed.kind !== "invalid";
    // Do not flash descendant-only matches away while their on-demand native
    // index is still in flight. Once it arrives, filtering is synchronous.
    if ((searching || ins.length || outs.length) && nativeContext.loading) return mergedGroups();
    if (!ins.length && !outs.length) return textMatchedGroups();
    return filterGroups(textMatchedGroups(), (group, block) => {
      const facets = new Set(rootEntry(group, block).facets.map(norm));
      return (ins.length === 0 || ins.some((name) => facets.has(name)))
        && outs.every((name) => !facets.has(name));
    });
  });

  const groupKey = (group: RefGroup) => pageIdentity(group.page);
  const shownByKey = createMemo(() => new Map(shown().map((group) => [groupKey(group), group] as const)));
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
    setCollapsedGroups(value ? new Set<string>(shown().map(groupKey)) : new Set<string>());
  };

  const cycle = (name: string) => {
    const key = norm(name);
    const f = Object.fromEntries(Object.entries(filters()).filter(([candidate]) => norm(candidate) !== key)) as FilterMap;
    const current = filterState(name);
    if (current === "in") f[name] = "out";
    else if (current !== "out") f[name] = "in";
    setFilters(f);
    saveFilters(props.name, f);
  };
  const count = () => shown().reduce((acc, g) => acc + g.blocks.length, 0);
  const totalCount = () => mergedGroups().reduce((acc, g) => acc + g.blocks.length, 0);
  const collapsed = () => collapsedOverride() ?? totalCount() >= referenceCollapseThreshold();
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
  const hasActiveFilter = () => searchDraft().trim() !== "" || Object.keys(filters()).length > 0;
  const updateSearch = (value: string) => {
    setSearchDraft(value);
    if (searchTimer !== undefined) clearTimeout(searchTimer);
    searchTimer = setTimeout(() => setSearchQuery(value), 120);
  };
  const clearAllFilters = () => {
    if (searchTimer !== undefined) clearTimeout(searchTimer);
    setSearchDraft("");
    setSearchQuery("");
    setFilters({});
    saveFilters(props.name, {});
  };

  return (
    <Show
      when={loadError() === null}
      fallback={
        <div class="linked-references reference-error" role="alert">
          <div class="references-header">Linked References</div>
          <div class="reference-filter-error">
            {loadError() === "bounded"
              ? "Couldn’t load references: the bounded result limit was exceeded."
              : "Couldn’t load references because the backend request failed."}
          </div>
        </div>
      }
    >
    <Show when={groups() && mergedGroups().length > 0}>
      <Show when={exportChooserOpen()}>
        <ReferenceExportChooser subject="Linked References" groups={shown()} onClose={() => setExportChooserOpen(false)} />
      </Show>
      <div class="linked-references">
        <div class="references-header" onClick={() => setCollapsedOverride(!collapsed())}>
          <span class="ref-collapse" classList={{ collapsed: collapsed() }}>
            <svg viewBox="0 0 24 24" class="triangle">
              <path d="M8 5l8 7-8 7z" />
            </svg>
          </span>
          Linked References <span class="references-count">{count()}</span>
          <button
            type="button"
            class="reference-filter-toggle"
            classList={{ active: filterOpen() || hasActiveFilter() }}
            aria-label="Filter linked references"
            aria-expanded={filterOpen()}
            title="Filter linked references"
            onClick={(event) => {
              event.stopPropagation();
              setFilterOpen(!filterOpen());
            }}
          >
            <svg viewBox="0 0 24 24" aria-hidden="true"><path d="M4 5h16l-6.2 7.1v5.4l-3.6 1.8v-7.2z" /></svg>
          </button>
          <button type="button" class="reference-export-toggle"
            aria-label="Copy / export linked references" title="Copy / export selected linked references"
            onClick={(event) => { event.stopPropagation(); setExportChooserOpen(true); }}
          ><svg viewBox="0 0 24 24" aria-hidden="true"><path d="M16 1H4a2 2 0 0 0-2 2v14h2V3h12V1zm3 4H8a2 2 0 0 0-2 2v14a2 2 0 0 0 2 2h11a2 2 0 0 0 2-2V7a2 2 0 0 0-2-2zm0 16H8V7h11v14z" /></svg></button>
        </div>
        <Show when={!collapsed()}>
          <Show when={occurrenceLimit().truncated}>
            <div class="reference-truncation" role="status">
              Showing {occurrenceLimit().shown} of {occurrenceLimit().total} matching occurrences.
            </div>
          </Show>
          <Show when={filterOpen()}>
            <div class="reference-filter-panel">
              <div class="reference-filter-search-row">
                <input
                  class="reference-filter-search"
                  type="search"
                  value={searchDraft()}
                  placeholder="Search reference text"
                  aria-label="Search linked reference text"
                  onInput={(event) => updateSearch(event.currentTarget.value)}
                  onKeyDown={(event) => {
                    if (event.key === "Escape") setFilterOpen(false);
                  }}
                />
                <button type="button" class="reference-filter-clear" disabled={!hasActiveFilter()} onClick={clearAllFilters}>
                  Clear
                </button>
              </div>
              <div class="reference-filter-summary">
                {count()} of {totalCount()} references
                <Show when={nativeContext.loading}> · indexing…</Show>
              </div>
              <Show when={searchError()}>
                {(error) => <div class="reference-filter-error">Invalid search: {error()}</div>}
              </Show>
              <Show when={nativeContext.error}>
                <div class="reference-filter-error">Couldn’t index descendant text; searching visible root text only.</div>
              </Show>
              <Show when={nativeContext()?.truncated || nativeContext()?.entries.some((entry) => entry.truncated)}>
                <div class="reference-filter-warning">Some very large reference trees are searched partially.</div>
              </Show>
              <Show when={coRefs().length > 0 || orphanFilters().length > 0}>
                <div class="ref-filter" aria-label="Reference facets">
                  <For each={coRefs()}>
                    {([name, n]) => (
                      <button
                        class="ref-filter-chip"
                        classList={{ "f-in": filterState(name) === "in", "f-out": filterState(name) === "out" }}
                        title="Click: include · again: exclude · again: clear"
                        onClick={() => cycle(name)}
                      >
                        {name} <span class="ref-filter-count">{n}</span>
                      </button>
                    )}
                  </For>
                  <For each={orphanFilters()}>
                    {(name) => (
                      <button class="ref-filter-chip"
                        classList={{ "f-in": filterState(name) === "in", "f-out": filterState(name) === "out" }}
                        title="No match in the current text search · click to cycle or clear"
                        onClick={() => cycle(name)}
                      >{name} <span class="ref-filter-count">0</span></button>
                    )}
                  </For>
                </div>
              </Show>
            </div>
          </Show>
          <Show when={shown().length > 1}>
            <div class="reference-bulk-controls" aria-label="Linked reference page groups">
              <button type="button" onClick={() => setAllGroups(true)}>Collapse all</button>
              <button type="button" onClick={() => setAllGroups(false)}>Expand all</button>
            </div>
          </Show>
          <For each={shown().map(groupKey)}>
            {(key) => {
              const group = () => shownByKey().get(key)!;
              let pageButton: HTMLButtonElement | undefined;
              const longPress = createLongPress(() => pageButton);
              onCleanup(longPress.dispose);
              return (
              <div class="reference-group">
                <div class="reference-group-header">
                  <button
                    type="button"
                    class="reference-group-disclosure"
                    aria-expanded={!groupCollapsed(group())}
                    aria-label={`${groupCollapsed(group()) ? "Expand" : "Collapse"} references from ${group().page}`}
                    onClick={() => setGroupCollapsed(group(), !groupCollapsed(group()))}
                  >
                    {groupCollapsed(group()) ? "▸" : "▾"}
                  </button>
                  <button
                    ref={pageButton}
                    type="button"
                    class="reference-page"
                    onClick={(e) => {
                      if (longPress.consumeClick(e)) {
                        e.preventDefault();
                        e.stopPropagation();
                        return;
                      }
                      if (e.shiftKey) openPageInSidebar(group().page, group().kind);
                      else openPage(group().page, group().kind);
                    }}
                    onAuxClick={(e) => {
                      if (e.button === 1) {
                        e.preventDefault();
                        openPageInNewTab(group().page, group().kind);
                      }
                    }}
                    onPointerDown={longPress.onPointerDown}
                    onPointerMove={longPress.onPointerMove}
                    onPointerUp={longPress.onPointerUp}
                    onPointerCancel={longPress.onPointerCancel}
                    onContextMenu={(e) => {
                      if (!shouldOpenTextContextMenu(e)) return;
                      e.preventDefault();
                      openPageContextMenu(e.clientX, e.clientY, group().page, group().kind);
                    }}
                  >
                    {group().page}
                  </button>
                </div>
                <Show when={!groupCollapsed(group())}>
                  <div
                    class="reference-blocks"
                    data-inpage-find-surface={`linked:${props.name}:${group().kind}:${group().page}`}
                  >
                    <LiveRefGroup
                      page={group().page}
                      kind={group().kind}
                      blocks={group().blocks}
                      evidence={group().evidence}
                      surface="ref"
                      showBreadcrumb
                    />
                  </div>
                </Show>
              </div>
              );
            }}
          </For>
        </Show>
      </div>
    </Show>
    </Show>
  );
}
