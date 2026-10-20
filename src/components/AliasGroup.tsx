import { t } from "../i18n";
import { For, Show, createMemo, createResource, type JSX } from "solid-js";
import { backend } from "../backend";
import { graphEpoch, pageInventoryRev } from "../graphSession";
import { graphOwner, latestOwner, readOwned } from "../owned";
import { resolvedTarget } from "../pageIndex";
import { pageIdentityKey } from "../pageIdentity";
import { readOr } from "../resourceRead";
import { PageRef } from "../render/inline";

/** The page header's alias group (D-20261019-AL1/AL2, card hB9Zjw): an alias
 * group is one thing, so the header tells "the names this thing has" from
 * "the real pages in this group". **Also called** lists the names with no file
 * of their own; a link to one opens the page declaring it (with several, the
 * one whose file path sorts first: `Resolved::Alias`). The second row lists
 * the other names that have their own file; a link to one opens that file.
 * Blocks are never merged. The backend owns the group (`page_alias_group`,
 * transitive over every page's `alias::`); this page's own declared names
 * show at once while an edit is still publishing, sorted into the two rows by
 * the page index. Both rows sort case-insensitively. A page outside any group renders nothing. Refetched when
 * the name inventory changes (an alias edit anywhere changes it), not on
 * ordinary saves: one O(group) IPC per mounted page and inventory change.
 * A bundled Guide page is not in the graph (`inGraph` false): it shows only
 * its own names. */
export function AliasGroupHeader(props: {
  name: string;
  declared: string[];
  inGraph: boolean;
  /** Click on the names row outside a link (the main pane edits the header). */
  onEdit?: (event: MouseEvent) => void;
}): JSX.Element {
  const owners = {};
  const [answer] = createResource(
    () => props.inGraph && `${props.name}\0${graphEpoch()}\0${pageInventoryRev()}`,
    async () => {
      const owner = latestOwner(owners, "alias-group", graphOwner());
      const result = await readOwned(owner, backend().pageAliasGroup(props.name));
      return result.kind === "stale" ? undefined : result.value;
    },
  );
  const rows = createMemo(() => {
    const group = readOr(answer, undefined, "alias group");
    const seen = new Set([pageIdentityKey(props.name)]);
    const aliases: string[] = [];
    const pages: string[] = [];
    const add = (into: string[], name: string) => {
      const key = pageIdentityKey(name);
      if (!key || seen.has(key)) return;
      seen.add(key);
      into.push(name);
    };
    for (const name of props.declared) add(resolvedTarget(name)?.kind === "existing" ? pages : aliases, name);
    for (const name of group?.aliases ?? []) add(aliases, name);
    for (const name of group?.pages ?? []) add(pages, name);
    // One order for the whole group, whichever member's page shows it.
    const byName = (a: string, b: string) => a.toLowerCase().localeCompare(b.toLowerCase());
    return { aliases: aliases.sort(byName), pages: pages.sort(byName) };
  });
  return <>
    <Show when={rows().aliases.length}>
      <div class="page-aliases" data-alias-names title={t("page-ui-also-called-title")} onClick={props.onEdit}>
        <span class="page-aliases-label">{t("page-ui-also-called")}</span>
        <For each={rows().aliases}>
          {(name) => <span class="alias-chip"><PageRef name={name} alias={name} /></span>}
        </For>
      </div>
    </Show>
    <Show when={rows().pages.length}>
      <div class="page-aliases page-alias-group" data-alias-group-pages title={t("page-ui-alias-group-pages-title")}>
        <span class="page-aliases-label">{t("page-ui-alias-group-pages")}</span>
        <For each={rows().pages}>
          {(name) => <span class="alias-group-page"><PageRef name={name} alias={name} /></span>}
        </For>
      </div>
    </Show>
  </>;
}
