import { For, Show, createResource, type JSX } from "solid-js";
import { backend } from "../backend";
import { openJournals, openPage, openPageInNewTab, openInNewTab, route } from "../router";
import { openSwitcher, favorites } from "../ui";

export function Sidebar(): JSX.Element {
  const [pages] = createResource(() => backend().listPages());

  const isActive = (name: string) => {
    const r = route();
    return r.kind === "page" && r.name === name;
  };

  return (
    <div class="left-sidebar-inner">
      <div class="sidebar-header">
        <div class="app-logo">Tine</div>
      </div>

      <div class="nav-search">
        <input
          class="search-input"
          type="text"
          placeholder="Search"
          readonly
          onClick={openSwitcher}
        />
      </div>

      <div class="nav-contents">
        <div
          class="nav-item"
          classList={{ active: route().kind === "journals" }}
          onClick={openJournals}
          onAuxClick={(e) => {
            if (e.button === 1) {
              e.preventDefault();
              openInNewTab({ kind: "journals" });
            }
          }}
        >
          <Icon name="journals" />
          <span>Journals</span>
        </div>

        <Show when={favorites().length > 0}>
          <div class="nav-section">
            <div class="nav-section-header">FAVORITES</div>
            <For each={favorites()}>
              {(name) => (
                <div
                  class="nav-page"
                  classList={{ active: isActive(name) }}
                  onClick={() => openPage(name, "page")}
                  onAuxClick={(e) => {
                    if (e.button === 1) {
                      e.preventDefault();
                      openPageInNewTab(name, "page");
                    }
                  }}
                >
                  ⭐ {name}
                </div>
              )}
            </For>
          </div>
        </Show>

        <div class="nav-section">
          <div class="nav-section-header">PAGES</div>
          <Show when={pages()}>
            <For each={pages()!.filter((p) => p.kind === "page")}>
              {(p) => (
                <div
                  class="nav-page"
                  classList={{ active: isActive(p.name) }}
                  onClick={() => openPage(p.name, "page")}
                  onAuxClick={(e) => {
                    if (e.button === 1) {
                      e.preventDefault();
                      openPageInNewTab(p.name, "page");
                    }
                  }}
                >
                  {p.name}
                </div>
              )}
            </For>
          </Show>
        </div>
      </div>

      <div class="sidebar-footer">
        <button class="new-page-btn">+ New page</button>
      </div>
    </div>
  );
}

function Icon(props: { name: string }): JSX.Element {
  // Minimal inline icons; the real Tabler set comes later.
  if (props.name === "journals") {
    return (
      <svg viewBox="0 0 24 24" class="nav-icon">
        <rect x="4" y="5" width="16" height="16" rx="2" fill="none" stroke="currentColor" stroke-width="1.6" />
        <line x1="4" y1="9" x2="20" y2="9" stroke="currentColor" stroke-width="1.6" />
        <line x1="9" y1="3" x2="9" y2="7" stroke="currentColor" stroke-width="1.6" />
        <line x1="15" y1="3" x2="15" y2="7" stroke="currentColor" stroke-width="1.6" />
      </svg>
    );
  }
  return <span />;
}
