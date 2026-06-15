import { For, Show, createEffect, createSignal, on, onCleanup, onMount, untrack, type JSX } from "solid-js";
import { doc, loadSingle, loadFeed, appendFeed, forceSave, isDirty, editingId, type FeedPage } from "../store";
import { route, openPage, openJournals } from "../router";
import {
  zoomedBlock, zoomOut, zoomInto, isFavorite, toggleFavorite, notesRefresh,
  isConflicted, clearConflict, markConflict,
} from "../ui";
import { backend } from "../backend";
import { Block } from "./Block";
import { LinkedReferences } from "./LinkedReferences";
import { UnlinkedReferences } from "./UnlinkedReferences";
import { isPropertyLine, blockView } from "../render/block";
import { InlineText } from "../render/inline";
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
    setReady(false);
    setLoadError(null);
    void (async () => {
      try {
        if (r.kind === "journals") {
          feedDone = false;
          const js = await backend().journalsDesc(FEED_PAGE, 0);
          journalOffset = js.length;
          if (js.length < FEED_PAGE) feedDone = true;
          loadFeed(js.length ? js : []);
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
        if (c.removed) {
          if (r.kind === "page" && r.name === c.name) openJournals();
          return;
        }
        if (r.kind === "page" && r.name === c.name) {
          if (isDirty(c.name)) return void markConflict(c.name);
          if (editingId()) return;
          void (async () => {
            const dto = await backend().getPage(c.name, c.kind);
            if (dto) loadSingle(toLoadable(dto, c.name));
          })();
        } else if (r.kind === "journals" && c.kind === "journal" && !editingId()) {
          void (async () => {
            const js = await backend().journalsDesc(FEED_PAGE, 0);
            journalOffset = js.length;
            feedDone = js.length < FEED_PAGE;
            loadFeed(js.length ? js : []);
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
          <Show when={route().kind === "page" && doc.pages[0] && isConflicted(doc.pages[0].name)}>
            <ConflictBanner name={doc.pages[0].name} kind={doc.pages[0].kind} />
          </Show>
          <For each={doc.pages}>{(p) => <PageSection page={p} />}</For>
          <Show when={route().kind === "journals"}>
            <LoadMore onHit={loadMore} />
          </Show>
          <Show when={route().kind === "page" && doc.pages[0]}>
            <LinkedReferences name={doc.pages[0].name} />
            <UnlinkedReferences name={doc.pages[0].name} />
          </Show>
        </div>
      }>
        <ZoomedView id={zoomValid()!} />
      </Show>
    </Show>
    </Show>
  );
}

// Shown when a save was refused because the file changed on disk (external edit
// or a Syncthing pull). Lets the user take the disk version or keep theirs.
function ConflictBanner(props: { name: string; kind: "journal" | "page" }): JSX.Element {
  const reload = async () => {
    const dto = await backend().getPage(props.name, props.kind);
    if (dto) loadSingle(dto.blocks.length ? dto : emptyPage(props.name, props.kind));
    clearConflict(props.name);
  };
  const keepMine = async () => {
    await forceSave(props.name);
    clearConflict(props.name);
  };
  return (
    <div class="conflict-banner">
      <span class="conflict-msg">
        <strong>“{props.name}” changed on disk</strong> (edited elsewhere or synced in). Your
        unsaved changes weren't written.
      </span>
      <span class="conflict-actions">
        <button class="conflict-btn" onClick={reload}>Use disk version</button>
        <button class="conflict-btn keep" onClick={keepMine}>Keep mine (overwrite)</button>
      </span>
    </div>
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
        <a class="crumb crumb-page" onClick={() => openPage(pageName(), pageKind())}>
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
      await backend().renamePage(props.page.name, next);
      openPage(next, "page");
    } catch (e) {
      alert(`Rename failed: ${String(e)}`);
    }
  };

  return (
    <div class="page-section">
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
            title={props.page.kind === "page" ? "Double-click to rename" : undefined}
            onClick={() => openPage(props.page.name, props.page.kind)}
            onDblClick={startRename}
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
        <button
          class="fav-star"
          classList={{ active: isFavorite(props.page.name) }}
          title={isFavorite(props.page.name) ? "Unfavorite" : "Add to favorites"}
          onClick={() => toggleFavorite(props.page.name, props.page.kind)}
        >
          {isFavorite(props.page.name) ? "★" : "☆"}
        </button>
        <Show when={props.page.kind === "page"}>
          <button
            class="page-delete"
            title="Delete page"
            onClick={async () => {
              if (confirm(`Delete page "${props.page.name}"? This removes the file.`)) {
                await backend().deletePage(props.page.name, "page");
                openJournals();
              }
            }}
          >
            🗑
          </button>
        </Show>
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
