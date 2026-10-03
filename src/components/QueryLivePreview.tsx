/** GH #619 item 7: the results of the query AS THE SHEET NOW SHOWS IT, inside the sheet.
 *
 *  The sheet covers the block's own results, so without this a user editing conditions sees nothing
 *  change until the sheet closes. This pane runs the session's current query (including an unsaved
 *  text-pane draft) itself, so what it shows is what the sheet's conditions say, not what was last
 *  persisted. Unit cost: one `query_run` per settled edit (a 250 ms quiet window collapses a burst of
 *  edits to one), at most `PREVIEW_ROWS` rows rendered, no graph write. A superseded answer never
 *  lands (I-20: `latestOwner`), and an identical request shares its in-flight IPC with the block's own
 *  run (`sharedQueryResult`). */
import { For, Show, createEffect, createMemo, createResource, createSignal, onCleanup, untrack, type JSX } from "solid-js";
import { backend } from "../backend";
import { dataRev, graphEpoch, graphMeta } from "../graphSession";
import { graphOwner, latestOwner, readOwned } from "../owned";
import { sharedQueryResult } from "../queryResultCache";
import { readLatestOr } from "../resourceRead";
import { visibleBody } from "../render/block";
import { bothFamilies } from "../queryTwin";
import { SearchResultRow } from "./SearchResultRow";
import type { ExecutionContext, PageRow, Query, ViewSettings } from "../editor/queryIr";
import type { RefGroup } from "../types";

const errorText = (error: unknown): string => (error instanceof Error ? error.message : String(error));

/** Rows rendered in the preview; the count line still reports the full total. */
export const PREVIEW_ROWS = 25;
/** Quiet time before a changed query runs. */
export const PREVIEW_DEBOUNCE_MS = 250;

interface PreviewRequest {
  query: Query;
  view: ViewSettings;
  context?: ExecutionContext;
  both: boolean;
  key: string;
}
/** GH #619 item 9: what "Pages and blocks" adds to an answer — the per-family totals and cut-off flags. */
interface PreviewFamilies {
  pageTotal: number;
  blockTotal: number;
  pageMore: boolean;
  blockMore: boolean;
  pageNote: string | null;
  blockNote: string | null;
}
interface PreviewAnswer {
  key: string;
  anchor: "block" | "page";
  blocks: { page: string; breadcrumb: string[]; text: string }[];
  pages: PageRow[];
  total: number;
  diagnostics: string[];
  families: PreviewFamilies | null;
}

type PreviewBlock = PreviewAnswer["blocks"][number];
const previewBlocks = (groups: RefGroup[]): PreviewBlock[] => groups.flatMap((group) => group.blocks.map((block) => ({
  page: group.page,
  breadcrumb: block.breadcrumb ?? [],
  text: [block.marker, ...visibleBody(block.raw)].filter(Boolean).join(" "),
})));

function without(groups: RefGroup[], hostBlockId: string | undefined): RefGroup[] {
  if (!hostBlockId) return groups;
  return groups
    .map((group) => ({ ...group, blocks: group.blocks.filter((block) => block.id !== hostBlockId) }))
    .filter((group) => group.blocks.length > 0);
}

