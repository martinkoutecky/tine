import { For, Show, createEffect, createMemo, createResource, createSignal, onCleanup, untrack, useContext, type JSX } from "solid-js";
import { doc, mainPages, pageByName, loadFeed, appendFeed, emptyPage, ensurePageLoaded, setFeedExtender, flushAll, formatForBlock, readPageProperty, setPageProperty, appendToTodayJournal, ensureEmptyBlock, type FeedPage } from "../store";
import { sameRoute, type PaneRouter } from "../router";
import { PaneContext, focusedRouter } from "../panes";
import {
  zoomedBlock, isFavorite, toggleFavorite,
  graphEpoch, openPageInSidebar, openPageContextMenu, carryDays, showCarryButtons,
  agendaQuery, openPageProps, dataRev,
} from "../ui";
import { carryDay, carryPrevDay, carryDaysBack } from "../carry";
import { backend } from "../backend";
import { switchGraph, refreshAfterRename } from "../graph";
import { Block } from "./Block";
import { LinkedReferences } from "./LinkedReferences";
import { UnlinkedReferences } from "./UnlinkedReferences";
import { QueryMacro } from "./Macro";
import { SheetTable } from "./SheetTable";
import { NamespaceCrumb, NamespaceHierarchy } from "./Namespace";
import { pageProperties, aliasNames, visibleBody } from "../render/block";
import { InlineText } from "../render/inline";
import { EmojiText } from "../render/emoji";
import { journalTitle } from "../journal";
import { endEditForSurface, startEditing } from "../editorController";
import type { PageDto, RefGroup } from "../types";
import { tagRef } from "../tags";
import { copyGuideIntoGraph, ensureGuidePagesLoaded, isGuidePageName } from "../guide";

export const FEED_PAGE = 3;
let journalOffset = 0;
let loadingMore = false;
let feedDone = false;

// Page properties NOT shown in the under-title property list: `alias` is surfaced
// as "aka" chips above, and `icon` is consumed as the page icon next to the title
// (OG hides it too). Other internal/metadata page props could be added here.
const PAGE_PROPS_HIDDEN = new Set(["alias", "icon", "tine.tag-table"]);
const TAG_TABLE_PROP = "tine.tag-table";

function paneContextFromContext() {
  const ctx = useContext(PaneContext);
  return ctx ?? { paneId: "main", router: focusedRouter() };
}

function paneRouterFromContext(): PaneRouter {
  return paneContextFromContext().router;
}

// OG always shows today's journal at the top of the feed, even with no file yet
// (the file is created lazily on first edit — Tine writes on save). So prepend
// an empty today page unless the newest journal on disk already is today.
export function withToday(js: PageDto[]): PageDto[] {
  const title = journalTitle(new Date());
  if (js.some((p) => p.name === title)) return js;
  return [emptyPage(title, "journal"), ...js];
}

export function toLoadablePage(dto: PageDto, name: string): PageDto {
  return dto.blocks.length
    ? dto
    : { ...dto, blocks: [{ id: `new-${name}`, raw: "", collapsed: false, children: [] }] };
}

export async function reloadJournalsFeedFromStart() {
  const js = await backend().journalsDesc(FEED_PAGE, 0);
  journalOffset = js.length;
  feedDone = js.length < FEED_PAGE;
  loadFeed(withToday(js), { endEdit: false });
}

