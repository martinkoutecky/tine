import { For, Show, createResource, createSignal, createMemo, createEffect, onCleanup, type JSX } from "solid-js";
import { backend } from "../backend";
import { openPage, openPageInNewTab } from "../router";
import { openPageInSidebar, openPageContextMenu } from "../ui";
import { LiveRefGroup } from "./LiveRefGroup";
import type { BacklinkFilterEntry, BacklinkFilterTarget, BlockDto, RefGroup } from "../types";
import { shouldOpenTextContextMenu } from "../contextMenuPolicy";
import { canonicalFold, matcherMatches, parseSearchQuery } from "../editor/searchQuery";

const norm = (s: string) => s.trim().toLowerCase();

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
export function LinkedReferences(props: { name: string }): JSX.Element {
  const [groups] = createResource(
    () => props.name,
    (n) => backend().getBacklinks(n)
  );
  const [collapsed, setCollapsed] = createSignal(false);
  const [collapsedGroups, setCollapsedGroups] = createSignal<Set<string>>(new Set());
  const [filterOpen, setFilterOpen] = createSignal(false);
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
    props.name;
    setFilters(loadFilters(props.name));
    setFilterOpen(false);
    setSearchDraft("");
    setSearchQuery("");
  });

  const targets = createMemo<BacklinkFilterTarget[]>(() =>
    (groups() ?? []).flatMap((group) =>
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
      (groups() ?? []).flatMap((group) =>
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

  // Co-referenced pages/tags and task states in each backlink tree, with counts.
  const coRefs = createMemo(() => {
    const counts = new Map<string, { name: string; count: number }>();
    for (const g of groups() ?? []) {
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

  const parsedSearch = createMemo(() => parseSearchQuery(searchQuery()));
  const searchError = createMemo(() => {
    const parsed = parsedSearch();
    return parsed.kind === "invalid" ? parsed.error : null;
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
    if ((searching || ins.length || outs.length) && nativeContext.loading) return groups() ?? [];
    if (!searching && !ins.length && !outs.length) return groups() ?? [];
    return (groups() ?? [])
      .map((g) => ({
        ...g,
        blocks: g.blocks.filter((b) => {
          const entry = rootEntry(g, b);
          const facets = new Set(entry.facets.map(norm));
          const contentMatches = !searching || matcherMatches(parsed, entry.normalizedText, entry.text);
          return contentMatches && ins.every((i) => facets.has(i)) && outs.every((o) => !facets.has(o));
        }),
      }))
      .map((g) => {
        const ids = new Set(g.blocks.map((block) => block.id));
        return { ...g, evidence: g.evidence?.filter((item) => ids.has(item.block_id)) };
      })
      .filter((g) => g.blocks.length > 0);
  });

  const groupKey = (group: RefGroup) => `${group.kind}:${group.page}`;
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
  const totalCount = () => (groups() ?? []).reduce((acc, g) => acc + g.blocks.length, 0);
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
    <Show when={groups() && groups()!.length > 0}>
      <div class="linked-references">
        <div class="references-header" onClick={() => setCollapsed(!collapsed())}>
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
        </div>
        <Show when={!collapsed()}>
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
              <Show when={coRefs().length > 0}>
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
                    type="button"
                    class="reference-page"
                    onClick={(e) => {
                      if (e.shiftKey) openPageInSidebar(group().page, group().kind);
                      else openPage(group().page, group().kind);
                    }}
                    onAuxClick={(e) => {
                      if (e.button === 1) {
                        e.preventDefault();
                        openPageInNewTab(group().page, group().kind);
                      }
                    }}
                    onContextMenu={(e) => {
                      if (!shouldOpenTextContextMenu(e.target)) return;
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
  );
}