export function QueryLivePreview(props: {
  query: () => Query | undefined;
  view: () => ViewSettings;
  context?: () => ExecutionContext | undefined;
  /** The block the query is written in: never one of its own results. */
  hostBlockId?: string;
  /** "Pages and blocks" mode: show both families of the query, as the block itself does. */
  both?: () => boolean;
}): JSX.Element {
  const request = createMemo<PreviewRequest | undefined>(() => {
    const query = props.query();
    if (!query) return undefined;
    const view = props.view();
    const context = props.context?.();
    const both = props.both?.() === true;
    return { query, view, context, both, key: JSON.stringify([query, view, context ?? null, both]) };
  }, undefined, { equals: (a, b) => a?.key === b?.key });

  const [settled, setSettled] = createSignal<PreviewRequest | undefined>(untrack(request));
  createEffect(() => {
    const next = request();
    if (next?.key === untrack(settled)?.key) return;
    const timer = setTimeout(() => setSettled(next), PREVIEW_DEBOUNCE_MS);
    onCleanup(() => clearTimeout(timer));
  });

  const owners = {};
  const [answer] = createResource(settled, async (req): Promise<PreviewAnswer | undefined> => {
    const owner = latestOwner(owners, "preview", graphOwner());
    const scope = `${graphMeta()?.root ?? ""}\0${graphEpoch()}`;
    const revision = untrack(dataRev);
    const landed = await readOwned(owner, sharedQueryResult(
      scope,
      `ir-preview\0${req.key}\0${revision}`,
      () => backend().queryRun(req.query, req.view, req.context),
    ));
    if (landed.kind === "stale") return undefined;
    const result = landed.value;
    const diagnostics = (result.diagnostics ?? []).filter((d) => !d.disabled).map((d) => d.message);
    if (req.both) {
      const twin = await bothFamilies(owner, {
        scope, key: `preview\0${req.key}\0${revision}`, query: req.query, view: req.view,
        context: req.context, own: result, hostBlockId: props.hostBlockId,
      });
      if (!twin) return undefined;
      const blocks = previewBlocks(twin.blockGroups);
      return {
        key: req.key, anchor: result.anchor, blocks, pages: twin.both.pages,
        total: twin.both.pageTotal + blocks.length, diagnostics,
        families: {
          pageTotal: twin.both.pageTotal, blockTotal: blocks.length,
          pageMore: twin.both.pageMore, blockMore: twin.both.blockMore,
          pageNote: twin.both.pageNote, blockNote: twin.both.blockNote,
        },
      };
    }
    if (result.anchor === "page") {
      return { key: req.key, anchor: "page", blocks: [], pages: result.pages, total: result.matched_total ?? result.pages.length, diagnostics, families: null };
    }
    const blocks = previewBlocks(without(result.groups, props.hostBlockId));
    return { key: req.key, anchor: "block", blocks, pages: [], total: blocks.length, diagnostics, families: null };
  });

  // The count and rows shown are always for the query the user last SETTLED on; while a newer one is
  // running the older rows stay (dimmed), so the pane does not flash empty on every keystroke.
  const shown = (): PreviewAnswer | undefined => readLatestOr(answer, undefined, "query preview");
  const pending = () => request()?.key !== shown()?.key;

  return (
    <div class="qs-live" role="region" aria-label="Live results" aria-busy={pending() ? "true" : "false"}
      classList={{ "qs-live-pending": pending() }}>
      <Show when={answer.error === undefined} fallback={
        <p class="qs-live-count" role="alert">Results couldn't be loaded: {errorText(answer.error)}</p>}>
        <Show when={shown()} fallback={<p class="qs-live-count" role="status">Running…</p>}>
          {(current) => (
            <>
              <p class="qs-live-count" role="status">
                <Show when={current().families} fallback={<>
                  {current().total} {current().anchor === "page" ? (current().total === 1 ? "page" : "pages") : (current().total === 1 ? "result" : "results")}
                  <Show when={current().total > PREVIEW_ROWS}> (showing the first {PREVIEW_ROWS})</Show>
                </>}>
                  {(families) => <>
                    {families().pageTotal} {families().pageTotal === 1 ? "page" : "pages"} · {families().blockTotal} {families().blockTotal === 1 ? "block" : "blocks"}
                    <Show when={families().pageTotal > PREVIEW_ROWS || families().blockTotal > PREVIEW_ROWS}> (showing the first {PREVIEW_ROWS} of each)</Show>
                  </>}
                </Show>
              </p>
              <Show when={current().diagnostics.length > 0}>
                <p class="qs-live-diagnostics" role="alert">{current().diagnostics.join(" · ")}</p>
              </Show>
              <Show when={current().total === 0 && current().diagnostics.length === 0}>
                <p class="qs-live-empty">Nothing matches these conditions.</p>
              </Show>
              <ul class="qs-live-rows" aria-label="Live results">
                <For each={current().pages.slice(0, PREVIEW_ROWS)}>{(row) => (
                  <li class="qs-live-row qs-live-page"><span class="switcher-kind">page</span> <span>{row.name}</span></li>
                )}</For>
                <For each={current().blocks.slice(0, PREVIEW_ROWS)}>{(row) => (
                  <li class="qs-live-row switcher-row block-result">
                    <SearchResultRow page={row.page} breadcrumb={row.breadcrumb} text={row.text} spans={[]} />
                  </li>
                )}</For>
              </ul>
              <Show when={current().families}>{(families) => <>
                <Show when={families().pageNote}>{(note) => <p class="qs-live-diagnostics" role="note">Pages: {note()}</p>}</Show>
                <Show when={families().blockNote}>{(note) => <p class="qs-live-diagnostics" role="note">Blocks: {note()}</p>}</Show>
                <Show when={families().pageMore}><p class="qs-live-more">More pages match than are shown.</p></Show>
                <Show when={families().blockMore}><p class="qs-live-more">More blocks match than are shown.</p></Show>
              </>}</Show>
            </>
          )}
        </Show>
      </Show>
    </div>
  );
}
