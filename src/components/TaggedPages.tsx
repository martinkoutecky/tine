import { t } from "../i18n";
import { For, Show, createResource, createSignal, type JSX } from "solid-js";
import { backend } from "../backend";
import { changed } from "../changeKeys";
import { referenceKey } from "../referenceKeys";
import { graphOwner, latestOwner, readOwned } from "../owned";
import { isPublishedExport } from "../publishedBackend";
import { readOr } from "../resourceRead";
import { PageRef } from "../render/inline";
import { ResourceFailure } from "./ResourceFailure";
import { ReferenceDisclosure } from "./ReferenceDisclosure";

/** OG page.cljs tagged-pages: list pages whose page tags contain this name or,
 * unlike OG (GH #715), any name in its alias group, the same group Linked
 * References use; a page tagging itself so is listed too, as in OG. The backend owns membership
 * (`tagged_pages`: normalization, Org, aliases); `(page-tags …)` queries stay
 * literal. Cost: one page scan per relevant tag change, O(pages), plus O(matches log
 * matches) display sorting. Read failures render a retry; stale graph reads are
 * discarded. Published exports have no live query engine and omit the section. */
export function TaggedPages(props: { name: string }): JSX.Element {
  const owners = {};
  const [collapsed, setCollapsed] = createSignal(false);
  const [answer, { refetch }] = createResource(
    () => isPublishedExport() ? null : `${props.name}\0${changed(referenceKey("tags", props.name))}`,
    async () => {
      const owner = latestOwner(owners, "tagged-pages", graphOwner());
      const result = await readOwned(owner, backend().taggedPages(props.name));
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
          <ReferenceDisclosure collapsed={collapsed()} />{t("tagged-pages-heading", { name: props.name })}
        </button>
        <Show when={!collapsed()}>
          <ul><For each={rows()?.pages}>{page => <li><PageRef name={page.name} alias={page.name} /></li>}</For></ul>
          <Show when={rows()?.exceeded}><div role="status">{t("tagged-pages-some-tagged-pages-are-omitted-because-the-query")}</div></Show>
        </Show>
      </section>
    </Show>
  </>;
}
