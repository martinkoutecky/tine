import { For, Show, Switch, Match, createMemo, createResource, createSignal, type JSX } from "solid-js";
import { backend } from "../backend";
import { openPage, openPageInNewTab } from "../router";
import { openPageInSidebar, openPageContextMenu, dataRev, graphEpoch, graphMeta } from "../ui";
import { blockProperty, doc, formatForPage, formatForBlock, resolveGuidePageDto, setBlockProperty, setRaw, withUndoUnit } from "../store";
import { resolveBlockBatched } from "../resolveBatch";
import { LiveRefGroup } from "./LiveRefGroup";
import { QueryBuilder } from "./QueryBuilder";
import {
  advancedToClause,
  clearSimpleForm,
  getSimpleForm,
  parseQuery,
  toDsl,
  type Clause,
} from "../editor/queryBuilder";
import { foldAggregate, groupRows } from "../editor/queryAggregate";
import { quoteEdnString, unquoteEdnString, splitTrailingMap, queryMacroExtents } from "../editor/edn";
import { visibleBody } from "../render/block";
import { facetsOf } from "../render/facets";
import { sheetConfig } from "../sheet/config";
import { InlineText } from "../render/inline";
import { SheetTable } from "./SheetTable";
import { SheetBoard } from "./SheetBoard";
import { SheetContainer } from "./SheetContainer";
import type { PageKind, RefGroup } from "../types";

const ADVANCED_RE = /\[\s*:find|:where|:find/;
type QueryView = "list" | "table" | "board";
const QUERY_VIEWS: QueryView[] = ["list", "table", "board"];
const QUERY_VIEW_LABEL: Record<QueryView, string> = {
  list: "List",
  table: "Table",
  board: "Board",
};

// Collapsed state for query results, keyed by graph + rendered query identity.
// A raw query-string key made unrelated dashboards across pages/graphs collide.
const QCOLLAPSE_KEY = "logseq-claude.queryCollapsed";
function loadCollapsed(key: string): boolean | null {
  try {
    const m = JSON.parse(localStorage.getItem(QCOLLAPSE_KEY) ?? "{}");
    return typeof m[key] === "boolean" ? m[key] : null;
  } catch {
    return null;
  }
}
function saveCollapsed(key: string, v: boolean) {
  try {
    const m = JSON.parse(localStorage.getItem(QCOLLAPSE_KEY) ?? "{}");
    // Keep explicit false: it overrides a source `:collapsed? true` default on
    // remount. Deleting false made an expanded query re-collapse immediately.
    m[key] = v;
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

interface ParsedQuery {
  form: string; // the query form, without the options map
  opts: string; // the raw trailing `{…}` options map, or ""
  title?: string;
  collapsed?: boolean;
  tableView?: boolean;
}
// Split a trailing front-matter options map off the query DSL and read the
// display options OG supports (:title / :collapsed? / :table-view?).
function splitQuery(arg: string): ParsedQuery {
  const { form, opts } = splitTrailingMap(arg);
  const tm = /:title\s+"((?:[^"\\]|\\.)*)"/.exec(opts);
  return {
    form,
    opts,
    title: tm ? unquoteEdnString(tm[1]) : undefined,
    collapsed: /:collapsed\?\s+true/.test(opts),
    tableView: /:table-view\?\s+true/.test(opts),
  };
}

