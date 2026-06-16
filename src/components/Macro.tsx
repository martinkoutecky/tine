import { For, Show, Switch, Match, createMemo, createResource, createSignal, type JSX } from "solid-js";
import { backend } from "../backend";
import { openPage, openPageInNewTab } from "../router";
import { openPageInSidebar, openPageContextMenu } from "../ui";
import { doc, ensurePageLoaded, pageByName } from "../store";
import { RefBlocks } from "./RefBlocks";
import { Block } from "./Block";
import { blockView } from "../render/block";
import { InlineText } from "../render/inline";
import type { PageKind, RefGroup } from "../types";

const ADVANCED_RE = /\[\s*:find|:where|:find/;

// Collapsed state for query results, keyed by query string and persisted so a
// query you fold (e.g. a TODO dashboard parked in the sidebar) stays folded.
const QCOLLAPSE_KEY = "logseq-claude.queryCollapsed";
function loadCollapsed(q: string): boolean {
  try {
    const m = JSON.parse(localStorage.getItem(QCOLLAPSE_KEY) ?? "{}");
    return !!m[q];
  } catch {
    return false;
  }
}
function saveCollapsed(q: string, v: boolean) {
  try {
    const m = JSON.parse(localStorage.getItem(QCOLLAPSE_KEY) ?? "{}");
    if (v) m[q] = true;
    else delete m[q];
    localStorage.setItem(QCOLLAPSE_KEY, JSON.stringify(m));
  } catch {
    // ignore
  }
}

interface Row {
  page: string;
  kind: PageKind;
  text: string;
  props: Record<string, string>;
}

// A {{query ...}} block: runs the query and renders matching blocks as a list
// or a sortable table.
export function QueryMacro(props: { body: string }): JSX.Element {
  const arg = () => props.body.replace(/^query\s*/i, "").trim();
  const [groups] = createResource(arg, (q) => backend().runQuery(q));
  const total = () => groups()?.reduce((a, g) => a + g.blocks.length, 0) ?? 0;
  const [table, setTable] = createSignal(false);
  const [sortCol, setSortCol] = createSignal<string>("");
  const [sortDir, setSortDir] = createSignal(1);
  const [collapsed, setCollapsed] = createSignal(loadCollapsed(arg()));
  const toggleCollapsed = () => {
    const v = !collapsed();
    setCollapsed(v);
    saveCollapsed(arg(), v);
  };

  const rows = createMemo<Row[]>(() =>
    (groups() ?? []).flatMap((g) =>
      g.blocks.map((b) => {
        const v = blockView(b.raw);
        const props: Record<string, string> = {};
        for (const [k, val] of v.properties) props[k] = val;
        return { page: g.page, kind: g.kind, text: v.lines.join(" "), props };
      })
    )
  );

  const cols = createMemo(() => {
    const keys = new Set<string>();
    for (const r of rows()) for (const k of Object.keys(r.props)) keys.add(k);
    return Array.from(keys);
  });

  const sorted = createMemo(() => {
    const c = sortCol();
    if (!c) return rows();
    const val = (r: Row) => (c === "page" ? r.page : c === "content" ? r.text : r.props[c] ?? "");
    return [...rows()].sort((a, b) => val(a).localeCompare(val(b)) * sortDir());
  });

  const sortBy = (c: string) => {
    if (sortCol() === c) setSortDir(-sortDir());
    else {
      setSortCol(c);
      setSortDir(1);
    }
  };
  // Clicks on query controls must not bubble to the block's onClick (which would
  // start editing the {{query}} block and replace results with raw markdown).
  const stop = (e: MouseEvent) => e.stopPropagation();
  const arrow = (c: string) => (sortCol() === c ? (sortDir() > 0 ? " ▲" : " ▼") : "");

  return (
    <div class="query-block">
      <Switch>
        <Match when={ADVANCED_RE.test(arg())}>
          <div class="query-unsupported">
            Advanced (datalog) query not supported yet: <code>{`{{${props.body}}}`}</code>
          </div>
        </Match>
        <Match when={true}>
          <div class="query-header">
            <span
              class="query-collapse"
              classList={{ collapsed: collapsed() }}
              title={collapsed() ? "Expand results" : "Collapse results"}
              onClick={(e) => {
                e.stopPropagation();
                toggleCollapsed();
              }}
            >
              <svg viewBox="0 0 24 24" class="triangle">
                <path d="M8 5l8 7-8 7z" />
              </svg>
            </span>
            Query <span class="query-count">{total()}</span>
            <button
              class="query-view-toggle"
              onClick={(e) => {
                e.stopPropagation();
                setTable(!table());
              }}
            >
              {table() ? "List" : "Table"}
            </button>
          </div>
          <Show when={!collapsed()}>
          <Show
            when={groups() && groups()!.length > 0}
            fallback={<div class="query-empty">No results</div>}
          >
            <Show
              when={table()}
              fallback={<For each={groups()}>{(g) => <QueryGroup group={g} />}</For>}
            >
              <table class="md-table query-table">
                <thead>
                  <tr onClick={stop}>
                    <th onClick={() => sortBy("content")}>Content{arrow("content")}</th>
                    <th onClick={() => sortBy("page")}>Page{arrow("page")}</th>
                    <For each={cols()}>
                      {(c) => <th onClick={() => sortBy(c)}>{c}{arrow(c)}</th>}
                    </For>
                  </tr>
                </thead>
                <tbody>
                  <For each={sorted()}>
                    {(r) => (
                      <tr>
                        <td>
                          <InlineText text={r.text} />
                        </td>
                        <td
                          class="qt-page"
                          onClick={(e) => {
                            e.stopPropagation();
                            if (e.shiftKey) openPageInSidebar(r.page, r.kind);
                            else openPage(r.page, r.kind);
                          }}
                          onAuxClick={(e) => {
                            if (e.button === 1) {
                              e.preventDefault();
                              e.stopPropagation();
                              openPageInNewTab(r.page, r.kind);
                            }
                          }}
                          onContextMenu={(e) => {
                            e.preventDefault();
                            e.stopPropagation();
                            openPageContextMenu(e.clientX, e.clientY, r.page, r.kind);
                          }}
                        >
                          {r.page}
                        </td>
                        <For each={cols()}>{(c) => <td>{r.props[c] ?? ""}</td>}</For>
                      </tr>
                    )}
                  </For>
                </tbody>
              </table>
            </Show>
          </Show>
          </Show>
        </Match>
      </Switch>
    </div>
  );
}

// One page's query results, rendered as LIVE editable blocks. The result page
// is loaded into the shared working set on demand; each result is the same
// <Block> the main view uses (so editing a result edits the real block and
// saves to its page). Until the page is loaded, a read-only block stands in.
function QueryGroup(props: { group: RefGroup }): JSX.Element {
  const g = () => props.group;
  const [ready] = createResource(
    () => ({ p: g().page, k: g().kind }),
    async ({ p, k }) => {
      if (!pageByName(p)) {
        const dto = await backend().getPage(p, k);
        if (dto) ensurePageLoaded(dto);
      }
      return true;
    }
  );
  return (
    <div class="query-group">
      <div
        class="query-page"
        onClick={(e) => {
          e.stopPropagation();
          if (e.shiftKey) openPageInSidebar(g().page, g().kind);
          else openPage(g().page, g().kind);
        }}
        onAuxClick={(e) => {
          if (e.button === 1) {
            e.preventDefault();
            e.stopPropagation();
            openPageInNewTab(g().page, g().kind);
          }
        }}
        onContextMenu={(e) => {
          e.preventDefault();
          e.stopPropagation();
          openPageContextMenu(e.clientX, e.clientY, g().page, g().kind);
        }}
      >
        {g().page}
      </div>
      <For each={g().blocks}>
        {(b) => (
          <Show
            when={ready() && doc.byId[b.id]}
            fallback={<RefBlocks blocks={[b]} page={g().page} pageKind={g().kind} />}
          >
            <Block id={b.id} />
          </Show>
        )}
      </For>
    </div>
  );
}

// A {{embed ((uuid))}} or {{embed [[Page]]}} block.
export function EmbedMacro(props: { body: string }): JSX.Element {
  const target = () => props.body.replace(/^embed\s*/i, "").trim();

  const [data] = createResource(target, async (t) => {
    const blockRef = /^\(\(([^)]+)\)\)$/.exec(t);
    if (blockRef) {
      const g = await backend().resolveBlock(blockRef[1]);
      return g ? { label: g.page, blocks: g.blocks } : null;
    }
    const pageRef = /^\[\[([^\]]+)\]\]$/.exec(t);
    if (pageRef) {
      const p = await backend().getPage(pageRef[1], "page");
      return p ? { label: p.name, blocks: p.blocks } : null;
    }
    return null;
  });

  return (
    <div class="embed-block">
      <Show when={data()} fallback={<div class="embed-missing">{`{{${props.body}}}`}</div>}>
        <RefBlocks blocks={data()!.blocks} page={data()!.label} />
      </Show>
    </div>
  );
}
