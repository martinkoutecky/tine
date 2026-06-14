import { For, Show, createEffect, createSignal, onCleanup, type JSX } from "solid-js";
import { doc, loadSingle, loadFeed, appendFeed, type FeedPage } from "../store";
import { route, openPage } from "../router";
import { backend } from "../backend";
import { Block } from "./Block";
import { LinkedReferences } from "./LinkedReferences";
import { isPropertyLine } from "../render/block";
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

  createEffect(() => {
    const r = route();
    setReady(false);
    void (async () => {
      if (r.kind === "journals") {
        const js = await backend().journalsDesc(FEED_PAGE, 0);
        journalOffset = js.length;
        loadFeed(js.length ? js : []);
      } else {
        const dto = await backend().getPage(r.name, r.pageKind);
        loadSingle(dto && dto.blocks.length ? dto : emptyPage(r.name, r.pageKind));
      }
      setReady(true);
    })();
  });

  const loadMore = async () => {
    if (route().kind !== "journals") return;
    const js = await backend().journalsDesc(FEED_PAGE, journalOffset);
    journalOffset += js.length;
    if (js.length) appendFeed(js);
  };

  return (
    <Show when={ready() && doc.loaded} fallback={<div class="page-loading" />}>
      <div class="page">
        <For each={doc.pages}>{(p) => <PageSection page={p} />}</For>
        <Show when={route().kind === "journals"}>
          <LoadMore onHit={loadMore} />
        </Show>
        <Show when={route().kind === "page" && doc.pages[0]}>
          <LinkedReferences name={doc.pages[0].name} />
        </Show>
      </div>
    </Show>
  );
}

function PageSection(props: { page: FeedPage }): JSX.Element {
  return (
    <div class="page-section">
      <h1
        class="page-title"
        classList={{ "journal-title": props.page.kind === "journal" }}
        onClick={() => openPage(props.page.name, props.page.kind)}
      >
        {props.page.title}
      </h1>
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
