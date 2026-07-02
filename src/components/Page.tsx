import { For, Show, createEffect, createMemo, createSignal, onCleanup, onMount, type JSX } from "solid-js";
import { doc, mainPages, pageByName, reloadPage, loadSingle, loadFeed, appendFeed, emptyPage, isDirty, isSaving, reloadDisposition, setFeedExtender, flushAll, formatForBlock, type FeedPage } from "../store";
import { route, sameRoute, openPage, openJournals, openPageInNewTab, restoreScrollFor } from "../router";
import {
  zoomedBlock, zoomInto, isFavorite, toggleFavorite,
  markConflict, isConflicted, graphEpoch, openPageInSidebar, openPageContextMenu, carryDays, showCarryButtons,
  agendaQuery, openPageProps,
} from "../ui";
import { carryDay, carryPrevDay, carryDaysBack } from "../carry";
import { backend } from "../backend";
import { switchGraph, refreshAfterRename } from "../graph";
import { Block } from "./Block";
import { LinkedReferences } from "./LinkedReferences";
import { UnlinkedReferences } from "./UnlinkedReferences";
import { QueryMacro } from "./Macro";
import { NamespaceCrumb, NamespaceHierarchy } from "./Namespace";
import { pageProperties, aliasNames, visibleBody } from "../render/block";
import { InlineText } from "../render/inline";
import { EmojiText } from "../render/emoji";
import { journalTitle } from "../journal";
import type { PageDto } from "../types";

const FEED_PAGE = 3;

// Page properties NOT shown in the under-title property list: `alias` is surfaced
// as "aka" chips above, and `icon` is consumed as the page icon next to the title
// (OG hides it too). Other internal/metadata page props could be added here.
const PAGE_PROPS_HIDDEN = new Set(["alias", "icon"]);

// OG always shows today's journal at the top of the feed, even with no file yet
// (the file is created lazily on first edit — Tine writes on save). So prepend
// an empty today page unless the newest journal on disk already is today.
function withToday(js: PageDto[]): PageDto[] {
  const title = journalTitle(new Date());
  if (js.some((p) => p.name === title)) return js;
  return [emptyPage(title, "journal"), ...js];
}