// A {{query ...}} block: runs the query and renders matching blocks as a list
// or a sortable table. When `blockId` is given (the block is a standalone query
// block, not an inline-in-text macro) an interactive builder bar is shown and
// edits rewrite the {{query ...}} macro in that block's raw text.
export function QueryMacro(props: {
  body: string;
  blockId?: string;
  title?: string;
  // When set, render nothing at all if the query has no results (used for the
  // app-inserted journal agenda, which should disappear once vacated — unlike a
  // user-authored {{query}} block, which keeps showing "No results" so it stays
  // editable).
  hideWhenEmpty?: boolean;
}): JSX.Element {
  const arg = () => props.body.replace(/^query\s*/i, "").trim();
  // Split a trailing front-matter options map ({:title … :collapsed? … :table-view? …})
  // off the query form, so builder/engine see only the form and the options drive
  // display defaults.
  const parsed = createMemo(() => splitQuery(arg()));
  const form = () => parsed().form;
  const sheet = createMemo(() => {
    if (!props.blockId || !doc.byId[props.blockId]) return null;
    return sheetConfig(facetsOf(doc.byId[props.blockId].raw, formatForBlock(props.blockId)).properties);
  });
  const currentView = (): QueryView => {
    if (!props.blockId) return "list";
    const view = blockProperty(props.blockId, "tine.view");
    return view === "table" || view === "board" ? view : "list";
  };
  const sheetFace = () => currentView() === "table" || currentView() === "board";
  const legacyTable = () => currentView() === "list" && parsed().tableView === true;
  const setQueryView = (next: QueryView) => {
    const blockId = props.blockId;
    if (!blockId) return;
    const node = doc.byId[blockId];
    if (!node) return;
    const storedView = blockProperty(blockId, "tine.view");
    if ((next === "list" && storedView === null) || (next !== "list" && storedView === next)) return;
    withUndoUnit(`query:view:${next}`, [node.page], () => {
      if (next === "list") {
        setBlockProperty(blockId, "tine.view", null);
        return;
      }
      setBlockProperty(blockId, "tine.view", next);
      if (next === "board" && blockProperty(blockId, "tine.group-by") === null) {
        setBlockProperty(blockId, "tine.group-by", "state");
      }
    });
  };

  // Rewrite just THIS {{query ...}} macro inside the owning block, preserving the
  // front-matter options and surrounding property lines (id::/collapsed::). The
  // extents are found brace/string/page-ref-aware (queryMacroExtents), NOT a lazy
  // regex. A block can hold more than one query, so target the extent whose
  // current body matches OURS (props.body) — editing the 2nd query must not
  // rewrite the 1st. Falls back to the only/first query for the common case.
  const rewriteMacro = (newMacro: string) => {
    if (!props.blockId) return;
    const raw = doc.byId[props.blockId]?.raw ?? "";
    const extents = queryMacroExtents(raw);
    if (!extents.length) return;
    const norm = (s: string) => s.replace(/\s+/g, " ").trim();
    const mine = norm(props.body);
    const target = extents.find((e) => norm(raw.slice(e.start + 2, e.end - 2)) === mine) ?? extents[0];
    setRaw(props.blockId, raw.slice(0, target.start) + newMacro + raw.slice(target.end));
  };
  const applyDsl = (dsl: string) => {
    const opts = parsed().opts ? ` ${parsed().opts}` : "";
    rewriteMacro(`{{query ${dsl}${opts}}}`);
  };
  // Edit the query's display title (:title "…" in the options map). Only offered
  // for a user-authored standalone query (blockId set, no app-supplied title).
  const [editingTitle, setEditingTitle] = createSignal(false);
  const titleText = () => props.title ?? parsed().title ?? "Query";
  const titleEditable = () => !!props.blockId && props.title === undefined;
  const setTitle = (t: string) => {
    if (!props.blockId) return;
    const inner = parsed().opts.replace(/^\{|\}$/g, "").trim();
    // Drop any existing :title (escape-aware), keep the other options.
    const rest = inner.replace(/:title\s+"(?:[^"\\]|\\.)*"\s*/, "").trim();
    // Strip chars that would break the {{…}} macro / {…} options map; escape the
    // rest so quotes/backslashes round-trip faithfully through splitQuery.
    const title = t.trim().replace(/[\r\n{}]/g, "");
    const parts = [title ? `:title "${quoteEdnString(title)}"` : "", rest].filter(Boolean);
    const opts = parts.length ? ` {${parts.join(" ")}}` : "";
    rewriteMacro(`{{query ${form()}${opts}}}`);
  };

  const isAdvanced = () => ADVANCED_RE.test(arg());
  const simpleBackDsl = createMemo<string | null>(() => {
    const blockId = props.blockId;
    if (!blockId || !isAdvanced()) return null;
    const stashed = getSimpleForm(blockId);
    if (stashed !== undefined) return stashed;
    const c = advancedToClause(form());
    return c ? toDsl(c) : null;
  });
  const simpleBackTitle = () =>
    simpleBackDsl() !== null
      ? "Back to the visual query builder"
      : "This advanced query can't be converted back to the visual builder automatically — edit it as raw text, or rebuild it visually.";
  const backToSimple = (e: MouseEvent) => {
    e.stopPropagation();
    const blockId = props.blockId;
    if (!blockId) return;
    const stashed = getSimpleForm(blockId);
    if (stashed !== undefined) {
      applyDsl(stashed);
      clearSimpleForm(blockId);
      return;
    }
    const c = advancedToClause(form());
    if (c) applyDsl(toDsl(c));
  };
  const simpleBackButton = () => (
    <span
      class="query-simple-toggle-wrap"
      title={simpleBackTitle()}
      onClick={(e) => e.stopPropagation()}
    >
      <button
        type="button"
        class="qb-sort query-simple-toggle"
        title={simpleBackTitle()}
        disabled={simpleBackDsl() === null}
        onClick={backToSimple}
      >
        ← Simple
      </button>
    </span>
  );
  const currentPage = () => (props.blockId ? doc.byId[props.blockId]?.page : undefined);
  const [advInfo, setAdvInfo] = createSignal<{ ran: string[]; ignored: string[]; supported: boolean } | null>(
    null
  );
  const collapseKey = () => JSON.stringify([
    graphMeta()?.root ?? "",
    props.blockId ?? currentPage() ?? "global",
    arg(),
  ]);
  const storedCollapse = loadCollapsed(collapseKey());
  const [collapsed, setCollapsed] = createSignal(storedCollapse ?? parsed().collapsed ?? false);
  const toggleCollapsed = () => {
    const v = !collapsed();
    setCollapsed(v);
    saveCollapsed(collapseKey(), v);
  };
  // Re-run when the query text changes OR after any save lands (dataRev), so
  // results track edits live — e.g. a task flipped to DONE leaves a (task TODO)
  // query. createResource keeps the previous value during refetch (no flicker).
  // A COLLAPSED query keys off the form only (no dataRev), so it fetches once for
  // its count and doesn't re-run a whole-graph scan on every save while hidden;
  // expanding it (key flips to include dataRev) refreshes it.
  const [groups] = createResource(
    () => (collapsed() ? `collapsed ${form()}` : `${form()} ${dataRev()}`),
    async () => {
      // Advanced (datalog) queries take a separate path that maps the supported
      // clause subset onto the engine and reports what ran vs was ignored.
      if (isAdvanced()) {
        const r = await backend().runAdvancedQuery(form(), currentPage());
        setAdvInfo({ ran: r.ran, ignored: r.ignored, supported: r.supported });
        return r.groups;
      }
      setAdvInfo(null);
      return backend().runQuery(form());
    }
  );
  const total = () => groups()?.reduce((a, g) => a + g.blocks.length, 0) ?? 0;
  // A `(sort-by …)` query is sorted GLOBALLY by the engine and returned as one
  // block per group in that order — so the list view must render flat (a single
  // ordered sequence with a per-row page breadcrumb), not grouped by page, or the
  // global order would be lost to page headers.
  const globalSort = createMemo(() => /\(\s*sort-by\b/i.test(form()));
  const [sortCol, setSortCol] = createSignal<string>("");
  const [sortDir, setSortDir] = createSignal(1);

  const rows = createMemo<Row[]>(() =>
    (groups() ?? []).flatMap((g) =>
      g.blocks.map((b) => {
        // Properties come off the DTO (computed once in Rust off the lsdoc parse);
        // the row's text is the visible body. No re-derivation here.
        const props: Record<string, string> = {};
        for (const [k, val] of b.properties ?? []) props[k] = val;
        return { page: g.page, kind: g.kind, text: visibleBody(b.raw).join(" "), props };
      })
    )
  );

  const cols = createMemo(() => {
    const keys = new Set<string>();
    for (const r of rows()) for (const k of Object.keys(r.props)) keys.add(k);
    return Array.from(keys);
  });

  // Result summarization (1a): the `(aggregate …)` / `(group-by …)` directives ride
  // in the DSL and are parse-but-ignored by the engine (it returns the full set), so
  // the math is computed HERE from the returned rows. Only the simple DSL carries
  // them (datalog aggregation is OG's :result-transform, which we list as ignored).
  const directives = createMemo<{ agg: Extract<Clause, { kind: "aggregate" }> | null; group: string | null }>(() => {
    if (isAdvanced()) return { agg: null, group: null };
    const root = parseQuery(form());
    const kids = root.kind === "op" && root.op === "and" ? root.children : [root];
    const agg = kids.find((c) => c.kind === "aggregate");
    const group = kids.find((c) => c.kind === "groupBy");
    return {
      agg: agg?.kind === "aggregate" ? agg : null,
      group: group?.kind === "groupBy" ? group.field : null,
    };
  });
  const aggLabel = () => {
    const a = directives().agg;
    if (!a || a.agg === "count") return "Count";
    return `${a.agg === "sum" ? "Sum" : "Avg"} of ${a.field}`;
  };
  type Summary =
    | { kind: "single"; text: string; skipped: number }
    | { kind: "grouped"; field: string; groups: { key: string; text: string; skipped: number }[] };
  const summary = createMemo<Summary | null>(() => {
    const d = directives();
    if (!d.agg && !d.group) return null;
    if (!d.group) return { kind: "single", ...foldAggregate(rows(), d.agg) };
    return {
      kind: "grouped",
      field: d.group,
      groups: Array.from(groupRows(rows(), d.group).entries()).map(([key, set]) => ({
        key,
        ...foldAggregate(set, d.agg),
      })),
    };
  });
  const summarySingle = () => {
    const s = summary();
    return s && s.kind === "single" ? s : null;
  };
  const summaryGrouped = () => {
    const s = summary();
    return s && s.kind === "grouped" ? s : null;
  };

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

  // Hide the whole block when asked and there's nothing to show (advanced
  // queries still render their "unsupported" notice).
  const hidden = () => props.hideWhenEmpty && !ADVANCED_RE.test(arg()) && total() === 0;

  return (
    <Show when={!hidden()}>
      <div class="query-block" classList={{ "query-sheet-block": sheetFace() }}>
        <Switch>
          <Match when={isAdvanced() && advInfo() && !advInfo()!.supported}>
            <div class="query-unsupported">
              <Show when={props.blockId}>{simpleBackButton()}</Show>
              Advanced (datalog) query: no supported clauses. <code>{`{{${props.body}}}`}</code>
            </div>
          </Match>
          <Match when={true}>
            <Show when={isAdvanced() && advInfo()?.supported}>
              <div class="query-adv-note">
                <Show when={props.blockId}>{simpleBackButton()}</Show>
                Partial datalog — ran: {advInfo()!.ran.join(", ") || "—"}
                <Show when={advInfo()!.ignored.length > 0}>
                  {` · ignored: ${advInfo()!.ignored.join(", ")}`}
                </Show>
              </div>
            </Show>
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
              <Show
                when={editingTitle()}
                fallback={
                  <span
                    class="query-title"
                    classList={{ "query-title-editable": titleEditable() }}
                    title={titleEditable() ? "Click to rename this query" : undefined}
                    onClick={(e) => {
                      if (titleEditable()) {
                        e.stopPropagation();
                        setEditingTitle(true);
                      }
                    }}
                  >
                    {titleText()}
                  </span>
                }
              >
                {(() => {
                  let canceled = false;
                  return (
                    <input
                      class="query-title-input"
                      autofocus
                      value={parsed().title ?? ""}
                      placeholder="Query title"
                      onClick={(e) => e.stopPropagation()}
                      onKeyDown={(e) => {
                        e.stopPropagation();
                        if (e.key === "Enter") {
                          setTitle(e.currentTarget.value);
                          setEditingTitle(false);
                        } else if (e.key === "Escape") {
                          canceled = true;
                          setEditingTitle(false);
                        }
                      }}
                      onBlur={(e) => {
                        if (!canceled) setTitle(e.currentTarget.value);
                        setEditingTitle(false);
                      }}
                    />
                  );
                })()}
              </Show>{" "}
              <span class="query-count">{total()}</span>
              <Show when={props.blockId}>
                <div class="query-view-switcher" role="group" aria-label="Query view" onClick={stop}>
                  <For each={QUERY_VIEWS}>
                    {(view) => (
                      <button
                        type="button"
                        classList={{ active: currentView() === view }}
                        onClick={(e) => {
                          e.stopPropagation();
                          setQueryView(view);
                        }}
                      >
                        {QUERY_VIEW_LABEL[view]}
                      </button>
                    )}
                  </For>
                </div>
              </Show>
            </div>
            {/* The visual builder only models the simple DSL. For an advanced
                (datalog) query, hide the chip bar (its clauses aren't builder-
                representable) — the block is editable as raw text by clicking it, and
                the ran/ignored note above shows which clauses took. */}
            <Show when={props.blockId && !isAdvanced()}>
              <QueryBuilder dsl={form} onChange={applyDsl} blockId={props.blockId} />
            </Show>
            <Show when={!collapsed()}>
              <Show
                when={sheetFace()}
                fallback={
                  <>
                    {/* Summary panel (1a): count/sum/avg overall, or a per-group breakdown.
                        Rendered above the full result list, which stays grouped by page. */}
                    <Show when={summarySingle()}>
                      {(s) => (
                        <div class="query-summary" onClick={stop}>
                          <span class="qs-label">{aggLabel()}:</span>{" "}
                          <span class="qs-value">{s().text}</span>
                          <Show when={s().skipped > 0}>
                            <span class="qs-skip"> ({s().skipped} non-numeric skipped)</span>
                          </Show>
                        </div>
                      )}
                    </Show>
                    <Show when={summaryGrouped()}>
                      {(s) => (
                        <table class="md-table query-summary-table" onClick={stop}>
                          <thead>
                            <tr>
                              <th>{s().field}</th>
                              <th>{aggLabel()}</th>
                            </tr>
                          </thead>
                          <tbody>
                            <For each={s().groups}>
                              {(row) => (
                                <tr>
                                  <td>{row.key}</td>
                                  <td>
                                    {row.text}
                                    <Show when={row.skipped > 0}>
                                      <span class="qs-skip"> ({row.skipped} skipped)</span>
                                    </Show>
                                  </td>
                                </tr>
                              )}
                            </For>
                          </tbody>
                        </table>
                      )}
                    </Show>
                    <Show
                      when={groups() && groups()!.length > 0}
                      fallback={<div class="query-empty">No results</div>}
                    >
                      <Show
                        when={legacyTable()}
                        fallback={
                          <Show
                            when={globalSort()}
                            fallback={
                              <For each={groups() ?? []}>
                                {(g) => <QueryGroup page={g.page} group={() => g} />}
                              </For>
                            }
                          >
                            {/* Sorted: flat global order (each group holds one block). Iterate the
                                groups DIRECTLY and pass the group object — re-`find()`ing the group
                                by page/id for every row was O(groups²) on broad queries (audit #3). */}
                            <For each={groups() ?? []}>
                              {(g) => <QueryGroup page={g.page} group={() => g} flat />}
                            </For>
                          </Show>
                        }
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
                                    <InlineText text={r.text} format={formatForPage(r.page)} />
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
                  </>
                }
              >
                <Show when={groups() && groups()!.length > 0} fallback={<div class="query-empty">No results</div>}>
                  <Show when={(sheet()?.view === "table" || sheet()?.view === "board") && props.blockId}>
                    <SheetContainer>
                      <Switch>
                        <Match when={sheet()?.view === "table"}>
                          <SheetTable ownerId={props.blockId!} rowSource="query" groups={groups() ?? []} />
                        </Match>
                        <Match when={sheet()?.view === "board"}>
                          <SheetBoard ownerId={props.blockId!} rowSource="query" groupBy={sheet()?.groupBy} groups={groups() ?? []} />
                        </Match>
                      </Switch>
                    </SheetContainer>
                  </Show>
                </Show>
              </Show>
            </Show>
          </Match>
        </Switch>
      </div>
    </Show>
  );
}