export function PageView(): JSX.Element {
  const pane = paneContextFromContext();
  const router = pane.router;
  const [ready, setReady] = createSignal(false);
  const [loadError, setLoadError] = createSignal<string | null>(null);

  // Depend on the active route BY VALUE: opening a background tab (or pinning /
  // reordering / closing another tab) mutates the `tabs` signal but not the active
  // route — without this, route() would re-fire this loader, remount the feed via
  // setReady(false), and reset scroll to the top.
  const currentRoute = createMemo(() => router.route(), undefined, { equals: sameRoute });
  createEffect(() => {
    const r = currentRoute();
    const epoch = graphEpoch(); // reload when the open graph changes
    setReady(false);
    setLoadError(null);
    // Surface keys are STATIC per pane (matching PaneLeaf's frozen provider
    // value — Solid context values don't react to route changes): the main
    // pane is "main" whatever it shows, every other pane is pane:{id}.
    // untrack is LOAD-BEARING: endEditForSurface reads editingId/activeSurface,
    // and without it this loader effect subscribes to them — so every
    // startEditing re-ran the loader, which instantly ended the fresh edit
    // (killed sheet cell editing) and re-fetched the feed per keystroke.
    untrack(() =>
      endEditForSurface("page-navigation", pane.paneId === "main" ? "main" : `pane:${pane.paneId}`)
    );
    void (async () => {
      try {
        if (r.kind === "journals") {
          const js = await backend().journalsDesc(FEED_PAGE, 0);
          if (epoch !== graphEpoch()) return; // graph switched mid-load — drop it
          journalOffset = js.length;
          if (js.length < FEED_PAGE) feedDone = true;
          else feedDone = false;
          loadFeed(withToday(js), { endEdit: false });
        } else {
          if (isGuidePageName(r.name)) {
            await ensureGuidePagesLoaded(true);
            if (epoch !== graphEpoch()) return;
            setReady(true);
            router.restoreScrollFor(r);
            return;
          }
          // A path-pinned route (#21) loads that SPECIFIC file — the way to reach a
          // duplicate-day stray that shares a (kind,name) with the canonical day;
          // everything else resolves by name as before.
          const dto = r.path
            ? await backend().getPageByPath(r.path)
            : await backend().getPage(r.name, r.pageKind);
          if (epoch !== graphEpoch()) return; // graph switched mid-load — drop it
          // null = page doesn't exist yet → start a fresh empty page. A failed
          // read throws and is caught below, so we never overwrite a page whose
          // load errored with empty content.
          ensurePageLoaded(dto ? toLoadablePage(dto, r.name) : emptyPage(r.name, r.pageKind));
        }
        setReady(true);
        // Put the scroll back where it was when we last left this entry (back/
        // forward, or returning to this tab). A new page has no saved offset → top.
        router.restoreScrollFor(r);
      } catch (e) {
        if (epoch !== graphEpoch()) return;
        setLoadError(String(e));
        setReady(true);
      }
    })();
  });

  const loadMore = async () => {
    if (currentRoute().kind !== "journals" || loadingMore || feedDone) return;
    loadingMore = true;
    const js = await backend().journalsDesc(FEED_PAGE, journalOffset);
    journalOffset += js.length;
    if (js.length) appendFeed(js);
    else feedDone = true;
    loadingMore = false;
  };

  createEffect(() => {
    if (currentRoute().kind !== "journals") return;
    const extender = async () => {
      const before = doc.feed.length;
      await loadMore();
      return doc.feed.length > before;
    };
    setFeedExtender(extender);
    onCleanup(() => setFeedExtender(null));
  });

  // GH #39: on macOS (WKWebView) the journal feed sometimes can't be scrolled on
  // first open until the window is nudged — resizing it makes scrolling start.
  // Cause: the feed content is injected ASYNCHRONOUSLY (after the load resolves,
  // replacing the `.page-loading` placeholder), and WebKit doesn't always
  // re-establish the scroll container's overflow region for content that grows
  // after first paint. A resize forces the relayout that fixes it — so we force
  // that relayout ourselves once the feed has content. Invisible + a no-op where
  // the quirk doesn't occur (Linux/WebKitGTK, Chromium). Runs on the journals
  // route whenever the feed transitions to non-empty.
  createEffect(() => {
    if (currentRoute().kind !== "journals") return;
    if (!doc.loaded || doc.feed.length === 0) return; // re-runs when the feed populates
    requestAnimationFrame(() => {
      const el = document.querySelector<HTMLElement>(".main-content");
      if (!el) return;
      void el.scrollHeight; // read: flush pending layout
      const prev = el.style.overflowY;
      el.style.overflowY = "hidden"; // toggle the scroll box so WebKit recomputes
      void el.offsetHeight; // read: apply the off state
      el.style.overflowY = prev; // restore (scrollTop is preserved across this)
    });
  });

  const pagesToRender = () => {
    const r = currentRoute();
    if (r.kind === "journals") return mainPages();
    const p = pageByName(r.name);
    return p ? [p] : [];
  };
  const zoomValid = () => {
    const r = currentRoute();
    const z = r.kind === "page" ? r.block ?? null : zoomedBlock();
    return z && doc.byId[z] ? z : null;
  };
  const contentReady = () => ready() && (currentRoute().kind !== "journals" || doc.loaded);

  return (
    <Show when={!loadError()} fallback={
      <div class="page">
        <div class="page-load-error">
          Couldn't open this page: <code>{loadError()}</code>
          <div class="page-load-error-hint">
            Tine did not modify the file. Try reopening, or check the file on disk.
          </div>
        </div>
      </div>
    }>
    <Show when={contentReady()} fallback={<div class="page-loading" />}>
      <Show when={zoomValid()} fallback={
        <div class="page">
          <For each={pagesToRender()}>
            {(p, i) => (
              <>
                <PageSection page={p} />
                {/* Agenda sits at the bottom of today's (the first) day, like OG.
                    Window is configurable (Settings → Journal) and keyed off the
                    item's scheduled/deadline date over the whole graph. */}
                <Show when={i() === 0 && currentRoute().kind === "journals"}>
                  <div class="agenda-block">
                    <QueryMacro
                      body={agendaQuery()}
                      title="Scheduled & Deadline"
                      hideWhenEmpty
                    />
                  </div>
                </Show>
              </>
            )}
          </For>
          <Show when={currentRoute().kind === "journals" && mainPages().length === 0}>
            <div class="page-load-error">
              No journal entries found in this graph.
              <div class="page-load-error-hint">
                Make sure you opened a Logseq graph (a folder with{" "}
                <code>journals/</code> + <code>pages/</code>). Use{" "}
                <button class="conflict-btn" onClick={() => void switchGraph()}>Open graph…</button>
              </div>
            </div>
          </Show>
          <Show when={currentRoute().kind === "journals"}>
            <LoadMore onHit={loadMore} />
          </Show>
          <Show when={currentRoute().kind === "page" && pagesToRender()[0]}>
            <Show when={pagesToRender()[0].kind === "page" && !pagesToRender()[0].guide}>
              <NamespaceHierarchy name={pagesToRender()[0].name} />
            </Show>
            <Show
              when={pagesToRender()[0].kind === "page" && !pagesToRender()[0].guide && tagTableEnabled(pagesToRender()[0].name)}
              fallback={<Show when={!pagesToRender()[0].guide}><LinkedReferences name={pagesToRender()[0].name} /></Show>}
            >
              <TagPageTable pageName={pagesToRender()[0].name} />
            </Show>
            <Show when={!pagesToRender()[0].guide}>
              <UnlinkedReferences name={pagesToRender()[0].name} />
            </Show>
          </Show>
        </div>
      }>
        <ZoomedView id={zoomValid()!} />
      </Show>
    </Show>
    </Show>
  );
}

