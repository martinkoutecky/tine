import { For, Show, createResource, type JSX } from "solid-js";
import { backend } from "../backend";
import { doc, ensurePageLoaded, pageByName } from "../store";
import { Block } from "./Block";
import { RefBlocks } from "./RefBlocks";
import type { BlockDto, PageKind } from "../types";

// Renders a list of result/backlink/embed blocks as LIVE editable <Block>s: the
// source page is loaded into the shared working set on demand, then each block
// is the same component the main view uses (so editing edits the real block and
// saves to its page). Until the page is loaded, a read-only RefBlock stands in.
//
// Keyed by block uuid so a reactive refresh that returns the same membership
// reuses existing rows and never yanks the caret out of a block being edited.
export function LiveRefGroup(props: { page: string; kind: PageKind; blocks: BlockDto[] }): JSX.Element {
  const blockIds = () => props.blocks.map((b) => b.id);
  const dtoById = (id: string) => props.blocks.find((b) => b.id === id);
  const [ready] = createResource(
    () => ({ p: props.page, k: props.kind }),
    async ({ p, k }) => {
      if (!pageByName(p)) {
        const dto = await backend().getPage(p, k);
        if (dto) ensurePageLoaded(dto);
      }
      return true;
    }
  );
  return (
    <For each={blockIds()}>
      {(id) => (
        <Show
          when={ready() && doc.byId[id]}
          fallback={
            <Show when={dtoById(id)}>
              {(d) => <RefBlocks blocks={[d()]} page={props.page} pageKind={props.kind} />}
            </Show>
          }
        >
          <Block id={id} />
        </Show>
      )}
    </For>
  );
}