// One page's query results, rendered as LIVE editable blocks. The result page
// is loaded into the shared working set on demand; each result is the same
// <Block> the main view uses (so editing a result edits the real block and
// saves to its page). Until the page is loaded, a read-only block stands in.
//
// Keyed by page name (outer <For>) and block uuid (inner <For>) so a reactive
// re-query that returns the same membership reuses the existing rows — it never
// re-mounts a block you're editing in a result and yanks the caret out.
function QueryGroup(props: { page: string; group: () => RefGroup | undefined; flat?: boolean }): JSX.Element {
  const kind = (): PageKind => props.group()?.kind ?? "page";
  return (
    <Show when={props.group()}>
      {(g) => (
        <div class="query-group" classList={{ "query-group-flat": props.flat }}>
          <div
            class={props.flat ? "query-crumb" : "query-page"}
            onClick={(e) => {
              e.stopPropagation();
              if (e.shiftKey) openPageInSidebar(props.page, kind());
              else openPage(props.page, kind());
            }}
            onAuxClick={(e) => {
              if (e.button === 1) {
                e.preventDefault();
                e.stopPropagation();
                openPageInNewTab(props.page, kind());
              }
            }}
            onContextMenu={(e) => {
              e.preventDefault();
              e.stopPropagation();
              openPageContextMenu(e.clientX, e.clientY, props.page, kind());
            }}
          >
            {props.page}
          </div>
          <LiveRefGroup page={props.page} kind={kind()} blocks={g().blocks} />
        </div>
      )}
    </Show>
  );
}