// A single zoomed-in block (its subtree) with an ancestor breadcrumb.
function ZoomedView(props: { id: string }): JSX.Element {
  const router = paneRouterFromContext();
  const ancestors = (): string[] => {
    const out: string[] = [];
    let p = doc.byId[props.id]?.parent ?? null;
    while (p !== null) {
      out.unshift(p);
      p = doc.byId[p].parent;
    }
    return out;
  };
  const pageName = () => doc.byId[props.id]?.page ?? "";
  const pageKind = () => doc.pages.find((p) => p.name === pageName())?.kind ?? "page";
  const crumb = (id: string) => visibleBody(doc.byId[id].raw)[0] || "…";

  return (
    <div class="page zoomed-page">
      <div class="zoom-breadcrumb">
        <a
          class="crumb crumb-page"
          onClick={() => router.openPage(pageName(), pageKind())}
        >
          {pageName()}
        </a>
        <For each={ancestors()}>
          {(aid) => (
            <>
              <span class="crumb-sep">›</span>
              <a class="crumb" onClick={() => router.focusBlock(aid)}>
                <InlineText text={crumb(aid)} format={formatForBlock(aid)} />
              </a>
            </>
          )}
        </For>
      </div>
      <div class="page-blocks zoomed-block">
        {/* A zoom root is a viewing boundary: reveal its immediate subtree even
            when collapsed on the parent page, without mutating collapsed::.
            Descendants still honor their own individual collapse state. */}
        <Block id={props.id} forceExpanded />
      </div>
    </div>
  );
}

