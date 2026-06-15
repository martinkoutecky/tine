import { For, Show, createEffect, createSignal, on, onCleanup, untrack, type JSX } from "solid-js";
import { doc, loadSingle, loadFeed, appendFeed, type FeedPage } from "../store";
import { route, openPage, openJournals } from "../router";
import { zoomedBlock, zoomOut, zoomInto, isFavorite, toggleFavorite, notesRefresh } from "../ui";
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
  let journalOffset = 0;
  let loadingMore = false;
  let feedDone = false;

  // Clear any zoom when the route changes (navigation exits a zoomed block).
  createEffect(() => {
    route();
    zoomOut();
  });

  createEffect(() => {
    const r = route();
    setReady(false);
    void (async () => {
      if (r.kind === "journals") {
        feedDone = false;
        const js = await backend().journalsDesc(FEED_PAGE, 0);
        journalOffset = js.length;
        if (js.length < FEED_PAGE) feedDone = true;
        loadFeed(js.length ? js : []);
      } else {
        const dto = await backend().getPage(r.name, r.pageKind);
        loadSingle(dto && dto.blocks.length ? dto : emptyPage(r.name, r.pageKind));
      }
      setReady(true);
    })();
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
            loadSingle(dto && dto.blocks.length ? dto : emptyPage(r.name, r.pageKind));
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
    <Show when={ready() && doc.loaded} fallback={<div class="page-loading" />}>
      <Show when={zoomValid()} fallback={
        <div class="page">
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
  return (
    <div class="page-section">
      <div class="page-title-row">
        <h1
          class="page-title"
          classList={{ "journal-title": props.page.kind === "journal" }}
          onClick={() => openPage(props.page.name, props.page.kind)}
        >
          {props.page.title}
        </h1>
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
