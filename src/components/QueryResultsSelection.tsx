import { For, Show, createMemo, createSignal, type JSX } from "solid-js";
import { t } from "../i18n";
import type { QueryHit, RefGroup } from "../types";
import { openQueryResultsCopy } from "./queryResultsCopy";

/** A frozen selection of displayed hits. Refreshing answers cannot silently
 * replace the user's selection; closing and reopening takes a new snapshot. */
export function QueryResultsSelection(props: { hits: QueryHit[] }): JSX.Element {
  const [snapshot, setSnapshot] = createSignal<QueryHit[] | null>(null);
  const [selected, setSelected] = createSignal<Set<QueryHit>>(new Set());
  const chosen = createMemo(() => (snapshot() ?? []).filter(hit => selected().has(hit)));
  const copy = () => {
    const groups = new Map<string, RefGroup>();
    const pages = [];
    for (const hit of chosen()) {
      if (hit.entity === "page") { pages.push(hit.page); continue; }
      const key = `${hit.kind}\0${hit.path ?? hit.page}`;
      let group = groups.get(key);
      if (!group) { group = { page: hit.page, kind: hit.kind, path: hit.path, blocks: [] }; groups.set(key, group); }
      group.blocks.push(hit.block);
    }
    void openQueryResultsCopy([...groups.values()], pages);
    setSnapshot(null);
  };
  return <div class="query-results-selection">
    <button type="button" disabled={!props.hits.length} onClick={() => {
      const hits = [...props.hits]; setSnapshot(hits); setSelected(new Set(hits));
    }}>{t("query-calm-select")}</button>
    <Show when={snapshot()}>{(hits) => <div role="group" aria-label={t("query-calm-select")}>
      <button type="button" onClick={() => setSelected(new Set(hits()))}>{t("query-calm-all")}</button>
      <button type="button" onClick={() => setSelected(new Set())}>{t("query-calm-none")}</button>
      <For each={hits()}>{hit => <label class="query-copy-row">
        <input type="checkbox" aria-label={t("query-calm-select-result")} checked={selected().has(hit)} onChange={event => setSelected(current => {
          const next = new Set(current); if (event.currentTarget.checked) next.add(hit); else next.delete(hit); return next;
        })} />
        {hit.entity === "page" ? hit.page.name : `${hit.page}: ${hit.display_text}`}
      </label>}</For>
      <button type="button" disabled={!chosen().length} onClick={copy}>{t("query-calm-copy")}</button>
      <button type="button" onClick={() => setSnapshot(null)}>{t("query-calm-done")}</button>
    </div>}</Show>
  </div>;
}
