import { For, Show, type JSX } from "solid-js";
import type { QueryPublicationPlan } from "../types";
import { pageIdentityKey } from "../pageIdentity";
import { t } from "../i18n";

/** The prior snapshot is only preview evidence; publication still checks the source fingerprint. */
export function FolderExportMembership(props: { plan: QueryPublicationPlan }): JSX.Element {
  const diff = () => {
    const previous = props.plan.previousPages;
    if (!previous) return null;
    const before = new Set(previous.map(pageIdentityKey));
    const current = props.plan.pages.map(page => page.name);
    const after = new Set(current.map(pageIdentityKey));
    return { added: current.filter(name => !before.has(pageIdentityKey(name))), removed: previous.filter(name => !after.has(pageIdentityKey(name))) };
  };
  return <Show when={diff()}>{changes => <div class="folder-export-membership">
    <p>{t("quick-share-membership", { added: changes().added.length, removed: changes().removed.length })}</p>
    <details><summary>{t("quick-share-membership-pages")}</summary><ul>
      <For each={changes().added}>{name => <li>+ {name}</li>}</For>
      <For each={changes().removed}>{name => <li>− {name}</li>}</For>
    </ul></details>
  </div>}</Show>;
}
