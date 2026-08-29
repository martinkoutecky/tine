import { For, Show, createResource, createSignal, createMemo, createEffect, onCleanup, type JSX } from "solid-js";
import { backend } from "../backend";
import { openPage, openPageInNewTab } from "../router";
import { openPageInSidebar, openPageContextMenu } from "../ui";
import { LiveRefGroup } from "./LiveRefGroup";
import type { BacklinkFilterEntry, BacklinkFilterTarget, BlockDto, RefGroup } from "../types";
import { shouldOpenPageContextMenu } from "../contextMenuPolicy";
import { isAndroidPlatform } from "../nativeChrome";
import { internalLinkAuxClick, internalLinkDest, internalLinkMouseDown } from "../linkGesture";
import { canonicalFold, matcherMatches, parseSearchQuery } from "../editor/searchQuery";
import {
  classifyReferenceLoadError,
  referenceLoadErrorMessage,
  type ReferenceLoadError,
} from "../lib/referenceLoadError";
import {
  collapsedGroupsFor,
  sectionOverride,
  setCollapsedGroupsFor,
  setSectionOverride,
} from "../referenceSectionState";
import { pageIdentityKey } from "../pageIdentity";
import { mergeReferenceGroups } from "../lib/referenceGroups";
import { ReferenceExportChooser } from "./ReferenceExportChooser";
import { createLongPress, isLongPressContextMenu } from "../render/longPress";

// One identity fold for chips, filters, and group merging (DUP-2/DUP-8): the
// old private `norm` (trim+toLowerCase) split NFC/NFD and boundary-slash
// spellings of one page into separate chips whose filters missed each other,
// and mismatched the backend, which keys the same request with refs::normalize.
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
    normalizedText: canonicalFold(entry.text),
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
const OG_REFERENCE_COLLAPSE_THRESHOLD = 100;