// A {{video}} / {{youtube}} / {{vimeo}} / {{bilibili}} macro: embeds YouTube,
// Vimeo or Bilibili as an iframe and direct media files as a <video>. Each of the
// provider-named macros also accepts a bare id (e.g. `{{vimeo 12345}}`), matching
// OG; the generic `{{video URL}}` sniffs the provider from the URL. Falls back to a
// link. (`youtube-timestamp` is a SEPARATE macro — handled before this one.)
export function VideoMacro(props: { body: string }): JSX.Element {
  const parsed = () => {
    const m = /^(\w+)\s*([\s\S]*)$/.exec(props.body.trim());
    const name = (m?.[1] ?? "video").toLowerCase();
    const arg = (m?.[2] ?? "").trim().replace(/^\[\[|\]\]$/g, "");
    return { name, arg };
  };
  const url = () => parsed().arg;
  const embed = () => {
    const { name, arg } = parsed();
    const yt = /(?:youtube\.com\/(?:watch\?v=|embed\/)|youtu\.be\/)([\w-]{11})/.exec(arg);
    if (yt) return `https://www.youtube.com/embed/${yt[1]}`;
    if (name === "youtube" && /^[\w-]{11}$/.test(arg)) return `https://www.youtube.com/embed/${arg}`;
    const vimeo = /vimeo\.com\/(\d+)/.exec(arg);
    if (vimeo) return `https://player.vimeo.com/video/${vimeo[1]}`;
    if (name === "vimeo" && /^\d+$/.test(arg)) return `https://player.vimeo.com/video/${arg}`;
    const bili = /bilibili\.com\/video\/(BV[0-9A-Za-z]+)/i.exec(arg);
    const bvid = bili ? bili[1] : name === "bilibili" && /^BV[0-9A-Za-z]+$/.test(arg) ? arg : null;
    if (bvid) return `https://player.bilibili.com/player.html?bvid=${bvid}&high_quality=1`;
    return null;
  };
  return (
    <Show
      when={embed()}
      fallback={
        <Show
          when={/\.(mp4|webm|ogg)(\?|$)/i.test(url())}
          fallback={<a class="external-link" href={url()} target="_blank" rel="noreferrer">{url()}</a>}
        >
          <video class="embed-video" src={url()} controls />
        </Show>
      }
    >
      <div class="embed-iframe-wrap">
        <iframe class="embed-iframe" src={embed()!} allowfullscreen title="video" />
      </div>
    </Show>
  );
}

