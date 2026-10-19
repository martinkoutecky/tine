// The sidebar's Tabs section (GH #704 v1, D-20261018-W1): the open tabs of the
// focused pane as a list at every width, so a window too narrow for the top
// bar's strip (a phone, a small window) can still see and switch tabs, and a
// background tab opened with "Open in new tab" is reachable. v1 is list,
// active highlight, switch and close; nesting, drag-reorder and a persisted
// tab tree are a later card. No persisted state: the section's expansion is
// session-local, so there is no session-format change.
import { For, Show, createSignal, type JSX } from "solid-js";
import { t } from "../i18n";
import { EmojiText } from "../render/emoji";
import { focusedRouterOf } from "../panes";
import { MAIN_WINDOW_ID } from "../windowRealm";
import { closeOffered, tabFullTitle, tabTitle } from "./TabBar";
import "../styles/sidebarTabs.css";

const [tabsSectionExpanded, setTabsSectionExpanded] = createSignal(true);

export function SidebarTabs(props: { onActiveNavigationComplete?: () => void }): JSX.Element {
  // The focused pane of the main window; the sidebar exists only there.
  const router = () => focusedRouterOf(MAIN_WINDOW_ID);
  return (
    // One tab is not worth a section; it appears when there is something to
    // switch to (the top bar's strip also has its single tab, but it is free).
    <Show when={router().tabs().length > 1}>
      <div class="nav-section" data-sidebar-section="tabs">
        <button
          type="button"
          class="nav-section-header nav-section-toggle"
          data-sidebar-section-toggle="tabs"
          aria-expanded={tabsSectionExpanded()}
          aria-controls="sidebar-tabs-list"
          onClick={() => setTabsSectionExpanded((open) => !open)}
        >
          <span class="nav-toggle-caret" classList={{ open: tabsSectionExpanded() }}>▸</span>{t("tab-bar-open-tabs")}<span class="nav-section-count">{router().tabs().length}</span>
        </button>
        <Show when={tabsSectionExpanded()}>
          <div id="sidebar-tabs-list" role="list">
            <For each={router().tabs()}>
              {(tab) => (
                <div
                  class="nav-page nav-tab"
                  classList={{ active: tab.id === router().activeId(), pinned: tab.pinned }}
                  role="listitem"
                  data-tab-id={tab.id}
                  onAuxClick={(event) => {
                    if (event.button !== 1) return;
                    event.preventDefault();
                    void router().closeTab(tab.id);
                  }}
                >
                  <button
                    type="button"
                    class="nav-tab-main"
                    aria-current={tab.id === router().activeId() ? "true" : undefined}
                    title={String(tabFullTitle(router().tabRoute(tab)))}
                    onClick={() => {
                      router().setActiveTab(tab.id);
                      props.onActiveNavigationComplete?.();
                    }}
                  >
                    <Show when={tab.pinned}><span class="nav-tab-pin"><EmojiText text="📌" /></span></Show>
                    <span class="nav-tab-title"><EmojiText text={tabTitle(router().tabRoute(tab))} /></span>
                  </button>
                  <Show when={closeOffered(router(), tab)}>
                    <button
                      type="button"
                      class="nav-tab-close"
                      title={t("tab-bar-close-tab")}
                      aria-label={t("tab-bar-close", { title: String(tabFullTitle(router().tabRoute(tab))) })}
                      onClick={(event) => {
                        event.stopPropagation();
                        void router().closeTab(tab.id);
                      }}
                    >✕</button>
                  </Show>
                </div>
              )}
            </For>
          </div>
        </Show>
      </div>
    </Show>
  );
}
