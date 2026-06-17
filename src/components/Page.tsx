import { For, Show, createEffect, createSignal, on, onCleanup, onMount, untrack, type JSX } from "solid-js";
import { doc, mainPages, pageByName, reloadPage, loadSingle, loadFeed, appendFeed, isDirty, editingId, setFeedExtender, flushPage, type FeedPage } from "../store";
import { route, openPage, openJournals } from "../router";
import {
  zoomedBlock, zoomOut, zoomInto, isFavorite, toggleFavorite, notesRefresh,
  markConflict, graphEpoch, openPageInSidebar, openPageContextMenu, carryDays, showCarryButtons,
} from "../ui";
import { carryDay, carryPrevDay, carryDaysBack } from "../carry";
import { backend } from "../backend";
import { switchGraph } from "../graph";
import { Block } from "./Block";
import { LinkedReferences } from "./LinkedReferences";
import { UnlinkedReferences } from "./UnlinkedReferences";
import { QueryMacro } from "./Macro";
import { NamespaceCrumb, NamespaceChildren } from "./Namespace";
import { isPropertyLine, blockView } from "../render/block";
import { InlineText } from "../render/inline";
import { journalTitle } from "../journal";
import type { PageDto } from "../types";

const FEED_PAGE = 3;

function emptyPage(name: string, kind: "journal" | "page"): PageDto {
  return {
    name,
    kind,
    title: name,
    pre_block: null,
    blocks: [{ id: `new-${name}`, raw: "", collapsed: false, children: [] }],
  };
}

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

  // Clear any zoom when the route changes (navigation exits a zoomed block).
  createEffect(() => {
    route();
    zoomOut();
  });

  createEffect(() => {
    const r = route();
    graphEpoch(); // reload when the open graph changes
    setReady(false);
    setLoadError(null);
    void (async () => {
      try {
        if (r.kind === "journals") {
          feedDone = false;
          const js = await backend().journalsDesc(FEED_PAGE, 0);
          journalOffset = js.length;
          if (js.length < FEED_PAGE) feedDone = true;
          loadFeed(withToday(js));
        } else {
          const dto = await backend().getPage(r.name, r.pageKind);
          // null = page doesn't exist yet → start a fresh empty page. A failed
          // read throws and is caught below, so we never overwrite a page whose
          // load errored with empty content.
          loadSingle(dto ? toLoadable(dto, r.name) : emptyPage(r.name, r.pageKind));
        }
        setReady(true);
      } catch (e) {
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
        // Only suppress an auto-reload if the user is editing a block ON the
        // changed page (don't yank that caret); editing elsewhere is fine.
        const editingThis = () => {
          const ed = editingId();
          return !!ed && doc.byId[ed]?.page === c.name;
        };
        if (c.removed) {
          if (r.kind === "page" && r.name === c.name) openJournals();
          return;
        }
        if (r.kind === "page" && r.name === c.name) {
          if (isDirty(c.name)) return void markConflict(c.name);
          if (editingThis()) return;
          void (async () => {
            const dto = await backend().getPage(c.name, c.kind);
            if (dto) loadSingle(toLoadable(dto, c.name));
          })();
          return;
        }
        if (r.kind === "journals" && c.kind === "journal") {
          // Never let an external change clobber an unsaved edit: if the changed
          // day has pending edits, surface a conflict instead of reloading; if a
          // block on it is being edited, leave the caret alone.
          if (isDirty(c.name)) return void markConflict(c.name);
          if (editingThis()) return;
          void (async () => {
            if (pageByName(c.name)) {
              // Refresh just the changed day in place — a full feed reload would
              // also drop OTHER loaded days' unsaved edits.
              const dto = await backend().getPage(c.name, c.kind);
              if (dto) reloadPage(dto);
              return;
            }
            // A new journal file appeared (e.g. today created elsewhere): pull
            // the feed to include it, but only if no loaded day is dirty.
            if (doc.feed.some((n) => isDirty(n))) return;
            const js = await backend().journalsDesc(FEED_PAGE, 0);
            journalOffset = js.length;
            feedDone = js.length < FEED_PAGE;
            loadFeed(withToday(js));
          })();
        }
        // Satellite page (open only in the sidebar / as a query result) changed
        // on disk: keep its live copy fresh, unless it has unsaved edits (→
        // conflict) or a block on it is being edited.
        if (pageByName(c.name) && !doc.feed.includes(c.name)) {
          if (isDirty(c.name)) return void markConflict(c.name);
          if (editingThis()) return;
          void (async () => {
            const dto = await backend().getPage(c.name, c.kind);
            if (dto) reloadPage(dto);
          })();
        }
      })
      .then((u) => (unsub = u));
    onCleanup(() => unsub());
  });

  // Reload the open notes (hls__) page after a highlight is written, so the new
  // annotation shows without re-opening the page.
  createEffect(
    on(
      notesRefresh,
      (nr) => {
        const r = untrack(route);
        if (r.kind === "page" && r.name === nr.page) {
          void (async () => {
            const dto = await backend().getPage(r.name, r.pageKind);
            if (dto) loadSingle(toLoadable(dto, r.name));
          })();
        }
      },
      { defer: true }
    )
  );

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
                {/* Agenda sits at the bottom of today's (the first) day, like OG. */}
                <Show when={i() === 0 && route().kind === "journals"}>
                  <div class="agenda-block">
                    <QueryMacro
                      body="query (and (or (scheduled) (deadline)) (between -7d +7d))"
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
              <NamespaceChildren name={mainPages()[0].name} />
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
  const crumb = (id: string) => blockView(doc.byId[id].raw).lines[0] || "…";

  return (
    <div class="page zoomed-page">
      <div class="zoom-breadcrumb">
        <a
          class="crumb crumb-page"
          onClick={() => {
            zoomOut();
            openPage(pageName(), pageKind());
          }}
        >
          {pageName()}
        </a>
        <For each={ancestors()}>
          {(aid) => (
            <>
              <span class="crumb-sep">›</span>
              <a class="crumb" onClick={() => zoomInto(aid)}>
                <InlineText text={crumb(aid)} />
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
      // Flush unsaved edits before the file is moved on disk and reloaded.
      await flushPage(props.page.name);
      await backend().renamePage(props.page.name, next);
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
            title={props.page.kind === "page" ? "Double-click to rename (shift-click → sidebar)" : "Shift-click to open in sidebar"}
            onClick={(e) => {
              if (e.shiftKey) openPageInSidebar(props.page.name, props.page.kind);
              else openPage(props.page.name, props.page.kind);
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
            {props.page.title}
          </h1>
        </Show>
        <CarryActions page={props.page} />
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
      <Show when={props.page.preBlock}>
        <div class="page-properties">
          <For each={props.page.preBlock!.split("\n").filter(isPropertyLine)}>
            {(line) => {
              const idx = line.indexOf("::");
              return (
                <div class="prop-row">
                  <span class="prop-key">{line.slice(0, idx).trim()}</span>
                  <span class="prop-value">
                    <InlineText text={line.slice(idx + 2).trim()} />
                  </span>
                </div>
              );
            }}
          </For>
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
