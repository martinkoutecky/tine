import { For, Show, createResource, type JSX } from "solid-js";
import { backend } from "../backend";
import { graphEpoch } from "../ui";
import { openPage, openPageInNewTab } from "../router";
import { openPageInSidebar, openPageContextMenu } from "../ui";
import { LiveRefGroup } from "./LiveRefGroup";

// Block-level "linked references": the blocks that reference THIS block (via
// `((uuid))` / `[..](((uuid)))` / `{{embed ((uuid))}}`), grouped by page. Toggled
// open by the per-block reference-count badge (Block.tsx). Mirrors the page-level
// LinkedReferences, minus the co-reference filter chips (OG doesn't show those on
// the block-ref panel). Refetches when the graph generation changes.
export function BlockReferences(props: { id: string }): JSX.Element {
  const [groups] = createResource(
    () => `${props.id} ${graphEpoch()}`,
    () => backend().getBlockReferrers(props.id)
  );
  const count = () => (groups() ?? []).reduce((acc, g) => acc + g.blocks.length, 0);

  return (
    <Show when={groups() && groups()!.length > 0}>
      <div class="block-references-inner">
        <div class="block-references-header">
          {count()} Linked Reference{count() === 1 ? "" : "s"}
        </div>
        <For each={groups()}>
          {(g) => (
            <div class="reference-group">
              <div
                class="reference-page"
                onClick={(e) => {
                  if (e.shiftKey) openPageInSidebar(g.page, g.kind);
                  else openPage(g.page, g.kind);
                }}
                onAuxClick={(e) => {
                  if (e.button === 1) {
                    e.preventDefault();
                    openPageInNewTab(g.page, g.kind);
                  }
                }}
                onContextMenu={(e) => {
                  e.preventDefault();
                  openPageContextMenu(e.clientX, e.clientY, g.page, g.kind);
                }}
              >
                {g.page}
              </div>
              <div class="reference-blocks">
                <LiveRefGroup page={g.page} kind={g.kind} blocks={g.blocks} />
              </div>
            </div>
          )}
        </For>
      </div>
    </Show>
  );
}