function PageSection(props: { page: FeedPage }): JSX.Element {
  const router = paneRouterFromContext();
  const [renaming, setRenaming] = createSignal(false);
  const [newName, setNewName] = createSignal("");
  // A page emptied of its last block (explicit Delete bypasses the Backspace
  // last-block guard) would render nothing to type into. Re-seed the phantom empty
  // bullet — same shape a brand-new day gets — so there's always a bullet present;
  // it only persists once the user types (ensureEmptyBlock leaves it non-dirty).
  createEffect(() => {
    if (props.page.roots.length === 0 && !props.page.readOnly) {
      ensureEmptyBlock(props.page.name);
    }
  });
  const startRename = () => {
    if (props.page.guide || props.page.readOnly) return;
    if (props.page.kind !== "page") return; // journals are named by their date
    setNewName(props.page.name);
    setRenaming(true);
  };
  const commitRename = async () => {
    const next = newName().trim();
    setRenaming(false);
    if (!next || next === props.page.name) return;
    try {
      // Flush ALL unsaved edits before the file is moved on disk — the rename
      // transaction reads every referencing page from disk to rewrite its
      // `[[refs]]`, so a dirty edit on ANY page (not just the renamed one) would
      // be read stale and its link left dangling. Abort if anything can't save.
      if (!(await flushAll())) {
        alert("Couldn't save pending edits — resolve the conflict before renaming.");
        return;
      }
      await backend().renamePage(props.page.name, next);
      // The backend rewrote refs across many pages via the self-write guard (no
      // watcher reload), so every in-memory page is now potentially stale; reset
      // + reload so a stale copy can't be saved back and revert the rename.
      refreshAfterRename();
      router.openPage(next, "page");
    } catch (e) {
      alert(`Rename failed: ${String(e)}`);
    }
  };

  return (
    <div class="page-section">
      <Show when={props.page.kind === "page"}>
        <NamespaceCrumb name={props.page.name} />
      </Show>
      <div class="page-title-row">
        <Show
          when={!renaming()}
          fallback={
            <input
              class="page-title-input"
              value={newName()}
              ref={(el) => queueMicrotask(() => (el.focus(), el.select()))}
              onInput={(e) => setNewName(e.currentTarget.value)}
              onKeyDown={(e) => {
                if (e.key === "Enter") void commitRename();
                else if (e.key === "Escape") setRenaming(false);
              }}
              onBlur={() => setRenaming(false)}
            />
          }
        >
          <h1
            class="page-title"
            classList={{ "journal-title": props.page.kind === "journal" }}
            title={props.page.guide ? "Bundled Guide page" : props.page.kind === "page" ? "Double-click to rename (shift-click → sidebar, middle-click → new tab)" : "Shift-click to open in sidebar, middle-click → new tab"}
            onClick={(e) => {
              if (e.shiftKey && !props.page.guide) openPageInSidebar(props.page.name, props.page.kind);
              else router.openPage(props.page.name, props.page.kind);
            }}
            onAuxClick={(e) => {
              if (e.button === 1) {
                e.preventDefault(); // middle-click → background tab, like a body link
                router.openPageInNewTab(props.page.name, props.page.kind);
              }
            }}
            onDblClick={startRename}
            onContextMenu={(e) => {
              if (props.page.guide) return;
              e.preventDefault();
              openPageContextMenu(e.clientX, e.clientY, props.page.name, props.page.kind);
            }}
          >
            <Show when={props.page.kind === "journal"}>
              <svg class="title-cal" viewBox="0 0 24 24" aria-hidden="true">
                <rect x="4" y="5" width="16" height="16" rx="2" fill="none" stroke="currentColor" stroke-width="1.7" />
                <line x1="4" y1="9.5" x2="20" y2="9.5" stroke="currentColor" stroke-width="1.7" />
                <line x1="8.5" y1="3" x2="8.5" y2="7" stroke="currentColor" stroke-width="1.7" />
                <line x1="15.5" y1="3" x2="15.5" y2="7" stroke="currentColor" stroke-width="1.7" />
              </svg>
            </Show>
            <Show
              when={pageProperties(props.page.preBlock, props.page.format)
                .find(([k]) => k.toLowerCase() === "icon")?.[1]
                ?.trim()}
            >
              {(icon) => (
                <span class="page-icon page-title-icon">
                  <EmojiText text={icon()} />
                </span>
              )}
            </Show>
            <EmojiText text={props.page.title} />
          </h1>
        </Show>
        <Show when={!props.page.guide}>
          <CarryActions page={props.page} />
          <TagTableToggle page={props.page} />
        </Show>
        <Show when={props.page.guide}>
          <button class="guide-copy-btn" onClick={() => void copyGuideIntoGraph(props.page.name)}>
            Copy the guide into your graph
          </button>
        </Show>
        <Show when={!props.page.guide}>
          <button
            class="page-gear"
            title="Page properties (alias, public, tags, icon, title)"
            onClick={(e) => openPageProps(props.page.name, e.clientX, e.clientY)}
          >
            <svg viewBox="0 0 24 24" class="gear-icon" aria-hidden="true">
              <path
                d="M12 8.5a3.5 3.5 0 100 7 3.5 3.5 0 000-7z M19.4 12.9c.04-.3.06-.6.06-.9s-.02-.6-.06-.9l1.7-1.3a.5.5 0 00.12-.64l-1.6-2.8a.5.5 0 00-.6-.22l-2 .8a6 6 0 00-1.55-.9l-.3-2.13a.5.5 0 00-.5-.42h-3.2a.5.5 0 00-.5.42l-.3 2.13a6 6 0 00-1.55.9l-2-.8a.5.5 0 00-.6.22l-1.6 2.8a.5.5 0 00.12.64l1.7 1.3c-.04.3-.06.6-.06.9s.02.6.06.9l-1.7 1.3a.5.5 0 00-.12.64l1.6 2.8c.13.23.4.31.6.22l2-.8c.47.37 1 .67 1.55.9l.3 2.13c.04.24.25.42.5.42h3.2c.25 0 .46-.18.5-.42l.3-2.13a6 6 0 001.55-.9l2 .8c.2.09.47.01.6-.22l1.6-2.8a.5.5 0 00-.12-.64z"
                fill="none"
                stroke="currentColor"
                stroke-width="1.4"
                stroke-linejoin="round"
              />
            </svg>
          </button>
          <button
            class="fav-star"
            classList={{ active: isFavorite(props.page.name) }}
            title={isFavorite(props.page.name) ? "Unfavorite" : "Add to favorites"}
            onClick={() => toggleFavorite(props.page.name, props.page.kind)}
          >
            <svg viewBox="0 0 24 24" class="star-icon" aria-hidden="true">
              <path
                d="M12 3.5l2.6 5.27 5.82.85-4.21 4.1.99 5.79L12 16.77l-5.2 2.73.99-5.79-4.21-4.1 5.82-.85z"
                fill={isFavorite(props.page.name) ? "currentColor" : "none"}
                stroke="currentColor"
                stroke-width="1.6"
                stroke-linejoin="round"
              />
            </svg>
          </button>
        </Show>
      </div>
      <Show when={aliasNames(props.page.preBlock, props.page.format).length}>
        <div class="page-aliases" title="Also known as — other names that link here">
          <span class="page-aliases-label">aka</span>
          <For each={aliasNames(props.page.preBlock, props.page.format)}>
            {(a) => <span class="alias-chip">{a}</span>}
          </For>
        </div>
      </Show>
      <Show when={pageProperties(props.page.preBlock, props.page.format).filter(([k]) => !PAGE_PROPS_HIDDEN.has(k.toLowerCase())).length}>
        <div class="page-properties">
          {/* `alias`/`icon` are surfaced elsewhere (chips / title icon) — see PAGE_PROPS_HIDDEN. */}
          <For each={pageProperties(props.page.preBlock, props.page.format).filter(([k]) => !PAGE_PROPS_HIDDEN.has(k.toLowerCase()))}>
            {([key, value]) => (
              <div class="prop-row">
                <span class="prop-key">{key}</span>
                <span class="prop-value">
                  <InlineText text={value} format={props.page.format} />
                </span>
              </div>
            )}
          </For>
        </div>
      </Show>
      <Show when={props.page.guide}>
        <div class="page-guide-banner">
          Bundled Guide page - read-only and not written to your graph.
          <button onClick={() => void copyGuideIntoGraph(props.page.name)}>
            Copy the guide into your graph
          </button>
        </div>
      </Show>
      <Show when={props.page.readOnly && !props.page.guide}>
        <div class="page-readonly-banner" title="Tine can't reproduce this .org file byte-for-byte, so it's shown read-only to avoid corrupting it. Edit it in Logseq/Emacs.">
          Read-only — this <code>.org</code> file uses a structure Tine can't safely
          round-trip yet, so it won't be edited here.
        </div>
      </Show>
      <div class="page-blocks">
        <For each={props.page.roots}>{(id) => <Block id={id} />}</For>
      </div>
    </div>
  );
}

