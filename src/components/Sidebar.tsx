import { For, Show, createEffect, createMemo, createSignal, onCleanup, type JSX } from "solid-js";
import { openJournals, openPage, openPageInNewTab, openFile, openInNewTab, route } from "../router";
import { openSwitcher, favorites, recentPages, openPageContextMenu, graphMeta, openPageInSidebar } from "../ui";
import { switchGraph, createNewGraph } from "../graph";
import { allPages as allGraphPages, pageListLabel } from "../pages";
import { EmojiText } from "../render/emoji";
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

  // `path` disambiguates nested pages that share a basename (#21 Phase 2). It's
  // optional: favorites / recent are keyed by name only, so they match on name
  // regardless of which file-path variant is currently routed.
  const isActive = (name: string, path?: string) => {
    const r = route();
    return (
      r.kind === "page" &&
      r.name === name &&
      (path === undefined || !r.path || r.path === path)
    );
  };
  const openEntry = (path: string, name: string) => {
    path ? openFile(path, name, "page") : openPage(name, "page");
  };
  // Shift+click on a sidebar page row opens it in the right sidebar (mirrors the
  // center-pane page-link behavior; GH #63). The onMouseDown guard suppresses the
  // browser's native shift-range text-selection (same fix as inline links, GH #42).
  const shiftGuard = (e: MouseEvent) => {
    if (e.shiftKey) e.preventDefault();
  };
  const openRowMenu = (e: MouseEvent, name: string, kind: PageKind) => {
    e.preventDefault();
    openPageContextMenu(e.clientX, e.clientY, name, kind);
  };

  return (
    <div class="left-sidebar-inner">
      <div class="sidebar-header">
        <div class="app-logo">Tine</div>
        <GraphSwitcher />
      </div>

      <div class="nav-search">
        <input
          class="search-input"
          type="text"
          placeholder="Search"
          readonly
          onClick={() => openSwitcher()}
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
                  onMouseDown={shiftGuard}
                  onClick={(e) =>
                    e.shiftKey ? openPageInSidebar(fav.name, fav.kind) : openPage(fav.name, fav.kind)
                  }
                  onAuxClick={(e) => {
                    if (e.button === 1) {
                      e.preventDefault();
                      openPageInNewTab(fav.name, fav.kind);
                    }
                  }}
                  onContextMenu={(e) => openRowMenu(e, fav.name, fav.kind)}
                >
                  {/* ⭐ + name via EmojiText: WebKitGTK's Skia COLRv1 path
                      crashes painting a raw color-emoji glyph on hardened
                      libstdc++ (#29); Twemoji <img> never touches the font. */}
                  <EmojiText text={`⭐ ${fav.name}`} />
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
                  onMouseDown={shiftGuard}
                  onClick={(e) =>
                    e.shiftKey ? openPageInSidebar(r.name, r.kind) : openPage(r.name, r.kind)
                  }
                  onAuxClick={(e) => {
                    if (e.button === 1) {
                      e.preventDefault();
                      openPageInNewTab(r.name, r.kind);
                    }
                  }}
                  onContextMenu={(e) => openRowMenu(e, r.name, r.kind)}
                >
                  <EmojiText text={r.name.startsWith("hls__") ? r.name.slice(5) : r.name} />
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
                  classList={{ active: isActive(p.name, p.path) }}
                  onMouseDown={shiftGuard}
                  onClick={(e) =>
                    e.shiftKey ? openPageInSidebar(p.name, "page") : openEntry(p.path, p.name)
                  }
                  onAuxClick={(e) => {
                    if (e.button === 1) {
                      e.preventDefault();
                      p.path
                        ? openInNewTab({ kind: "page", name: p.name, pageKind: "page", path: p.path })
                        : openPageInNewTab(p.name, "page");
                    }
                  }}
                  onContextMenu={(e) => openRowMenu(e, p.name, "page")}
                >
                  <EmojiText text={pageListLabel(p, allPages())} />
                </div>
              )}
            </For>
            <Show when={allPages().length > ALL_PAGES_CAP}>
              <div class="nav-page nav-page-more" onClick={() => openSwitcher()}>
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
        <button class="new-page-btn" onClick={() => openSwitcher()}>+ New page</button>
      </div>
    </div>
  );
}

// Display name for the active graph = basename of its root folder (OG shows the
// same). Falls back to "No graph" when none is loaded (e.g. mock/first run).
function graphDisplayName(): string {
  const root = graphMeta()?.root;
  if (!root) return "No graph";
  const base = root.replace(/[/\\]+$/, "").split(/[/\\]/).pop();
  return base || root;
}

// The current-graph control in the sidebar header. OG puts a graph-name dropdown
// top-left (database icon → switch/new/all-graphs/re-index). Tine has no
// multi-graph list yet (that's R4a), so this surfaces the already-existing
// open/create actions — making graph switching DISCOVERABLE rather than buried
// in Settings.
function GraphSwitcher(): JSX.Element {
  const [open, setOpen] = createSignal(false);
  const close = () => setOpen(false);

  // Esc closes the menu (the backdrop handles click-outside).
  createEffect(() => {
    if (!open()) return;
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") close();
    };
    window.addEventListener("keydown", onKey);
    onCleanup(() => window.removeEventListener("keydown", onKey));
  });

  return (
    <div class="graph-switch">
      <button
        class="graph-switch-btn"
        title={graphMeta()?.root ?? undefined}
        onClick={() => setOpen(!open())}
      >
        <svg viewBox="0 0 24 24" class="graph-switch-icon">
          <ellipse cx="12" cy="5" rx="7" ry="3" fill="none" stroke="currentColor" stroke-width="1.6" />
          <path d="M5 5v7c0 1.66 3.13 3 7 3s7-1.34 7-3V5" fill="none" stroke="currentColor" stroke-width="1.6" />
          <path d="M5 12v7c0 1.66 3.13 3 7 3s7-1.34 7-3v-7" fill="none" stroke="currentColor" stroke-width="1.6" />
        </svg>
        <span class="graph-switch-name">{graphDisplayName()}</span>
        <span class="graph-switch-caret">▾</span>
      </button>
      <Show when={open()}>
        <div
          class="graph-switch-backdrop"
          onClick={close}
          onContextMenu={(e) => {
            e.preventDefault();
            close();
          }}
        />
        <div class="ctx-menu graph-switch-menu">
          <div
            class="ctx-item"
            onClick={() => {
              close();
              void switchGraph();
            }}
          >
            Open graph…
          </div>
          <div
            class="ctx-item"
            onClick={() => {
              close();
              void createNewGraph();
            }}
          >
            New graph…
          </div>
        </div>
      </Show>
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
