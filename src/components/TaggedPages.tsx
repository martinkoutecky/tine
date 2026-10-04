import { For, Show, createResource, createSignal, type JSX } from "solid-js";
import { backend } from "../backend";
import { dataRev } from "../graphSession";
import { graphOwner, latestOwner, readOwned } from "../owned";
import { isPublishedExport } from "../publishedBackend";
import { readOr } from "../resourceRead";
import { PageRef } from "../render/inline";
import { ResourceFailure } from "./ResourceFailure";

/** OG page.cljs tagged-pages: list pages whose page tags contain this name.
 * The existing query door owns membership (including normalization and Org).
 * Cost: one page-tags query per graph revision, O(pages), plus O(matches log
 * matches) display sorting. Read failures render a retry; stale graph reads are
 * discarded. Published exports have no live query engine and omit the section. */
export function TaggedPages(props: { name: string }): JSX.Element {
  const owners = {};
  const [collapsed, setCollapsed] = createSignal(false);
  const [answer, { refetch }] = createResource(
    () => isPublishedExport() ? null : `${props.name}\0${dataRev()}`,
    async () => {
      const owner = latestOwner(owners, "tagged-pages", graphOwner());
      const tag = JSON.stringify(props.name);
      const parsed = await readOwned(owner, backend().parseQuery(`(page-tags ${tag})`, "macro_query"));
      if (parsed.kind === "stale") return undefined;
      const result = await readOwned(owner, backend().queryRun(parsed.value.query, parsed.value.view));
      if (result.kind === "stale") return undefined;
      const diagnostic = result.value.diagnostics?.find(item => !item.disabled);
      if (diagnostic) throw new Error(diagnostic.message);
      if (result.value.anchor !== "page") throw new Error("Expected tagged pages");
      return { ...result.value, pages: [...result.value.pages].sort((a, b) => a.name.toLowerCase().localeCompare(b.name.toLowerCase())) };
    },
  );
  const rows = () => readOr(answer, undefined, "tagged pages");
  return <>
    <ResourceFailure of={answer} what="tagged pages" onRetry={() => void refetch()} />
    <Show when={rows()?.pages.length}>
      <section class="tagged-pages linked-references">
        <button type="button" class="references-header" aria-expanded={!collapsed()} onClick={() => setCollapsed(!collapsed())}>
          <span aria-hidden="true">{collapsed() ? "▸" : "▾"}</span> Pages tagged with "{props.name}"
        </button>
        <Show when={!collapsed()}>
          <ul><For each={rows()?.pages}>{page => <li><PageRef name={page.name} alias={page.name} /></li>}</For></ul>
          <Show when={rows()?.exceeded}><div role="status">Some tagged pages are omitted because the query result limit was reached.</div></Show>
        </Show>
      </section>
    </Show>
  </>;
}