function tagTableEnabled(pageName: string): boolean {
  return readPageProperty(pageName, TAG_TABLE_PROP)?.toLowerCase() === "true";
}

function quoteQueryString(value: string): string {
  return `"${value.replace(/\\/g, "\\\\").replace(/"/g, "\\\"")}"`;
}

function tagQuery(pageName: string): string {
  return `(tag ${quoteQueryString(pageName)})`;
}

function taggedCount(groups: readonly RefGroup[] | undefined): number {
  return groups?.reduce((sum, group) => sum + group.blocks.length, 0) ?? 0;
}

export function TagTableToggle(props: { page: FeedPage }): JSX.Element {
  const [groups] = createResource(
    () => (props.page.kind === "page" ? `${props.page.name}\0${dataRev()}` : null),
    () => backend().runQuery(tagQuery(props.page.name))
  );
  const enabled = () => tagTableEnabled(props.page.name);
  const visible = () => props.page.kind === "page" && (enabled() || taggedCount(groups()) > 0);
  return (
    <Show when={visible()}>
      <button
        class="tag-table-toggle"
        classList={{ active: enabled() }}
        title={enabled() ? "Hide tag table" : "Show tagged blocks as a table"}
        onClick={() => setPageProperty(props.page.name, TAG_TABLE_PROP, enabled() ? null : "true")}
      >
        ⊞ Table
      </button>
    </Show>
  );
}

