import { For, Show, createResource, createSignal, type JSX } from "solid-js";
import { backend } from "../backend";
import { openJournals, openPage, openPageInNewTab, openInNewTab, route } from "../router";
import { openSwitcher, favorites, recentPages } from "../ui";

export function Sidebar(): JSX.Element {
  const [pages] = createResource(() => backend().listPages());
  const [showAll, setShowAll] = createSignal(false);

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
              {(fav) => (
                <div
                  class="nav-page"
                  classList={{ active: isActive(fav.name) }}
                  onClick={() => openPage(fav.name, fav.kind)}
                  onAuxClick={(e) => {
                    if (e.button === 1) {
                      e.preventDefault();
                      openPageInNewTab(fav.name, fav.kind);
                    }
                  }}
                >
                  ⭐ {fav.name}
                </div>
              )}
            </For>
          </div>
        </Show>

        <Show when={recentPages().length > 0}>
          <div class="nav-section">
            <div class="nav-section-header">RECENT</div>
            <For each={recentPages()}>
              {(r) => (
                <div
                  class="nav-page"
                  classList={{ active: isActive(r.name) }}
                  onClick={() => openPage(r.name, r.kind)}
                  onAuxClick={(e) => {
                    if (e.button === 1) {
                      e.preventDefault();
                      openPageInNewTab(r.name, r.kind);
                    }
                  }}
                >
                  {r.name.startsWith("hls__") ? r.name.slice(5) : r.name}
                </div>
              )}
            </For>
          </div>
        </Show>

        <div class="nav-section">
          <div
            class="nav-section-header nav-section-toggle"
            onClick={() => setShowAll(!showAll())}
          >
            <span class="nav-toggle-caret" classList={{ open: showAll() }}>▸</span>
            ALL PAGES
            <Show when={pages()}>
              <span class="nav-section-count">{pages()!.filter((p) => p.kind === "page").length}</span>
            </Show>
          </div>
          <Show when={showAll() && pages()}>
            <For each={pages()!.filter((p) => p.kind === "page").sort((a, b) => a.name.localeCompare(b.name))}>
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