// A {{tweet URL}} / {{twitter URL}} macro (`twitter` is OG's alias for `tweet`) —
// rendered as a link (no third-party script embedding).
export function TweetMacro(props: { body: string }): JSX.Element {
  const url = () => props.body.replace(/^(tweet|twitter)\s*/i, "").trim();
  return (
    <a class="external-link tweet-link" href={url()} target="_blank" rel="noreferrer">
      🐦 {url()}
    </a>
  );
}

// `{{youtube-timestamp <seconds>}}` — OG makes this seek a sibling on-page YouTube
// player via the IFrame Player API. Tine embeds YouTube as a bare <iframe> with no
// player handle, so there's nothing to drive; we degrade to a styled, formatted
// timestamp label (m:ss / h:mm:ss). Flagged as a known parity gap.
export function YoutubeTimestamp(props: { body: string }): JSX.Element {
  const secs = () => {
    const raw = props.body.replace(/^youtube-timestamp\s*/i, "").trim();
    const n = parseInt(raw, 10);
    return Number.isFinite(n) ? n : 0;
  };
  const label = () => {
    const s = Math.max(0, secs());
    const h = Math.floor(s / 3600);
    const m = Math.floor((s % 3600) / 60);
    const sec = s % 60;
    const pad = (x: number) => String(x).padStart(2, "0");
    return h > 0 ? `${h}:${pad(m)}:${pad(sec)}` : `${m}:${pad(sec)}`;
  };
  return (
    <span class="youtube-ts" title="YouTube-player seeking isn't supported yet (the embed is a bare iframe)">
      ⏱ {label()}
    </span>
  );
}