export function TagPageTable(props: { pageName: string }): JSX.Element {
  const [groups] = createResource(
    () => `${props.pageName}\0${dataRev()}`,
    () => backend().runQuery(tagQuery(props.pageName))
  );
  const addRow = async () => {
    const ok = await appendToTodayJournal(`${tagRef(props.pageName)} `);
    if (!ok) return;
    const today = pageByName(journalTitle(new Date()));
    const id = today?.roots[today.roots.length - 1];
    if (id && doc.byId[id]) startEditing(id, doc.byId[id].raw.length);
  };
  return (
    <div class="tag-page-table">
      <SheetTable
        ownerId={`tag-page:${encodeURIComponent(props.pageName)}`}
        rowSource="query"
        groups={groups() ?? []}
        addRow={addRow}
        addRowLabel={`Add ${tagRef(props.pageName)} row`}
        schemaPage={props.pageName}
      />
    </div>
  );
}

// Discoverable carry-over actions under a journal's title (replaces having to
// right-click → "Carry…"). Today gets pull-in buttons (from the previous
// non-empty day, and from the last N days); a past day gets a push-to-today
// button. Named pages show nothing.
function CarryActions(props: { page: FeedPage }): JSX.Element {
  const isJournal = () => props.page.kind === "journal";
  const isToday = () => isJournal() && props.page.name === journalTitle(new Date());
  return (
    <Show when={isJournal() && showCarryButtons()}>
      <div class="page-carry-actions">
        <Show
          when={isToday()}
          fallback={
            <button
              class="carry-btn carry-btn-push"
              title="Move this day's unfinished tasks to today"
              onClick={() => void carryDay(props.page.name)}
            >
              <span class="carry-label-full">Carry unfinished tasks → today</span>
              <span class="carry-label-short">To today</span>
            </button>
          }
        >
          <button
            class="carry-btn carry-btn-prev"
            title="Pull unfinished tasks from the most recent day that has content"
            onClick={() => void carryPrevDay()}
          >
            <span class="carry-label-full">Carry from previous day</span>
            <span class="carry-label-short">Previous</span>
          </button>
          <button
            class="carry-btn carry-btn-days"
            title={`Pull unfinished tasks from the last ${carryDays()} days (change N in Settings)`}
            onClick={() => void carryDaysBack(carryDays())}
          >
            <span class="carry-label-full">Carry last {carryDays()} days</span>
            <span class="carry-label-short">Last {carryDays()}d</span>
          </button>
        </Show>
      </div>
    </Show>
  );
}

// Auto-loads more journals when scrolled into view.
function LoadMore(props: { onHit: () => void }): JSX.Element {
  let sentinel: HTMLDivElement | undefined;
  createEffect(() => {
    if (!sentinel) return;
    const obs = new IntersectionObserver((entries) => {
      if (entries.some((e) => e.isIntersecting)) props.onHit();
    });
    obs.observe(sentinel);
    onCleanup(() => obs.disconnect());
  });
  return <div ref={sentinel} class="feed-sentinel" />;
}
