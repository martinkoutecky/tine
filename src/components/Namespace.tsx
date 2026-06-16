import { For, Show, createResource, type JSX } from "solid-js";
import { backend } from "../backend";
import { openPage } from "../router";
import { graphEpoch } from "../ui";

// Namespace hierarchy for a page named `a/b/c`: a clickable breadcrumb of the
// ancestor namespaces (shown above the title) and a list of direct child pages
// (shown below the page). Mirrors OG's hierarchy component.

/** Breadcrumb of ancestor namespaces, e.g. for "a/b/c" → a › b (clickable). */
export function NamespaceCrumb(props: { name: string }): JSX.Element {
  const parts = () => props.name.split("/");
  return (
    <Show when={parts().length > 1}>
      <div class="ns-crumb">
        <For each={parts().slice(0, -1)}>
          {(_, i) => {
            const prefix = () => parts().slice(0, i() + 1).join("/");
            return (
              <>
                <span class="ns-crumb-item" onClick={() => openPage(prefix(), "page")}>
                  {parts()[i()]}
                </span>
                <span class="ns-crumb-sep">/</span>
              </>
            );
          }}
        </For>
      </div>
    </Show>
  );
}

/** Direct child pages of a namespace (pages named `<name>/<segment>`). */
export function NamespaceChildren(props: { name: string }): JSX.Element {
  const [children] = createResource(
    () => ({ n: props.name, e: graphEpoch() }),
    async ({ n }) => {
      const prefix = `${n}/`.toLowerCase();
      const all = await backend().listPages();
      const seen = new Set<string>();
      const direct: { name: string; full: string }[] = [];
      for (const p of all) {
        if (!p.name.toLowerCase().startsWith(prefix)) continue;
        const rest = p.name.slice(n.length + 1);
        const seg = rest.split("/")[0];
        const full = `${n}/${seg}`;
        if (!seen.has(full.toLowerCase())) {
          seen.add(full.toLowerCase());
          direct.push({ name: seg, full });
        }
      }
      return direct.sort((a, b) => a.name.localeCompare(b.name));
    }
  );
  return (
    <Show when={(children() ?? []).length > 0}>
      <div class="namespace-children">
        <div class="references-header">Namespace</div>
        <For each={children()}>
          {(c) => (
            <div class="ns-child" onClick={() => openPage(c.full, "page")}>
              {c.name}
            </div>
          )}
        </For>
      </div>
    </Show>
  );
}