// `{{cloze answer}}` (optionally `{{cloze answer\\cue}}`) — in OG this is hidden
// only inside the SRS flashcard-review loop. Tine has no SRS engine, so we degrade
// to a click-to-reveal: shows the cue (or `[...]`) until clicked, then the answer.
export function ClozeMacro(props: { body: string }): JSX.Element {
  const [revealed, setRevealed] = createSignal(false);
  const parts = () => props.body.replace(/^cloze\s*/i, "").trim().split(/\\\\/);
  const answer = () => (parts()[0] ?? "").trim();
  const cue = () => parts()[1]?.trim();
  return (
    <span
      class="cloze"
      classList={{ revealed: revealed() }}
      title={revealed() ? "Click to hide" : "Click to reveal"}
      onClick={(e) => {
        e.stopPropagation();
        setRevealed((v) => !v);
      }}
    >
      {revealed() ? answer() : (cue() ?? "[...]")}
    </span>
  );
}

// `{{zotero-imported-file ...}}` / `{{zotero-linked-file ...}}` — OG resolves the
// Zotero item-key to a real attachment via its Zotero connector (data dir + item
// metadata + storage config). Tine has no Zotero integration, so resolving would
// yield a dead link; we degrade to a muted, non-navigating label rather than a
// broken link. Flagged as a known parity gap (niche).
export function ZoteroMacro(props: { body: string }): JSX.Element {
  const arg = () => props.body.replace(/^zotero-(imported|linked)-file\s*/i, "").trim();
  return (
    <span class="zotero-ref" title="Zotero integration isn't supported in Tine">
      📎 {arg() || "Zotero attachment"}
    </span>
  );
}

