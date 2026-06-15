import { For, Show, Switch, Match, createMemo, createResource, createSignal, type JSX } from "solid-js";
import { backend } from "../backend";
import { openPage } from "../router";
import { RefBlocks } from "./RefBlocks";
import { blockView } from "../render/block";
import { InlineText } from "../render/inline";
import type { PageKind } from "../types";

const ADVANCED_RE = /\[\s*:find|:where|:find/;

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
            Query <span class="query-count">{total()}</span>
            <button class="query-view-toggle" onClick={() => setTable(!table())}>
              {table() ? "List" : "Table"}
            </button>
          </div>
          <Show
            when={groups() && groups()!.length > 0}
            fallback={<div class="query-empty">No results</div>}
          >
            <Show
              when={table()}
              fallback={
                <For each={groups()}>
                  {(g) => (
                    <div class="query-group">
                      <div class="query-page" onClick={() => openPage(g.page, g.kind)}>
                        {g.page}
                      </div>
                      <RefBlocks blocks={g.blocks} />
                    </div>
                  )}
                </For>
              }
            >
              <table class="md-table query-table">
                <thead>
                  <tr>
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
                        <td class="qt-page" onClick={() => openPage(r.page, r.kind)}>
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
        </Match>
      </Switch>
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
        <RefBlocks blocks={data()!.blocks} />
      </Show>
    </div>
  );
}
