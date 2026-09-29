/** Page half of a friendly-search result. It receives already bounded and
 * ordered hits from graph search, renders O(returned rows), and only navigates.
 * The caller supplies the route operation; no file write or lookup occurs here. */
import { For, Match, Show, Switch, type JSX } from "solid-js";
import type { QueryHit } from "../types";
import type { QueryPresentation } from "../router";
import { buildSearchExcerpt } from "./SearchResultRow";

export type QueryPageHit = Extract<QueryHit, { entity: "page" }>;

/** Physical path distinguishes same-named files; virtual references use names.
 * Cost O(path or name length), with no failure for a valid page hit. */
export function pageHitKey(hit: QueryPageHit): string {
  return hit.page.path ? `page\0${hit.page.path}\0${hit.page.kind}` : `name\0${hit.page.name}`;
}

function PageText(props: { hit: QueryPageHit }): JSX.Element {
  const spans = () => props.hit.evidence
    .filter((item) => item.field === "page_name")
    .flatMap((item) => item.spans);
  return <For each={buildSearchExcerpt(props.hit.page.name, spans())}>{(part) => part.marked
    ? <mark>{part.text}</mark> : part.text}</For>;
}

/** Navigation-only page rows. Alias evidence labels its physical owner, and
 * content membership shows the matched block excerpt under that owner's name.
 * Order and bounds come from graph search. Rendering costs O(returned rows and
 * their displayed text); navigation failures belong to `onOpen`. */
export function QueryPageResults(props: {
  hits: QueryPageHit[];
  presentation: QueryPresentation;
  surfaceId: (hit: QueryPageHit) => string;
  onOpen: (hit: QueryPageHit) => void;
}): JSX.Element {
  const link = (hit: QueryPageHit) => <button
    type="button"
    class="query-result-row switcher-row"
    data-inpage-find-surface={props.surfaceId(hit)}
    onClick={() => props.onOpen(hit)}
  ><span class="switcher-kind">page</span><span class="search-result-body">
    <span class="search-result-context">{hit.page.kind === "journal" ? "Journal" : "Page"}</span>
    <span class="search-result-excerpt"><PageText hit={hit} /></span>
    <Show when={hit.evidence.some((item) => item.field === "visible_content")}>
      <span class="search-result-excerpt query-page-content-excerpt"><For each={buildSearchExcerpt(hit.display_text,
        hit.evidence.filter((item) => item.field === "visible_content").flatMap((item) => item.spans))}>
        {(part) => part.marked ? <mark>{part.text}</mark> : part.text}
      </For></span>
    </Show>
    <Show when={hit.matched_alias}><span class="query-page-alias">matched alias {hit.matched_alias}</span></Show>
  </span></button>;
  return <Switch>
    <Match when={props.presentation === "table"}>
      <div class="query-results-table-wrap"><table class="query-results-table">
        <caption class="sr-only">Page results</caption>
        <thead><tr><th scope="col">Page</th><th scope="col">Kind</th></tr></thead>
        <tbody><For each={props.hits}>{(hit) => <tr data-page-key={pageHitKey(hit)}>
          <td>{link(hit)}</td><td>{hit.page.kind === "journal" ? "Journal" : "Page"}</td>
        </tr>}</For></tbody>
      </table></div>
    </Match>
    <Match when={props.presentation === "board"}>
      <div class="query-results-board" aria-label="Page results grouped">
        <For each={props.hits}>{(hit) => <div class="query-board-card" data-page-key={pageHitKey(hit)}>{link(hit)}</div>}</For>
      </div>
    </Match>
    <Match when={props.presentation === "list"}>
      <ul class="query-results-list" aria-label="Page results">
        <For each={props.hits}>{(hit) => <li data-page-key={pageHitKey(hit)}>{link(hit)}</li>}</For>
      </ul>
    </Match>
    <Match when={props.presentation === "search"}>
      <div class="query-results-search" role="list" aria-label="Page results">
        <For each={props.hits}>{(hit) => <div role="listitem" data-page-key={pageHitKey(hit)}>{link(hit)}</div>}</For>
      </div>
    </Match>
  </Switch>;
}
