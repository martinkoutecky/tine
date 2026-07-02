import { For, Show, createMemo, createSignal, type JSX } from "solid-js";
import { openJournals, openPage, openPageInNewTab, openInNewTab, route } from "../router";
import { openSwitcher, favorites, recentPages, openPageContextMenu } from "../ui";
import { allPages as allGraphPages } from "../pages";
import { NamespaceTree } from "./Namespace";
import type { PageKind } from "../types";

// Cap the rendered "All pages" list. Beyond this, rendering every row (each
// reading route() for its active state) makes both the initial render and every
// navigation O(pages); past a few hundred the list isn't scannable anyway, so we
// show the first N alphabetically and point the rest at search (Ctrl-K).
const ALL_PAGES_CAP = 300;

export function Sidebar(): JSX.Element {
  // The whole-graph page list is the shared, graph-epoch-keyed resource (see
  // src/pages.ts) — fetched once per graph generation and shared with the
  // namespace tree/macro/hierarchy, not a sidebar-private fetch. Epoch keying
  // still fixes the old "graph still loading at mount" race (it refetches when
  // the graph becomes ready).
  const [showAll, setShowAll] = createSignal(false);
  const [showNs, setShowNs] = createSignal(false);

  // Filter + sort ONCE per page-list change (memoized), not on every render /
  // navigation as the inline `.filter().sort()` in JSX did.
  const allPages = createMemo(() =>
    (allGraphPages() ?? [])
      .filter((p) => p.kind === "page")
      .sort((a, b) => a.name.localeCompare(b.name))
  );

  const isActive = (name: string) => {
    const r = route();
    return r.kind === "page" && r.name === name;
  };
  const openRowMenu = (e: MouseEvent, name: string, kind: PageKind) => {
    e.preventDefault();
    openPageContextMenu(e.clientX, e.clientY, name, kind);
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
          onClick={() => openJournals()}
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
                  onContextMenu={(e) => openRowMenu(e, fav.name, fav.kind)}
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
                  onContextMenu={(e) => openRowMenu(e, r.name, r.kind)}
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
            <Show when={allGraphPages()}>
              <span class="nav-section-count">{allPages().length}</span>
            </Show>
          </div>
          <Show when={showAll() && allGraphPages()}>
            <For each={allPages().slice(0, ALL_PAGES_CAP)}>
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
                  onContextMenu={(e) => openRowMenu(e, p.name, "page")}
                >
                  {p.name}
                </div>
              )}
            </For>
            <Show when={allPages().length > ALL_PAGES_CAP}>
              <div class="nav-page nav-page-more" onClick={openSwitcher}>
                +{allPages().length - ALL_PAGES_CAP} more — search to open…
              </div>
            </Show>
          </Show>
        </div>

        <div class="nav-section">
          <div
            class="nav-section-header nav-section-toggle"
            onClick={() => setShowNs(!showNs())}
          >
            <span class="nav-toggle-caret" classList={{ open: showNs() }}>▸</span>
            NAMESPACES
          </div>
          <Show when={showNs()}>
            <NamespaceTree onPageContextMenu={openRowMenu} />
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
