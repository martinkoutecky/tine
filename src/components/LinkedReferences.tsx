import { For, Show, createResource, createSignal, type JSX } from "solid-js";
import { backend } from "../backend";
import { openPage, openPageInNewTab } from "../router";
import { openPageInSidebar, openPageContextMenu } from "../ui";
import { LiveRefGroup } from "./LiveRefGroup";

// The "Linked References" section at the bottom of a page (backlinks). Renders
// live, editable blocks (edits save to the source page) and is collapsible.
export function LinkedReferences(props: { name: string }): JSX.Element {
  const [groups] = createResource(
    () => props.name,
    (n) => backend().getBacklinks(n)
  );
  const [collapsed, setCollapsed] = createSignal(false);
  const count = () => groups()?.reduce((acc, g) => acc + g.blocks.length, 0) ?? 0;

  return (
    <Show when={groups() && groups()!.length > 0}>
      <div class="linked-references">
        <div class="references-header" onClick={() => setCollapsed(!collapsed())}>
          <span class="ref-collapse" classList={{ collapsed: collapsed() }}>
            <svg viewBox="0 0 24 24" class="triangle">
              <path d="M8 5l8 7-8 7z" />
            </svg>
          </span>
          Linked References <span class="references-count">{count()}</span>
        </div>
        <Show when={!collapsed()}>
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
        </Show>
      </div>
    </Show>
  );
}
