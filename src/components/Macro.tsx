import { For, Show, Switch, Match, createMemo, createResource, createSignal, type JSX } from "solid-js";
import { backend } from "../backend";
import { openPage, openPageInNewTab } from "../router";
import { openPageInSidebar, openPageContextMenu, dataRev } from "../ui";
import { setRaw, doc } from "../store";
import { LiveRefGroup } from "./LiveRefGroup";
import { QueryBuilder } from "./QueryBuilder";
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
  const m = /\{[^{}]*\}\s*$/.exec(arg);
  if (!m) return { form: arg.trim(), opts: "" };
  const opts = m[0].trim();
  return {
    form: arg.slice(0, m.index).trim(),
    opts,
    title: /:title\s+"([^"]*)"/.exec(opts)?.[1],
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

  // Rewrite just the {{query ...}} macro inside the owning block, preserving the
  // front-matter options and surrounding property lines (id::/collapsed::).
  const applyDsl = (dsl: string) => {
    if (!props.blockId) return;
    const raw = doc.byId[props.blockId]?.raw ?? "";
    const opts = parsed().opts ? ` ${parsed().opts}` : "";
    const next = raw.replace(/\{\{query\b[\s\S]*?\}\}/i, () => `{{query ${dsl}${opts}}}`);
    setRaw(props.blockId, next);
  };
  // Re-run when the query text changes OR after any save lands (dataRev), so
  // results track edits live — e.g. a task flipped to DONE leaves a (task TODO)
  // query. createResource keeps the previous value during refetch (no flicker).
  const [groups] = createResource(
    () => `${form()} ${dataRev()}`,
    () => backend().runQuery(form())
  );
  const total = () => groups()?.reduce((a, g) => a + g.blocks.length, 0) ?? 0;
  const [table, setTable] = createSignal(parsed().tableView ?? false);
  const [sortCol, setSortCol] = createSignal<string>("");
  const [sortDir, setSortDir] = createSignal(1);
  const [collapsed, setCollapsed] = createSignal(loadCollapsed(arg()) || (parsed().collapsed ?? false));
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

  // Hide the whole block when asked and there's nothing to show (advanced
  // queries still render their "unsupported" notice).
  const hidden = () => props.hideWhenEmpty && !ADVANCED_RE.test(arg()) && total() === 0;

  return (
    <Show when={!hidden()}>
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
            {props.title ?? parsed().title ?? "Query"} <span class="query-count">{total()}</span>
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
          <Show when={props.blockId}>
            <QueryBuilder dsl={form} onChange={applyDsl} />
          </Show>
          <Show when={!collapsed()}>
          <Show
            when={groups() && groups()!.length > 0}
            fallback={<div class="query-empty">No results</div>}
          >
            <Show
              when={table()}
              fallback={
                <For each={(groups() ?? []).map((g) => g.page)}>
                  {(pageName) => (
                    <QueryGroup
                      page={pageName}
                      group={() => (groups() ?? []).find((g) => g.page === pageName)}
                    />
                  )}
                </For>
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
function QueryGroup(props: { page: string; group: () => RefGroup | undefined }): JSX.Element {
  const kind = (): PageKind => props.group()?.kind ?? "page";
  return (
    <Show when={props.group()}>
      {(g) => (
        <div class="query-group">
          <div
            class="query-page"
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

// A {{video URL}} / {{youtube URL}} macro: embeds YouTube/Vimeo as an iframe and
// direct media files as a <video>. Falls back to a link.
export function VideoMacro(props: { body: string }): JSX.Element {
  const url = () => props.body.replace(/^(video|youtube)\s*/i, "").trim().replace(/^\[\[|\]\]$/g, "");
  const embed = () => {
    const u = url();
    const yt = /(?:youtube\.com\/(?:watch\?v=|embed\/)|youtu\.be\/)([\w-]{11})/.exec(u);
    if (yt) return `https://www.youtube.com/embed/${yt[1]}`;
    const vimeo = /vimeo\.com\/(\d+)/.exec(u);
    if (vimeo) return `https://player.vimeo.com/video/${vimeo[1]}`;
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

// A {{tweet URL}} macro — rendered as a link (no third-party script embedding).
export function TweetMacro(props: { body: string }): JSX.Element {
  const url = () => props.body.replace(/^tweet\s*/i, "").trim();
  return (
    <a class="external-link tweet-link" href={url()} target="_blank" rel="noreferrer">
      🐦 {url()}
    </a>
  );
}

// A {{embed ((uuid))}} or {{embed [[Page]]}} block.
export function EmbedMacro(props: { body: string }): JSX.Element {
  const target = () => props.body.replace(/^embed\s*/i, "").trim();

  const [data] = createResource(target, async (t) => {
    const blockRef = /^\(\(([^)]+)\)\)$/.exec(t);
    if (blockRef) {
      const g = await backend().resolveBlock(blockRef[1]);
      return g ? { page: g.page, kind: g.kind, blocks: g.blocks } : null;
    }
    const pageRef = /^\[\[([^\]]+)\]\]$/.exec(t);
    if (pageRef) {
      const p = await backend().getPage(pageRef[1], "page");
      return p ? { page: p.name, kind: "page" as PageKind, blocks: p.blocks } : null;
    }
    return null;
  });

  return (
    <div class="embed-block">
      <Show when={data()} fallback={<div class="embed-missing">{`{{${props.body}}}`}</div>}>
        <LiveRefGroup page={data()!.page} kind={data()!.kind} blocks={data()!.blocks} />
      </Show>
    </div>
  );
}