export function LinkedReferences(props: { name: string }): JSX.Element {
  const [loadError, setLoadError] = createSignal<ReferenceLoadError | null>(null);
  const [groups] = createResource(
    () => props.name,
    async (n) => {
      setLoadError(null);
      try {
        return await backend().getBacklinks(n);
      } catch (error) {
        setLoadError(classifyReferenceLoadError(error));
        return [];
      }
    }
  );
  const mergedGroups = createMemo(() => mergeReferenceGroups(groups() ?? []));
  // GH #272: held outside the component so a remount cannot silently re-collapse
  // a section the user expanded. See referenceSectionState.
  const [collapsedOverrideSignal, setCollapsedOverrideSignal] =
    createSignal<boolean | null>(sectionOverride("linked", props.name) ?? null);
  const collapsedOverride = collapsedOverrideSignal;
  const setCollapsedOverride = (value: boolean) => {
    setSectionOverride("linked", props.name, value);
    setCollapsedOverrideSignal(value);
  };
  const [collapsedGroupsSignal, setCollapsedGroupsSignal] =
    createSignal<Set<string>>(collapsedGroupsFor("linked", props.name));
  const collapsedGroups = collapsedGroupsSignal;
  const setCollapsedGroups = (next: Set<string> | ((current: Set<string>) => Set<string>)) => {
    setCollapsedGroupsSignal((current) => {
      const value = typeof next === "function" ? next(current) : next;
      setCollapsedGroupsFor("linked", props.name, value);
      return value;
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
  // Reload the saved per-page state when the page changes. The section's own
  // expand/collapse is RESTORED here, not reset (GH #272): a pane can route to
  // another page without remounting, and coming back must not silently discard
  // the choice the user made on the first page.
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

  const parsedSearch = createMemo(() => parseSearchQuery(searchQuery()));
  const searchError = createMemo(() => {
    const parsed = parsedSearch();
    return parsed.kind === "invalid" ? parsed.error : null;
  });

  /** Filter backlink roots, trim each group's evidence to the survivors, and
   *  drop groups that lose every root. Shared so the text pass runs ONCE and
   *  both the facet chips and the reference list read the same result. */
  const filterGroups = (
    groups: RefGroup[],
    keep: (group: RefGroup, block: BlockDto) => boolean
  ): RefGroup[] =>
    groups
      .map((g) => ({ ...g, blocks: g.blocks.filter((b) => keep(g, b)) }))
      .map((g) => {
        const ids = new Set(g.blocks.map((block) => block.id));
        return { ...g, evidence: g.evidence?.filter((item) => ids.has(item.block_id)) };
      })
      .filter((g) => g.blocks.length > 0);

  // The text query applied on its own, WITHOUT the facet chips. The chips are
  // the list the user picks from, so they must follow the typed text (GH #173
  // follow-up — OG's "Search in linked pages" narrows exactly this) but must
  // NOT follow the chip selections, or selecting a chip would remove the
  // controls needed to undo it.
  const textMatchedGroups = createMemo<RefGroup[]>(() => {
    const parsed = parsedSearch();
    const searching = parsed.kind !== "empty" && parsed.kind !== "invalid";
    if (!searching || nativeContext.loading) return mergedGroups();
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

  // An active include/exclude chip whose last backlink the text query filtered
  // away still has to be reachable, or the user is stranded with an invisible
  // filter and zero results. These are listed after the matching chips at zero.
  // Deliberately a SEPARATE memo: folding them into coRefs() would make the
  // whole chip list depend on filters(), so every click would re-create every
  // chip node mid-cycle.
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
    // index is still in flight: the fallback corpus is a SUBSET of the native
    // one, so a fallback miss cannot prove a real miss and dropping the root
    // would hide a genuine match. Once the index arrives filtering is
    // synchronous. The summary says so rather than reporting a filtered count.
    if ((searching || ins.length || outs.length) && nativeContext.loading) return mergedGroups();
    if (!ins.length && !outs.length) return textMatchedGroups();
    // GH #273: positive include chips OR — a backlink stays when ANY included
    // page/tag is present, and zero positive chips leaves the facet side
    // unconstrained. Exclude chips stay cumulative, and because this runs over
    // textMatchedGroups() the text filter stays conjunctive with the facets.
    return filterGroups(textMatchedGroups(), (group, block) => {
      const facets = new Set(rootEntry(group, block).facets.map(norm));
      return (ins.length === 0 || ins.some((i) => facets.has(i)))
        && outs.every((o) => !facets.has(o));
    });
  });

  const groupKey = (group: RefGroup) => pageIdentityKey(group.page);
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
  const collapsed = () => collapsedOverride() ?? totalCount() >= OG_REFERENCE_COLLAPSE_THRESHOLD;
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
  /** A filter is asked for but the descendant index it needs has not arrived,
   *  so the list below is deliberately UNFILTERED. Say that instead of
   *  reporting "N of N references", which asserts a finished filter. */
  const filterPending = () => {
    if (!nativeContext.loading) return false;
    const parsed = parsedSearch();
    const searching = parsed.kind !== "empty" && parsed.kind !== "invalid";
    return searching || Object.keys(filters()).length > 0;
  };
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
            {referenceLoadErrorMessage(loadError()!)}
          </div>
        </div>
      }
    >
    <Show when={groups() && mergedGroups().length > 0}>
      <Show when={exportChooserOpen()}>
        {/* GH #348: batch export honors the visible (filtered) set, matching
            what the section actually shows the user right now. */}
        <ReferenceExportChooser
          subject="Linked References"
          groups={shown()}
          onClose={() => setExportChooserOpen(false)}
        />
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
            class="reference-export-toggle"
            aria-label="Copy / export linked references"
            title="Copy / export selected linked references"
            onClick={(event) => {
              event.stopPropagation();
              setExportChooserOpen(true);
            }}
          >
            <svg viewBox="0 0 24 24" aria-hidden="true"><path d="M16 1H4a2 2 0 0 0-2 2v14h2V3h12V1zm3 4H8a2 2 0 0 0-2 2v14a2 2 0 0 0 2 2h11a2 2 0 0 0 2-2V7a2 2 0 0 0-2-2zm0 16H8V7h11v14z" fill="currentColor" /></svg>
          </button>
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
                <Show
                  when={filterPending()}
                  fallback={
                    <>
                      {count()} of {totalCount()} references
                      <Show when={nativeContext.loading}> · indexing…</Show>
                    </>
                  }
                >
                  Indexing {totalCount()} references… the filter applies when this finishes
                </Show>
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
                      <button
                        class="ref-filter-chip"
                        classList={{ "f-in": filterState(name) === "in", "f-out": filterState(name) === "out" }}
                        title="No match in the current text search · click to cycle or clear"
                        onClick={() => cycle(name)}
                      >
                        {name} <span class="ref-filter-count">0</span>
                      </button>
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
                    data-page-context-menu
                    onMouseDown={internalLinkMouseDown}
                    onClick={(e) => {
                      if (longPress.consumeClick()) {
                        e.preventDefault();
                        e.stopPropagation();
                        return;
                      }
                      const dest = internalLinkDest(e);
                      if (dest === "sidebar") openPageInSidebar(group().page, group().kind);
                      else if (dest === "background") openPageInNewTab(group().page, group().kind);
                      else openPage(group().page, group().kind);
                    }}
                    onPointerDown={longPress.onPointerDown}
                    onPointerMove={longPress.onPointerMove}
                    onPointerUp={longPress.onPointerUp}
                    onPointerCancel={longPress.onPointerCancel}
                    onAuxClick={(e) => internalLinkAuxClick(e, () => openPageInNewTab(group().page, group().kind))}
                    onContextMenu={(e) => {
                      if (!shouldOpenPageContextMenu(e.target)) return;
                      const synthetic = isLongPressContextMenu(e);
                      if (isAndroidPlatform && !synthetic && longPress.completedHold()) {
                        e.preventDefault();
                        e.stopPropagation();
                        return;
                      }
                      if (isAndroidPlatform && !synthetic) longPress.dispose();
                      e.preventDefault();
                      e.stopPropagation();
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