// A {{embed ((uuid))}} or {{embed [[Page]]}} block.
export function EmbedMacro(props: { body: string }): JSX.Element {
  const target = () => props.body.replace(/^embed\s*/i, "").trim();

  const [data] = createResource(
    () => `${target()} ${graphEpoch()}`,
    async () => {
    const t = target();
    const blockRef = /^\(\(([^)]+)\)\)$/.exec(t);
    if (blockRef) {
      const g = await resolveBlockBatched(blockRef[1]);
      // embedId = the embedded block's own id, so its ref-count badge is hidden
      // inside the embed (OG hide-block-refs-count?); its children keep theirs.
      return g ? { page: g.page, kind: g.kind, blocks: g.blocks, embedId: g.blocks[0]?.id } : null;
    }
    const pageRef = /^\[\[([^\]]+)\]\]$/.exec(t);
    if (pageRef) {
      // Backend miss → the virtual in-app Guide, matched by bare title (the embed
      // carries no source context to remap the name). No-op for real graphs.
      const p = (await backend().getPage(pageRef[1], "page")) ?? resolveGuidePageDto(pageRef[1]);
      return p ? { page: p.name, kind: "page" as PageKind, blocks: p.blocks, embedId: undefined } : null;
    }
    return null;
  });

  return (
    <div class="embed-block">
      <Show when={data()} fallback={<div class="embed-missing">{`{{${props.body}}}`}</div>}>
        <LiveRefGroup page={data()!.page} kind={data()!.kind} blocks={data()!.blocks} embedId={data()!.embedId} />
      </Show>
    </div>
  );
}
