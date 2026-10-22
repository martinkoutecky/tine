import { createReferenceGroupCollapse } from "../referenceGroupCollapse";
import { bulkCollapseTarget } from "../collapseToggle";
import { t } from "../i18n";
import { For, Show, createResource, createSignal, createMemo, createEffect, onCleanup, type JSX } from "solid-js";
import { backend } from "../backend";
import { classifyReferenceLoadError, referenceLoadErrorMessage, type ReferenceLoadError } from "../referenceLoadError";
import { graphOwner, latestOwner, readOwned } from "../owned";
import { openPage, openPageInNewTab } from "../router";
import { openRouteInOtherPane } from "../panes";
import { internalLinkAuxClick, internalLinkDest, internalLinkMouseDown } from "../linkGesture";
import { openPageInSidebar, openPageContextMenu, searchRemoveAccents } from "../ui";
import { graphMeta } from "../graphSession";
import { LiveRefGroup } from "./LiveRefGroup";
import type { BacklinkFilterEntry, BacklinkFilterTarget, BlockDto, RefGroup } from "../types";
import { shouldOpenTextContextMenu } from "../contextMenuPolicy";
import { createLongPress } from "../render/longPress";
import { matcherMatches, parseSearchQuery } from "../editor/searchQuery";
import { searchFold } from "../editor/searchFold";
import { ReferenceExportChooser } from "./ReferenceExportChooser";
import { pageIdentityKey } from "../pageIdentity";
import { mergeReferenceGroups } from "../referenceGroups";
import { sectionOverride, setSectionOverride } from "../referenceSectionState";
import { readOr } from "../resourceRead";
import { ReferenceDisclosure } from "./ReferenceDisclosure";
import { createMembershipResource, groupsContainEdit } from "../resultMembership";
import { activeSurface } from "../editorController";
import { navigationName } from "../pageIndex";
import { readPageProperty, setPageProperty } from "../document";
import { parseReferenceFilters, writeReferenceFilters } from "../referenceFilters";

// One identity fold for chips, filters and group merging: the old private `norm`
// (trim + toLowerCase) split NFC/NFD and boundary-slash spellings of one page
// into separate chips whose filters missed each other, and disagreed with the
// backend, which keys the same request with refs::normalize.
const norm = (s: string) => pageIdentityKey(s);

type BoundedEvidence = NonNullable<RefGroup["evidence"]>[number] & {
  total?: number;
  truncated?: boolean;
};

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
export function LinkedReferences(props: { name: string; journalFeed?: boolean }): JSX.Element {
  return <ReferenceSection name={props.name} journalFeed={props.journalFeed} />;
}

