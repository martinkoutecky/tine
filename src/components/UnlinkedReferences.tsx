import { For, Show, createResource, createSignal, type JSX } from "solid-js";
import { backend } from "../backend";
import { openPage } from "../router";
import { RefBlocks } from "./RefBlocks";

// "Unlinked References" — plain-text mentions of the page, collapsed by default.
export function UnlinkedReferences(props: { name: string }): JSX.Element {
  const [open, setOpen] = createSignal(false);
  const [groups] = createResource(
    () => (open() ? props.name : null),
    (n) => (n ? backend().getUnlinkedRefs(n) : Promise.resolve([]))
  );
  const count = () => groups()?.reduce((a, g) => a + g.blocks.length, 0) ?? 0;

  return (
    <div class="unlinked-references">
      <div class="references-header clickable" onClick={() => setOpen(!open())}>
        {open() ? "▾" : "▸"} Unlinked References
        <Show when={open() && groups()}>
          <span class="references-count">{count()}</span>
        </Show>
      </div>
      <Show when={open()}>
        <For each={groups()}>
          {(g) => (
            <div class="reference-group">
              <div class="reference-page" onClick={() => openPage(g.page, g.kind)}>
                {g.page}
              </div>
              <div class="reference-blocks">
                <RefBlocks blocks={g.blocks} page={g.page} />
              </div>
            </div>
          )}
        </For>
      </Show>
    </div>
  );
}