export function PageView(): JSX.Element {
  const [ready, setReady] = createSignal(false);
  const [loadError, setLoadError] = createSignal<string | null>(null);
  let journalOffset = 0;
  let loadingMore = false;
  let feedDone = false;

  // A non-existent page opens as a single empty bullet; but a page that *exists*
  // with properties and zero bullets (e.g. `title::`-only) must keep its
  // pre-block — otherwise editing it would save an empty file and drop the
  // properties. So preserve the DTO and only add an editable block.
  const toLoadable = (dto: PageDto, name: string): PageDto =>
    dto.blocks.length
      ? dto
      : { ...dto, blocks: [{ id: `new-${name}`, raw: "", collapsed: false, children: [] }] };

  // Depend on the active route BY VALUE: opening a background tab (or pinning /
  // reordering / closing another tab) mutates the `tabs` signal but not the active
  // route — without this, route() would re-fire this loader, remount the feed via
  // setReady(false), and reset scroll to the top.
  const currentRoute = createMemo(route, undefined, { equals: sameRoute });
  createEffect(() => {
    const r = currentRoute();
    const epoch = graphEpoch(); // reload when the open graph changes
    setReady(false);
    setLoadError(null);
    void (async () => {
      try {
        if (r.kind === "journals") {
          feedDone = false;
          const js = await backend().journalsDesc(FEED_PAGE, 0);
          if (epoch !== graphEpoch()) return; // graph switched mid-load — drop it
          journalOffset = js.length;
          if (js.length < FEED_PAGE) feedDone = true;
          loadFeed(withToday(js));
        } else {
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
          loadSingle(dto ? toLoadable(dto, r.name) : emptyPage(r.name, r.pageKind));
        }
        setReady(true);
        // Put the scroll back where it was when we last left this entry (back/
        // forward, or returning to this tab). A new page has no saved offset → top.
        restoreScrollFor(r);
      } catch (e) {
        if (epoch !== graphEpoch()) return;
        setLoadError(String(e));
        setReady(true);
      }
    })();
  });

  // Live external-change watcher: when the file watcher reports a page changed
  // on disk (Logseq / Syncthing), auto-reload it — unless it has unsaved edits
  // (→ conflict banner) or its editor is focused (don't yank the cursor).
  onMount(() => {
    let unsub = () => {};
    void backend()
      .onGraphChanged((c) => {
        const r = route();
        if (c.removed) {
          // Deleted externally. If the page has unsaved local state, surface a CONFLICT
          // instead of silently navigating away (which would drop the in-memory edits) —
          // the user can Keep-mine to recreate it (audit M5). Otherwise reload journals
          // in place (it no longer exists), even on a pinned tab.
          const disp = reloadDisposition(c.name);
          if (disp === "conflict") {
            markConflict(c.name);
            return;
          }
          if (disp === "skip") return; // being edited / move mid-flight — leave the caret
          if (r.kind === "page" && r.name === c.name) openJournals({ inPlace: true });
          return;
        }
        // One rule for "the changed page should/shouldn't be reloaded from disk"
        // (reloadDisposition, src/store.ts): "conflict" = has unsaved edits → never
        // clobber; "skip" = a block on it is being edited or a move is mid-flight →
        // leave the caret alone; "reload" = safe to take the disk version. This used
        // to be hand-coded (and could drift) in each of the branches below.
        const disp = reloadDisposition(c.name);
        if (disp === "skip") return;
        if (disp === "conflict") {
          markConflict(c.name);
          return;
        }
        if (r.kind === "page" && r.name === c.name) {
          void (async () => {
            const dto = await backend().getPage(c.name, c.kind);
            if (dto) loadSingle(toLoadable(dto, c.name));
          })();
          return;
        }
        if (r.kind === "journals" && c.kind === "journal") {
          void (async () => {
            if (pageByName(c.name)) {
              // Refresh just the changed day in place — a full feed reload would
              // also drop OTHER loaded days' unsaved edits.
              const dto = await backend().getPage(c.name, c.kind);
              if (dto) reloadPage(dto);
              return;
            }
            // A new journal file appeared (e.g. today created elsewhere): pull
            // the feed to include it, but only if no OTHER loaded day is dirty.
            if (doc.feed.some((n) => isDirty(n) || isConflicted(n) || isSaving(n))) return;
            const js = await backend().journalsDesc(FEED_PAGE, 0);
            journalOffset = js.length;
            feedDone = js.length < FEED_PAGE;
            loadFeed(withToday(js));
          })();
          return;
        }
        // Satellite page (open only in the sidebar / as a query result) changed
        // on disk: keep its live copy fresh.
        if (pageByName(c.name) && !doc.feed.includes(c.name)) {
          void (async () => {
            const dto = await backend().getPage(c.name, c.kind);
            if (dto) reloadPage(dto);
          })();
        }
      })
      .then((u) => (unsub = u));
    onCleanup(() => unsub());
  });

  const loadMore = async () => {
    if (route().kind !== "journals" || loadingMore || feedDone) return;
    loadingMore = true;
    const js = await backend().journalsDesc(FEED_PAGE, journalOffset);
    journalOffset += js.length;
    if (js.length) appendFeed(js);
    else feedDone = true;
    loadingMore = false;
  };

  // Let a cross-day move-down pull in older days when it runs off the last
  // loaded one (returns whether more was actually loaded).
  setFeedExtender(async () => {
    const before = doc.feed.length;
    await loadMore();
    return doc.feed.length > before;
  });

  const zoomValid = () => {
    const z = zoomedBlock();
    return z && doc.byId[z] ? z : null;
  };

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
    <Show when={ready() && doc.loaded} fallback={<div class="page-loading" />}>
      <Show when={zoomValid()} fallback={
        <div class="page">
          <For each={mainPages()}>
            {(p, i) => (
              <>
                <PageSection page={p} />
                {/* Agenda sits at the bottom of today's (the first) day, like OG.
                    Window is configurable (Settings → Journal) and keyed off the
                    item's scheduled/deadline date over the whole graph. */}
                <Show when={i() === 0 && route().kind === "journals"}>
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
          <Show when={route().kind === "journals" && mainPages().length === 0}>
            <div class="page-load-error">
              No journal entries found in this graph.
              <div class="page-load-error-hint">
                Make sure you opened a Logseq graph (a folder with{" "}
                <code>journals/</code> + <code>pages/</code>). Use{" "}
                <button class="conflict-btn" onClick={() => void switchGraph()}>Open graph…</button>
              </div>
            </div>
          </Show>
          <Show when={route().kind === "journals"}>
            <LoadMore onHit={loadMore} />
          </Show>
          <Show when={route().kind === "page" && mainPages()[0]}>
            <Show when={mainPages()[0].kind === "page"}>
              <NamespaceHierarchy name={mainPages()[0].name} />
            </Show>
            <LinkedReferences name={mainPages()[0].name} />
            <UnlinkedReferences name={mainPages()[0].name} />
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
          onClick={() => openPage(pageName(), pageKind())}
        >
          {pageName()}
        </a>
        <For each={ancestors()}>
          {(aid) => (
            <>
              <span class="crumb-sep">›</span>
              <a class="crumb" onClick={() => zoomInto(aid)}>
                <InlineText text={crumb(aid)} format={formatForBlock(aid)} />
              </a>
            </>
          )}
        </For>
      </div>
      <div class="page-blocks zoomed-block">
        <Block id={props.id} />
      </div>
    </div>
  );
}

function PageSection(props: { page: FeedPage }): JSX.Element {
  const [renaming, setRenaming] = createSignal(false);
  const [newName, setNewName] = createSignal("");
  const startRename = () => {
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
      openPage(next, "page");
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
            title={props.page.kind === "page" ? "Double-click to rename (shift-click → sidebar, middle-click → new tab)" : "Shift-click to open in sidebar, middle-click → new tab"}
            onClick={(e) => {
              if (e.shiftKey) openPageInSidebar(props.page.name, props.page.kind);
              else openPage(props.page.name, props.page.kind);
            }}
            onAuxClick={(e) => {
              if (e.button === 1) {
                e.preventDefault(); // middle-click → background tab, like a body link
                openPageInNewTab(props.page.name, props.page.kind);
              }
            }}
            onDblClick={startRename}
            onContextMenu={(e) => {
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
        <CarryActions page={props.page} />
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
      <Show when={props.page.readOnly}>
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
              class="carry-btn"
              title="Move this day's unfinished tasks to today"
              onClick={() => void carryDay(props.page.name)}
            >
              Carry unfinished tasks → today
            </button>
          }
        >
          <button
            class="carry-btn"
            title="Pull unfinished tasks from the most recent day that has content"
            onClick={() => void carryPrevDay()}
          >
            Carry from previous day
          </button>
          <button
            class="carry-btn"
            title={`Pull unfinished tasks from the last ${carryDays()} days (change N in Settings)`}
            onClick={() => void carryDaysBack(carryDays())}
          >
            Carry last {carryDays()} days
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
