import { For, Show, createResource, createSignal, createMemo, createEffect, type JSX } from "solid-js";
import { backend } from "../backend";
import { openPage, openPageInNewTab } from "../router";
import { openPageInSidebar, openPageContextMenu } from "../ui";
import { LiveRefGroup } from "./LiveRefGroup";
import type { BlockDto, RefGroup } from "../types";

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
function refsInRaw(raw: string): string[] {
  const out: string[] = [];
  const re = /\[\[([^\]]+)\]\]|#\[\[([^\]]+)\]\]|#([\w/_.-]+)/g;
  let m: RegExpExecArray | null;
  while ((m = re.exec(raw))) out.push(m[1] ?? m[2] ?? m[3]);
  return out;
}

/** Filter facets contributed by a backlink root and its visible descendants.
 *  OG's reference filter treats task states like TODO as facets too. Keep one
 *  value per root (so counts are backlink counts, not repeated-token counts). */
function filterFacets(block: BlockDto, target: string): Map<string, string> {
  const facets = new Map<string, string>();
  const visit = (current: BlockDto) => {
    for (const ref of refsInRaw(current.raw)) {
      const key = norm(ref);
      if (key !== norm(target) && !facets.has(key)) facets.set(key, ref);
    }
    if (current.marker) {
      const key = norm(current.marker);
      if (!facets.has(key)) facets.set(key, current.marker);
    }
    for (const child of current.children) visit(child);
  };
  visit(block);
  return facets;
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
  // page name -> "in" (must also reference) | "out" (must not reference).
  const [filters, setFilters] = createSignal<FilterMap>(loadFilters(props.name));
  // Reload the saved filter when the page changes.
  createEffect(() => setFilters(loadFilters(props.name)));

  // Co-referenced pages/tags and task states in each backlink tree, with counts.
  const coRefs = createMemo(() => {
    const counts = new Map<string, { name: string; count: number }>();
    for (const g of groups() ?? []) {
      for (const b of g.blocks) {
        for (const [key, name] of filterFacets(b, props.name)) {
          const previous = counts.get(key);
          counts.set(key, { name: previous?.name ?? name, count: (previous?.count ?? 0) + 1 });
        }
      }
    }
    return [...counts.values()]
      .map(({ name, count }) => [name, count] as const)
      .sort((a, b) => b[1] - a[1] || a[0].localeCompare(b[0]));
  });

  const shown = createMemo<RefGroup[]>(() => {
    const f = filters();
    const ins = Object.keys(f).filter((k) => f[k] === "in").map(norm);
    const outs = Object.keys(f).filter((k) => f[k] === "out").map(norm);
    if (!ins.length && !outs.length) return groups() ?? [];
    return (groups() ?? [])
      .map((g) => ({
        ...g,
        blocks: g.blocks.filter((b) => {
          const facets = filterFacets(b, props.name);
          return ins.every((i) => facets.has(i)) && outs.every((o) => !facets.has(o));
        }),
      }))
      .filter((g) => g.blocks.length > 0);
  });

  const cycle = (name: string) => {
    const f = { ...filters() };
    f[name] = f[name] === "in" ? "out" : f[name] === "out" ? (undefined as never) : "in";
    if (f[name] === undefined) delete f[name];
    setFilters(f);
    saveFilters(props.name, f);
  };
  const count = () => shown().reduce((acc, g) => acc + g.blocks.length, 0);

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
        </div>
        <Show when={!collapsed()}>
          <Show when={coRefs().length > 1}>
            <div class="ref-filter" onClick={(e) => e.stopPropagation()}>
              <For each={coRefs()}>
                {([name, n]) => (
                  <button
                    class="ref-filter-chip"
                    classList={{ "f-in": filters()[name] === "in", "f-out": filters()[name] === "out" }}
                    title="Click: include · again: exclude · again: clear"
                    onClick={() => cycle(name)}
                  >
                    {name} <span class="ref-filter-count">{n}</span>
                  </button>
                )}
              </For>
            </div>
          </Show>
          <For each={shown()}>
            {(g) => (
              <div class="reference-group">
                <div
                  class="reference-page"
                  onClick={(e) => {
                    if (e.shiftKey) openPageInSidebar(g.page, g.kind);
                    else openPage(g.page, g.kind);
                  }}
                  onAuxClick={(e) => {
                    if (e.button === 1) {
                      e.preventDefault();
                      openPageInNewTab(g.page, g.kind);
                    }
                  }}
                  onContextMenu={(e) => {
                    e.preventDefault();
                    openPageContextMenu(e.clientX, e.clientY, g.page, g.kind);
                  }}
                >
                  {g.page}
                </div>
                <div class="reference-blocks">
                  <LiveRefGroup page={g.page} kind={g.kind} blocks={g.blocks} />
                </div>
              </div>
            )}
          </For>
        </Show>
      </div>
    </Show>
  );
}
