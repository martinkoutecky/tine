import { For, Show, type JSX } from "solid-js";
import {
  tabs,
  activeId,
  setActiveTab,
  closeTab,
  togglePin,
  reorderTab,
  routeTitle,
} from "../router";

// Tab strip: click to activate, middle-click to close, double-click to pin
// (pinned tabs persist across sessions), drag to reorder.
export function TabBar(): JSX.Element {
  let dragId: string | null = null;
  const visible = () => tabs().length > 1 || tabs().some((t) => t.pinned);

  return (
    <Show when={visible()}>
      <div class="tab-bar">
        <For each={tabs()}>
          {(t) => (
            <div
              class="tab"
              classList={{ active: t.id === activeId(), pinned: t.pinned }}
              draggable={true}
              onDragStart={() => (dragId = t.id)}
              onDragOver={(e) => e.preventDefault()}
              onDrop={() => {
                if (dragId) reorderTab(dragId, t.id);
                dragId = null;
              }}
              onClick={() => setActiveTab(t.id)}
              onDblClick={() => togglePin(t.id)}
              onAuxClick={(e) => {
                if (e.button === 1) {
                  e.preventDefault();
                  closeTab(t.id);
                }
              }}
              title={t.pinned ? "Pinned — double-click to unpin" : "Double-click to pin"}
            >
              <Show when={t.pinned}>
                <span class="tab-pin">📌</span>
              </Show>
              <span class="tab-title">{routeTitle(t.route)}</span>
              <span
                class="tab-close"
                onClick={(e) => {
                  e.stopPropagation();
                  closeTab(t.id);
                }}
              >
                ✕
              </span>
            </div>
          )}
        </For>
      </div>
    </Show>
  );
}
