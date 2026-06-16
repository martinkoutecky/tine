import { For, Show, createResource, type JSX } from "solid-js";
import { backend } from "../backend";
import { openPage } from "../router";
import { RefBlocks } from "./RefBlocks";

// The "Linked References" section at the bottom of a page (backlinks).
export function LinkedReferences(props: { name: string }): JSX.Element {
  const [groups] = createResource(
    () => props.name,
    (n) => backend().getBacklinks(n)
  );

  const count = () => groups()?.reduce((acc, g) => acc + g.blocks.length, 0) ?? 0;

  return (
    <Show when={groups() && groups()!.length > 0}>
      <div class="linked-references">
        <div class="references-header">
          Linked References <span class="references-count">{count()}</span>
        </div>
        <For each={groups()}>
          {(g) => (
            <div class="reference-group">
              <div class="reference-page" onClick={() => openPage(g.page, g.kind)}>
                {g.page}
              </div>
              <div class="reference-blocks">
                <RefBlocks blocks={g.blocks} page={g.page} pageKind={g.kind} />
              </div>
            </div>
          )}
        </For>
      </div>
    </Show>
  );
}