/** Shared filter and rendering owner for linked and plain-text references. */
export function ReferenceSection(props: { name: string; unlinked?: boolean; journalFeed?: boolean }): JSX.Element {
  const section = props.unlinked ? "unlinked" : "linked";
  const heading = () => t(props.unlinked ? "references-unlinked" : "references-linked");
  const savedFilterKey = () => props.unlinked ? `unlinked\0${props.name}` : props.name;
  const readScope = {};
  let alive = true;
  onCleanup(() => { alive = false; });
  const [loadError, setLoadError] = createSignal<ReferenceLoadError | null>(null);
  const [groupsResource] = createMembershipResource(
    () => props.name,
    () => props.name,
    async (n) => {
      const owner = latestOwner(readScope, "backlinks", graphOwner(() => alive && props.name === n));
      setLoadError(null);
      try {
        const result = await readOwned(owner, props.unlinked ? backend().getUnlinkedRefs(n) : backend().getBacklinks(n));
        return result.kind === "current" ? result.value : [];
      } catch (error) {
        if (owner()) setLoadError(classifyReferenceLoadError(error));
        return [];
      }
    },
    // Editing the source in the feed cannot unmount that editor when these
    // backlinks change. Only an editor inside the result surface needs its
    // answer retained until blur; source edits may follow published revisions.
    (groups, id) => (!props.journalFeed || !!activeSurface()?.startsWith("ref:")) && groupsContainEdit(groups, id),
  );
  // The resource loader reports failures through loadError; readOr covers reads.
  const groups = () => readOr(groupsResource, undefined, "linked references");
  const mergedGroups = createMemo(() => mergeReferenceGroups(groups() ?? []));
  // Unlinked's saved override is historically "open", linked's is "collapsed".
  const savedCollapse = () => props.unlinked ? !(sectionOverride(section, props.name) ?? false) : sectionOverride(section, props.name) ?? null;
  const [collapsedOverride, setCollapsedOverrideSignal] = createSignal<boolean | null>(savedCollapse());
  const setCollapsedOverride = (value: boolean) => {
    setSectionOverride(section, props.name, props.unlinked ? !value : value);
    setCollapsedOverrideSignal(value);
  };
  const { groupCollapsed, setGroupCollapsed, setAll, reload: reloadGroupCollapse } = createReferenceGroupCollapse(section, () => props.name);
  const [filterOpen, setFilterOpen] = createSignal(false);
  const [exportChooserOpen, setExportChooserOpen] = createSignal(false);
  const [searchDraft, setSearchDraft] = createSignal("");
  const [searchQuery, setSearchQuery] = createSignal("");
  let searchTimer: ReturnType<typeof setTimeout> | undefined;
  onCleanup(() => {
    if (searchTimer !== undefined) clearTimeout(searchTimer);
  });
  // page name -> "in" (must also reference) | "out" (must not reference).
  const [rawFilters, setFilters] = createSignal<FilterMap>(loadFilters(savedFilterKey()));
  // The existing inventory owns alias precedence and live updates. Resolve at
  // the shared identity boundary: chips, membership and legacy saved names
  // must all ask the same answerer, never scan the graph or build an alias map.
  const filters = createMemo<FilterMap>(() => Object.fromEntries(
    Object.entries(rawFilters()).map(([name, state]) => [navigationName(name), state])
  ));
  createEffect(() => {
    const property = props.unlinked ? null : readPageProperty(props.name, "filters");
    setFilters(property === null ? loadFilters(savedFilterKey()) : parseReferenceFilters(property) ?? {});
  });
  const persistFilters = (f: FilterMap) => {
    saveFilters(savedFilterKey(), f);
    // Unlinked has a separate device filter; OG's page property belongs only
    // to Linked References. All graph writes use the guarded document intent.
    if (!props.unlinked) setPageProperty(props.name, "filters", writeReferenceFilters(f));
  };
  // Reload the saved filter when the page changes.
  createEffect(() => {
    reloadGroupCollapse();
    setCollapsedOverrideSignal(savedCollapse());
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
  const [nativeContextResource] = createResource(
    () => {
      if (!needsNativeContext() || !groups()) return null;
      return { name: props.name, targets: targets() };
    },
    ({ name, targets }) => backend().getBacklinkFilterContext(name, targets)
  );
  // The "Couldn't index descendant text" row below was unreachable: reading a
  // rejected `nativeContext` threw before any Show could render it.
  const nativeContext = () => readOr(nativeContextResource, undefined, "reference filter index");
  const parsedSearch = createMemo(() => parseSearchQuery(searchQuery(), searchRemoveAccents()));
  // GH #746: the filter index (folded search text, page facets) costs a wasm
  // fold per reference root and is built only while a filter or search needs
  // it. Memos run eagerly, so an ungated index was rebuilt over every result
  // (13 MB of unlinked references: 0.7 s) after each save, collapsed or not.
  const indexNeeded = () => filterOpen() || Object.keys(filters()).length > 0 || parsedSearch().kind !== "empty";
  const fallbackByRoot = createMemo(() => !indexNeeded() ? new Map<string, SearchableFilterEntry>() :
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
  const rootFacets = (group: RefGroup, block: BlockDto) => new Map(
    rootEntry(group, block).facets.map((name) => {
      const canonical = navigationName(name);
      return [norm(canonical), canonical] as const;
    })
  );

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
    if (nativeContextResource.loading || parsed.kind === "empty" || parsed.kind === "invalid") return mergedGroups();
    return filterGroups(mergedGroups(), (group, block) => {
      const entry = rootEntry(group, block);
      return matcherMatches(parsed, entry.normalizedText, entry.text);
    });
  });

  // Co-referenced pages/tags and task states in each backlink tree, with counts.
  const coRefs = createMemo(() => {
    const counts = new Map<string, { name: string; count: number }>();
    // Only the open filter panel shows the chips.
    if (!filterOpen()) return [];
    for (const g of textMatchedGroups()) {
      for (const b of g.blocks) {
        // Count reference roots, not spellings: two aliases (or an alias and
        // the canonical name) in one tree contribute once to this page chip.
        for (const [key, name] of rootFacets(g, b)) {
          const previous = counts.get(key);
          counts.set(key, { name: previous?.name ?? name, count: (previous?.count ?? 0) + 1 });
        }
      }
    }
    return [...counts.values()]
      .map(({ name, count }) => [name, count] as const)
      // GH #433: alphabetical (case-folded, locale-aware), so chips stay put as
      // counts change while typing or filtering. OG sorts by count.
      .sort((a, b) => a[0].localeCompare(b[0], undefined, { sensitivity: "base", numeric: true }) || (a[0] < b[0] ? -1 : a[0] > b[0] ? 1 : 0));
  });

  const orphanFilters = createMemo(() => {
    const present = new Set(coRefs().map(([name]) => norm(name)));
    return Object.keys(filters()).filter((name) => !present.has(norm(name)));
  });
  const filterState = (name: string): "in" | "out" | undefined => {
    const key = norm(name);
    return Object.entries(filters()).find(([candidate]) => norm(candidate) === key)?.[1];
  };

  // A filter is asked for but the descendant index it needs is still in flight,
  // so the list is deliberately UNFILTERED (descendant-only matches must not
  // flash away) and the summary says so instead of reporting "N of N references".
  // Once the index arrives, filtering is synchronous.
  const filterPending = () => {
    if (!nativeContextResource.loading) return false;
    const parsed = parsedSearch();
    return (parsed.kind !== "empty" && parsed.kind !== "invalid") || Object.keys(filters()).length > 0;
  };
  const shown = createMemo<RefGroup[]>(() => {
    const f = filters();
    const ins = Object.keys(f).filter((k) => f[k] === "in").map(norm);
    const outs = Object.keys(f).filter((k) => f[k] === "out").map(norm);
    if (filterPending()) return mergedGroups();
    if (!ins.length && !outs.length) return textMatchedGroups();
    return filterGroups(textMatchedGroups(), (group, block) => {
      const facets = rootFacets(group, block);
      return (ins.length === 0 || ins.every((name) => facets.has(name)))
        && outs.every((name) => !facets.has(name));
    });
  });

  const groupKey = (group: RefGroup) => pageIdentityKey(group.page);
  const shownByKey = createMemo(() => new Map(shown().map((group) => [groupKey(group), group] as const)));
  const setAllGroups = (value: boolean) => setAll(shown(), value);

  const cycle = (name: string) => {
    const key = norm(name);
    const f = Object.fromEntries(Object.entries(filters()).filter(([candidate]) => norm(candidate) !== key)) as FilterMap;
    const current = filterState(name);
    if (current === "in") f[name] = "out";
    else if (current !== "out") f[name] = "in";
    setFilters(f);
    persistFilters(f);
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
    persistFilters({});
  };

  return (
    <Show
      when={props.unlinked || loadError() === null}
      fallback={
        <div class="linked-references reference-error" role="alert">
          <div class="references-header">{heading()}</div>
          <div class="reference-filter-error">
            {referenceLoadErrorMessage(loadError()!)}
          </div>
        </div>
      }
    >
    <Show when={props.unlinked || (groups() && mergedGroups().length > 0)}>
      <Show when={exportChooserOpen()}>
        <ReferenceExportChooser subject={heading()} groups={shown()} onClose={() => setExportChooserOpen(false)} />
      </Show>
      <div class={`${section}-references`}>
        <div class="references-header" onClick={() => setCollapsedOverride(!collapsed())}>
          <ReferenceDisclosure collapsed={collapsed()} />
          {heading()} <span class="references-count">{count()}</span>
          <Show when={props.unlinked && groupsResource.loading && groups() === undefined}><span class="references-loading"> {t("references-loading")}</span></Show>
          <button type="button" class="reference-export-toggle"
            aria-label={t(props.unlinked ? "references-copy-unlinked" : "references-copy-linked")} title={t(props.unlinked ? "references-copy-unlinked-title" : "references-copy-linked-title")}
            disabled={!count()}
            onClick={(event) => { event.stopPropagation(); setExportChooserOpen(true); }}
          ><svg viewBox="0 0 24 24" aria-hidden="true"><path d="M16 1H4a2 2 0 0 0-2 2v14h2V3h12V1zm3 4H8a2 2 0 0 0-2 2v14a2 2 0 0 0 2 2h11a2 2 0 0 0 2-2V7a2 2 0 0 0-2-2zm0 16H8V7h11v14z" /></svg></button>
          <button
            type="button"
            class="reference-filter-toggle"
            classList={{ active: filterOpen() || hasActiveFilter() }}
            aria-label={t(props.unlinked ? "references-filter-unlinked" : "references-filter-linked")}
            aria-expanded={filterOpen()}
            title={t(props.unlinked ? "references-filter-unlinked" : "references-filter-linked")}
            onClick={(event) => {
              event.stopPropagation();
              setFilterOpen(!filterOpen());
            }}
          >
            <svg viewBox="0 0 24 24" aria-hidden="true"><path d="M4 5h16l-6.2 7.1v5.4l-3.6 1.8v-7.2z" /></svg>
          </button>
        </div>
        <Show when={!collapsed()}>
          <Show when={loadError()}>
            <div class="reference-filter-error reference-error" role="alert">{referenceLoadErrorMessage(loadError()!)}</div>
          </Show>
          <Show when={occurrenceLimit().truncated}>
            <div class="reference-truncation" role="status">
              {t("references-occurrences", { shown: occurrenceLimit().shown, total: occurrenceLimit().total })}
            </div>
          </Show>
          <Show when={filterOpen()}>
            <div class="reference-filter-panel">
              <div class="reference-filter-search-row">
                <input
                  class="reference-filter-search"
                  type="search"
                  value={searchDraft()}
                  placeholder={t("references-search-placeholder")}
                  aria-label={t(props.unlinked ? "references-search-unlinked" : "references-search-linked")}
                  onInput={(event) => updateSearch(event.currentTarget.value)}
                  onKeyDown={(event) => {
                    if (event.key === "Escape") setFilterOpen(false);
                  }}
                />
                <button type="button" class="reference-filter-clear" disabled={!hasActiveFilter()} onClick={clearAllFilters}>
                  {t("references-clear")}
                </button>
              </div>
              <div class="reference-filter-summary">
                <Show
                  when={filterPending()}
                  fallback={
                    <>
                      {t("references-summary", { shown: count(), total: totalCount() })}
                      <Show when={nativeContextResource.loading}> · {t("references-indexing")}</Show>
                    </>
                  }
                >
                  {t("references-index-pending", { total: totalCount() })}
                </Show>
              </div>
              <Show when={searchError()}>
                {(error) => <div class="reference-filter-error">{t("references-invalid-search", { error: error() })}</div>}
              </Show>
              <Show when={nativeContextResource.error}>
                <div class="reference-filter-error">{t("references-index-error")}</div>
              </Show>
              <Show when={nativeContext()?.truncated || nativeContext()?.entries.some((entry) => entry.truncated)}>
                <div class="reference-filter-warning">{t("references-index-partial")}</div>
              </Show>
              <Show when={coRefs().length > 0 || orphanFilters().length > 0}>
                <div class="ref-filter" aria-label={t("references-facets")}>
                  <For each={coRefs()}>
                    {([name, n]) => (
                      <button
                        class="ref-filter-chip"
                        classList={{ "f-in": filterState(name) === "in", "f-out": filterState(name) === "out" }}
                        title={t("references-facet-cycle")}
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
                        title={t("references-facet-orphan")}
                        onClick={() => cycle(name)}
                      >{name} <span class="ref-filter-count">0</span></button>
                    )}
                  </For>
                </div>
              </Show>
            </div>
          </Show>
          <Show when={shown().length > 1}>
            <div class="reference-bulk-controls" aria-label={t(props.unlinked ? "references-groups-unlinked" : "references-groups-linked")}>
              <button type="button" onClick={() => setAllGroups(bulkCollapseTarget(shown(), groupCollapsed))}>{t(bulkCollapseTarget(shown(), groupCollapsed) ? "references-collapse-all" : "references-expand-all")}</button>
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
                    aria-label={t(groupCollapsed(group()) ? "references-expand-group" : "references-collapse-group", { page: group().page })}
                    onClick={() => setGroupCollapsed(group(), !groupCollapsed(group()))}
                  >
                    <ReferenceDisclosure collapsed={groupCollapsed(group())} />
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
                      const dest = internalLinkDest(e);
                      if (dest === "sidebar") openPageInSidebar(group().page, group().kind);
                      else if (dest === "background") openPageInNewTab(group().page, group().kind);
                      else if (dest === "pane") openRouteInOtherPane({ kind: "page", name: group().page, pageKind: group().kind });
                      else openPage(group().page, group().kind);
                    }}
                    onMouseDown={internalLinkMouseDown}
                    onAuxClick={(e) => internalLinkAuxClick(e, () => openPageInNewTab(group().page, group().kind))}
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
                      path={group().path}
                      surface="ref"
                      showBreadcrumb
                      excerpt={props.unlinked}
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
