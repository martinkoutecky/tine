import { For, Show, createResource, createSignal, createMemo, type JSX } from "solid-js";
import { backend } from "../backend";
import { openPage, openPageInNewTab } from "../router";
import { openPageInSidebar, openPageContextMenu } from "../ui";
import { LiveRefGroup } from "./LiveRefGroup";
import type { RefGroup } from "../types";

const norm = (s: string) => s.trim().toLowerCase();
function refsInRaw(raw: string): string[] {
  const out: string[] = [];
  const re = /\[\[([^\]]+)\]\]|#\[\[([^\]]+)\]\]|#([\w/_.-]+)/g;
  let m: RegExpExecArray | null;
  while ((m = re.exec(raw))) out.push(m[1] ?? m[2] ?? m[3]);
  return out;
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
  const [filters, setFilters] = createSignal<Record<string, "in" | "out">>({});

  // Co-referenced pages (other pages mentioned alongside the target), with counts.
  const coRefs = createMemo(() => {
    const counts: Record<string, number> = {};
    for (const g of groups() ?? []) {
      for (const b of g.blocks) {
        for (const r of refsInRaw(b.raw)) {
          if (norm(r) === norm(props.name)) continue;
          counts[r] = (counts[r] ?? 0) + 1;
        }
      }
    }
    return Object.entries(counts).sort((a, b) => b[1] - a[1]);
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
          const rs = refsInRaw(b.raw).map(norm);
          return ins.every((i) => rs.includes(i)) && outs.every((o) => !rs.includes(o));
        }),
      }))
      .filter((g) => g.blocks.length > 0);
  });

  const cycle = (name: string) => {
    const f = { ...filters() };
    f[name] = f[name] === "in" ? "out" : f[name] === "out" ? (undefined as never) : "in";
    if (f[name] === undefined) delete f[name];
    setFilters(f);
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
