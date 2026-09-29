import { For, Show, createEffect, createMemo, createResource, createSignal, onCleanup, untrack, useContext, type JSX } from "solid-js";
import { mainPages, pageByName, loadFeed, appendFeed, emptyPage, withToday, toLoadablePage, loadRoutedPage, setFeedExtender, formatForBlock, readPageProperty, setPageProperty, appendToTodayJournal, ensureEmptyBlock, insertEmptyChildBlock, insertOutlineAfter, promotePagePreamble, beginPageHeaderEdit, pageHeaderProperties, isBlockMoving, isDirty, isSaving, resolveBlockRef, installPageIdentityNavigation, rekeyPageIdentityByPath, type FeedPage, node as docNode, feedNames, isLoaded, loadedPage } from "../document";
import { sameRoute, pageTargetFromFeedPage, pageTargetFromRoute, pageTargetMatchesLoaded, type PaneRouter } from "../router";
import { PaneContext, focusedRouter, rewritePageTargetAcrossPanes } from "../panes";
import { isFavorite, toggleFavorite, openPageInSidebar, openPageContextMenu, carryDays, showCarryButtons, agendaQuery, contextMenu, renamePageInNavigation } from "../ui";
import { graphEpoch, dataRev, graphMeta } from "../graphSession";
import { captureBinding } from "../binding";
import { graphOwner, latestOwner, readOwned, type Owner } from "../owned";
import { isConflicted } from "../document";
import { carryDay, carryPrevDay, carryDaysBack } from "../carry";
import { backend } from "../backend";
import { pushToast } from "../toasts";
import { ensureJournalTemplateForDay, renameOrMergePage, renameOutcomeMessage, switchGraph } from "../graph";
import { Block, OutlineScopeContext } from "./Block";
import { LinkedReferences } from "./LinkedReferences";
import { UnlinkedReferences } from "./UnlinkedReferences";
import { QueryMacro } from "./Macro";
import { SheetTable } from "./SheetTable";
import { TodayTaskSummary } from "./TodayTaskSummary";
import { selectedThemePresentation } from "../themeGallery";
import { NamespaceCrumb, NamespaceHierarchy } from "./Namespace";
import { aliasNamesOf, visibleBody } from "../render/block";
import { InlineText, PageRef } from "../render/inline";
import { EmojiText } from "../render/emoji";
import { journalTitle, currentDayKey, localDateFromDayKey, localDayKey, localDayRolloverDelay } from "../journal";
import { editingId, endEditForSurface, startEditing } from "../editorController";
import type { JournalFeedPage, RefGroup } from "../types";
import { tagRef } from "../tags";
import { copyGuideIntoGraph, ensureGuidePagesLoaded, isGuidePageName } from "../guide";
import { isPropertiesOnly, splitPagePreamble } from "../editor/properties";
import { shouldOpenTextContextMenu } from "../contextMenuPolicy";
import { PagePropertyValue } from "./PagePropertyValue";
import { PageConflictResolution } from "./ConflictResolution";
import { conflictForPage } from "../conflictQueue";

installPageIdentityNavigation((from, to) => {
  // Rewrite both pinned and formerly pathless routes to the exact file owner.
  // The document working set publishes the new logical name afterward.
  renamePageInNavigation(from, to);
  rewritePageTargetAcrossPanes(from, to);
  rewritePageTargetAcrossPanes({ name: from.name, pageKind: from.pageKind }, to);
});

export const FEED_PAGE = 3;
let journalAsOfDay: number | null = null;
let nextBeforeDay: number | null = null;
let feedGeneration = 0;
let loadingGeneration: number | null = null;
let latestFeedRestart: Promise<unknown | null> | null = null;
let publishedFeedEpoch: number | null = null;
let publishedFeedNames: readonly string[] | null = null;
let feedDone = false;
let pendingFeedRestart = false;
const feedOwners = {};

/** A feed response belongs to one graph and one or more concrete Journals
 * surfaces.  App's watcher supplies a captured owner too, so a response begun
 * before navigation/graph switch cannot update the shared feed store. */
export interface JournalsFeedOwner {
  graphEpoch: number;
  isLive: () => boolean;
}

function feedHasActiveEdit(): boolean {
  // An editor in a sidebar, a page tab, or another split pane is unrelated to
  // the working set that loadFeed replaces.  Only a block owned by a visible
  // feed page is unsafe here.
  return feedNames().some(pageHasActiveEdit);
}

function pageHasActiveEdit(name: string): boolean {
  const edited = editingId();
  return !!(edited && docNode(edited)?.page === name) || isDirty(name) || isSaving(name) || isConflicted(name) || isBlockMoving(name);
}

function responseMatches(day: number, response: JournalFeedPage): boolean {
  return response.as_of_day === day && localDayKey() === day;
}

/** A window that has not yet bound a graph (startup, before `load_graph`
 *  returns) has no journals to read: the backend refuses every graph read with
 *  missing-graph-binding. That refusal is not a failed read — the bind bumps
 *  the graph epoch, which re-runs the Journals route loader. So an unbound
 *  window issues no feed read and reports nothing (og 12e P2). */
function windowUnbound(): boolean {
  return backend().graphBindingGeneration() === 0;
}

function ownerIsLive(owner: JournalsFeedOwner): boolean {
  return graphEpoch() === owner.graphEpoch && owner.isLive();
}

function hasPublishedFeed(epoch: number): boolean {
  return publishedFeedEpoch === epoch && publishedFeedNames === feedNames();
}

/** The single start-over owner for route loads, watcher changes and calendar
 * rollover.  It intentionally keeps the old feed/cursor until a response has
 * passed all ownership checks. */
// Returns the caught backend rejection for the route's initial-error display,
// or null after success, deferral, or a stale owner. Refresh callers show a toast.
function restartJournalFeed(owner: JournalsFeedOwner, retried = false, rollover = false): Promise<unknown | null> {
  if (!ownerIsLive(owner)) return Promise.resolve(null);
  const pending = runJournalFeedRestart(owner, retried, rollover);
  latestFeedRestart = pending;
  void pending.then(() => {
    if (latestFeedRestart === pending) latestFeedRestart = null;
  }, () => {
    if (latestFeedRestart === pending) latestFeedRestart = null;
  });
  return pending;
}

let journalRefreshFlight: { graphEpoch: number; day: number; owner: JournalsFeedOwner; promise: Promise<unknown | null> } | null = null;

/** Ensure today's configured template before any feed read for that day. */
async function refreshJournalFeedForCurrentDay(owner: JournalsFeedOwner): Promise<unknown | null> {
  if (!ownerIsLive(owner) || windowUnbound()) return null;
  const date = new Date();
  const day = localDayKey(date);
  const rollover = journalAsOfDay !== null && journalAsOfDay !== day && feedNames().length > 0;
  if (!graphMeta()?.default_journal_template) return restartJournalFeed(owner, false, rollover);
  const current = journalRefreshFlight;
  if (current && current.graphEpoch === owner.graphEpoch && current.day === day && ownerIsLive(current.owner)) {
    current.owner = owner;
    return current.promise;
  }
  ++feedGeneration;
  if (!rollover && feedHasActiveEdit()) { pendingFeedRestart = true; return null; }
  const flight = { graphEpoch: owner.graphEpoch, day, owner, promise: Promise.resolve<unknown | null>(null) };
  flight.promise = (async () => {
    const ensured = await ensureJournalTemplateForDay(date, () => ownerIsLive(flight.owner)
      && !pageHasActiveEdit(journalTitle(date)) && (rollover || !feedHasActiveEdit()));
    const liveOwner = flight.owner;
    if (typeof ensured === "object") {
      if (ownerIsLive(liveOwner)) {
        pendingFeedRestart = true;
        pushToast("Could not load journal feed. It will retry when the view refreshes.", "error");
      }
      return ensured.error;
    }
    if (ensured !== "ready" || localDayKey() !== day || !ownerIsLive(liveOwner)) {
      if (ownerIsLive(liveOwner)) pendingFeedRestart = true;
      return null;
    }
    if (!rollover && feedHasActiveEdit()) { pendingFeedRestart = true; return null; }
    return restartJournalFeed(liveOwner, false, rollover);
  })();
  journalRefreshFlight = flight;
  try { return await flight.promise; }
  finally { if (journalRefreshFlight === flight) journalRefreshFlight = null; }
}

async function runJournalFeedRestart(owner: JournalsFeedOwner, retried: boolean, rollover: boolean): Promise<unknown | null> {
  // An already-dead watcher/surface must be entirely inert.  In particular it
  // must not steal the generation from a live request that is about to land.
  if (!ownerIsLive(owner) || windowUnbound()) return null;
  const generation = ++feedGeneration; // invalidate starts/appends before checking edit safety
  const requestOwner = latestOwner(feedOwners, "restart", graphOwner(() => ownerIsLive(owner)));
  if (!rollover && feedHasActiveEdit()) {
    pendingFeedRestart = true;
    return null;
  }
  const browserDay = localDayKey();
  loadingGeneration = generation;
  try {
    const result = await readOwned(requestOwner, backend().journalFeedPage(FEED_PAGE, null));
    if (result.kind === "stale") return null;
    const response = result.value;
    if (!responseMatches(browserDay, response)) {
      if (generation === feedGeneration && ownerIsLive(owner) && !retried && (rollover || !feedHasActiveEdit())) {
        return restartJournalFeed(owner, true, rollover);
      }
      // A stale/disposed owner cannot create deferred work for a later surface.
      if (generation === feedGeneration && ownerIsLive(owner)) pendingFeedRestart = true;
      return null;
    }
    // Clear the deferred flag before loadFeed synchronously updates doc.feed;
    // otherwise the intentionally reactive pending-retry effect observes the
    // old true value during that store write and starts a duplicate restart.
    if (generation !== feedGeneration || !ownerIsLive(owner) || (!rollover && feedHasActiveEdit())) {
      if (generation === feedGeneration && ownerIsLive(owner)) pendingFeedRestart = true;
      return null;
    }
    pendingFeedRestart = false;
    if (!loadFeed(withToday(response.pages), {
      endEdit: false,
      preserveExisting: rollover,
      isRequestLive: () => generation === feedGeneration && ownerIsLive(owner) && responseMatches(browserDay, response),
    })) return null;
    publishedFeedEpoch = owner.graphEpoch;
    publishedFeedNames = feedNames();
    journalAsOfDay = response.as_of_day;
    nextBeforeDay = response.next_before_day;
    feedDone = response.done;
    return null;
  } catch (error) {
    // A failed refresh must leave the displayed feed and its cursor usable.
    // Focus, visibility, load-more, or the next calendar check will retry.
    if (generation === feedGeneration && ownerIsLive(owner)) {
      pendingFeedRestart = true;
      pushToast("Could not load journal feed. It will retry when the view refreshes.", "error");
    }
    return error;
  } finally {
    if (loadingGeneration === generation) loadingGeneration = null;
  }
}

// Page properties NOT shown in the under-title property list: `alias` is surfaced
// as "aka" chips above, and `icon` is consumed as the page icon next to the title
// (OG hides it too). Other internal/metadata page props could be added here.
const PAGE_PROPS_HIDDEN = new Set(["alias", "icon", "tine.tag-table"]);
const TAG_TABLE_PROP = "tine.tag-table";

function paneContextFromContext() {
  const ctx = useContext(PaneContext);
  return ctx ?? { paneId: "main", router: focusedRouter() };
}

// OG always shows today's journal at the top of the feed, even with no file yet
// (the file is created lazily on first edit — Tine writes on save). So prepend
// an empty today page unless the newest journal on disk already is today.
export { withToday, toLoadablePage } from "../document";

/** Refresh the shared Journals feed from the backend's first page, ensuring a
 * configured daily template first. Cost follows the journal inventory scan,
 * returned page blocks, and at most one template write. A dead owner or active
 * feed edit leaves the visible feed unchanged. A deferred/stale template attempt
 * postpones the feed read; a template or feed failure keeps the old feed,
 * schedules retry on edit release, focus/visibility or day rollover, and
 * shows an error toast. This call does not schedule an immediate retry. */
export async function reloadJournalsFeedFromStart(owner: JournalsFeedOwner): Promise<void> {
  await refreshJournalFeedForCurrentDay(owner);
}

/** Render the active pane route. Ordinary page routes fetch a file; a missing
 * page becomes empty and editable, while a read failure shows an error. Guide
 * routes load bundled pages. Journal routes wait for the live feed read when an
 * older startup read is superseded. In-place refresh keeps an existing feed
 * visible; a route load shows a placeholder until its read settles. A read
 * failure with no existing feed shows an error. Route ownership discards stale
 * loads. Working-set admission costs O(loaded pages + admitted page blocks +
 * cached sheet dimensions + evicted blocks). Guide routes fetch all bundled
 * Guide pages; journal feeds scan/sort inventory and admit returned blocks.
 * Reference and query children, including Agenda and optional tag tables,
 * may await whole-graph parsing and traverse graph-wide blocks. */
export function PageView(): JSX.Element {
  const pane = paneContextFromContext();
  const router = pane.router;
  // Route equality alone is not a component lifetime: a pane can disappear
  // while its router still says Journals.  Every owner issued by this surface
  // carries this revocation token, so an already-issued IPC can finish but can
  // never change the shared feed after unmount.
  let surfaceAlive = true;
  onCleanup(() => { surfaceAlive = false; });
  const [ready, setReady] = createSignal(false);
  // Keep the route whose asynchronous load actually completed separate from the
  // router's desired route. A cached large page is already present in doc.pages;
  // rendering directly from currentRoute() let it mount once immediately and
  // again around the loader transition. Besides doubling warm-navigation work,
  // an older rejected request could replace a newer page with its error state.
  const [loadedRoute, setLoadedRoute] = createSignal<ReturnType<PaneRouter["route"]> | null>(null);
  const [loadError, setLoadError] = createSignal<string | null>(null);

  // Depend on the active route BY VALUE: opening a background tab (or pinning /
  // reordering / closing another tab) mutates the `tabs` signal but not the active
  // route — without this, route() would re-fire this loader, remount the feed via
  // setReady(false), and reset scroll to the top.
  const currentRoute = createMemo(() => router.route(), undefined, { equals: sameRoute });
  const journalOwner = (route = currentRoute(), epoch = graphEpoch(), tabId = router.activeId(), revision = router.routeIntentRevision()): JournalsFeedOwner => ({
    graphEpoch: epoch,
    isLive: () => surfaceAlive && router.activeId() === tabId && router.routeIntentRevision() === revision && sameRoute(currentRoute(), route),
  });
  createEffect(() => {
    const r = currentRoute();
    const epoch = graphEpoch(); // reload when the open graph changes
    const tabId = router.activeId();
    const revision = router.routeIntentRevision();
    const owned = () => surfaceAlive && epoch === graphEpoch()
      && router.activeId() === tabId && router.routeIntentRevision() === revision && sameRoute(currentRoute(), r);
    const routeOwner = graphOwner(owned);
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
        if (r.kind === "query" || r.kind === "pdf" || r.kind === "invalid" || r.kind === "conflicts") {
          // Query workspaces are rendered by PaneLeaf, not PageView. Keep this
          // guard so the page loader never interprets a virtual route as a file.
          setLoadedRoute(r);
          setReady(true);
          return;
        } else if (r.kind === "journals") {
          // Before the window binds its graph there is nothing to read; stay
          // loading. The bind's epoch bump re-runs this loader.
          if (untrack(windowUnbound)) return;
          // restartJournalFeed synchronously reads the working set safety gate.
          // Keep those reads out of this route/epoch loader's dependency set:
          // loadFeed replaces doc.feed, and subscribing here would self-reload.
          const initialRead = untrack(() => refreshJournalFeedForCurrentDay(journalOwner(r, epoch, tabId, revision)));
          const initialGeneration = feedGeneration;
          let feedError = await initialRead;
          if (!owned()) return;
          // A watcher or second Journals surface can supersede this read while
          // both native calls wait on the initial store publication. The older
          // request is correctly discarded, but its route must await the winner.
          while (!hasPublishedFeed(epoch) && latestFeedRestart && owned()) {
            feedError = await latestFeedRestart;
          }
          if (!owned()) return;
          // A superseding owner may have disappeared before publishing. The
          // still-visible route takes one fresh read in that case.
          if (!hasPublishedFeed(epoch) && feedGeneration !== initialGeneration && !pendingFeedRestart) {
            feedError = await refreshJournalFeedForCurrentDay(journalOwner(r, epoch, tabId, revision));
          }
          if (!owned()) return;
          if (!hasPublishedFeed(epoch)) throw feedError ?? new Error("Journal feed read failed.");
        } else {
          if (isGuidePageName(r.name)) {
            await ensureGuidePagesLoaded(true);
            if (!owned()) return;
            setLoadedRoute(r);
            setReady(true);
            router.restoreScrollFor(r);
            return;
          }
          // A path-pinned route (#21) loads that SPECIFIC file — the way to reach a
          // duplicate-day stray that shares a (kind,name) with the canonical day;
          // everything else resolves by name as before.
          // This is a snapshot for the route request, not an effect dependency:
          // loadRoutedPage publishes loadedPage below and must not restart us.
          const loadedPath = r.path ? undefined : untrack(() => loadedPage(r.name)?.id);
          const result = await readOwned(routeOwner, r.path || loadedPath
            ? backend().getPageByPath(r.path ?? loadedPath!)
            : backend().getPage(r.name, r.pageKind));
          if (result.kind === "stale") return;
          const dto = result.value;
          // A saved path can use another case spelling on a case-insensitive
          // volume. Adopt the file's disk spelling in tabs, Recent, and the
          // route before loading it into the exact-path working set.
          if (r.path && dto?.id && dto.id !== r.path && dto.kind === r.pageKind
            && dto.id.toLowerCase() === r.path.toLowerCase()) {
            const from = { name: r.name, pageKind: r.pageKind, path: r.path };
            const to = { name: dto.name, pageKind: dto.kind, path: dto.id };
            renamePageInNavigation(from, to);
            router.rewritePageTarget(from, to);
            return;
          }
          if (dto?.id && dto.kind === r.pageKind && dto.name !== r.name
              && dto.id === (r.path ?? loadedPath)) {
            if (!rekeyPageIdentityByPath(dto.id, dto.name, dto.rev ?? null)
                && loadedPage(r.name)?.id === dto.id) {
              throw new Error("The selected physical page has an active edit or conflicting identity.");
            }
            const from = { name: r.name, pageKind: r.pageKind, ...(r.path ? { path: r.path } : {}) };
            const to = { name: dto.name, pageKind: dto.kind, path: dto.id };
            renamePageInNavigation(from, to);
            rewritePageTargetAcrossPanes(from, to);
            return;
          }
          if (r.path && (!dto || dto.id !== r.path || dto.name !== r.name || dto.kind !== r.pageKind)) {
            throw new Error("The selected physical page is no longer available at that path.");
          }
          // Core page identity is Unicode-case-insensitive while display names
          // preserve their original spelling. Alias-map warmup normally
          // canonicalizes before navigation; this adoption also covers an early
          // click or restored route that raced that map. Re-route once so the
          // exact-keyed working set, tab history, Recent, and editor all own the
          // backend's canonical display name instead of a phantom case variant.
          if (dto && !r.path && r.pageKind === "page" && dto.name !== r.name) {
            renamePageInNavigation(r.name, dto.name);
            router.replaceActiveRoute({ ...r, name: dto.name });
            return;
          }
          // null = page doesn't exist yet → start a fresh empty page. A failed
          // read throws and is caught below, so we never overwrite a page whose
          // load errored with empty content.
          loadRoutedPage(dto ? toLoadablePage(dto, r.name) : emptyPage(r.name, r.pageKind));
          if (r.path && pageByName(r.name)?.id !== r.path)
            throw new Error("The selected file cannot replace a page with an active edit or unsaved changes.");
        }
        if (!owned()) return;
        setLoadedRoute(r);
        setReady(true);
        // Put the scroll back where it was when we last left this entry (back/
        // forward, or returning to this tab). A new page has no saved offset → top.
        router.restoreScrollFor(r);
      } catch (e) {
        if (!owned()) return;
        setLoadedRoute(r);
        setLoadError(String(e));
        setReady(true);
      }
    })();
  });

  const loadMore = async () => {
    const route = currentRoute();
    if (route.kind !== "journals") return;
    const owner = journalOwner(route);
    if (!ownerIsLive(owner)) return;
    if (pendingFeedRestart || journalAsOfDay !== localDayKey()) {
      await refreshJournalFeedForCurrentDay(owner);
      return;
    }
    if (loadingGeneration !== null || feedDone || nextBeforeDay === null) return;
    const generation = feedGeneration;
    const requestOwner: Owner = graphOwner(() => generation === feedGeneration && ownerIsLive(owner));
    const asOfDay = journalAsOfDay;
    const cursor = nextBeforeDay;
    loadingGeneration = generation;
    try {
      const result = await readOwned(requestOwner, backend().journalFeedPage(FEED_PAGE, cursor));
      if (result.kind === "stale") return;
      const response = result.value;
      if (
        generation !== feedGeneration || !ownerIsLive(owner) || asOfDay === null ||
        cursor !== nextBeforeDay || response.as_of_day !== asOfDay || !responseMatches(asOfDay, response)
      ) {
        if (generation === feedGeneration && ownerIsLive(owner)) await refreshJournalFeedForCurrentDay(owner);
        return;
      }
      if (response.pages.length) {
        appendFeed(response.pages);
        publishedFeedNames = feedNames();
      }
      nextBeforeDay = response.next_before_day;
      feedDone = response.done;
    } catch {
      if (generation === feedGeneration && ownerIsLive(owner)) pendingFeedRestart = true;
    } finally {
      if (loadingGeneration === generation) loadingGeneration = null;
    }
  };

  // Local calendar rollover is a one-shot revalidation.  Calendar construction
  // (rather than 24h arithmetic) remains correct on DST transitions.
  createEffect(() => {
    const route = currentRoute();
    if (route.kind !== "journals") return;
    const owner = journalOwner(route);
    let timer: number | undefined;
    let disposed = false;
    const restart = () => { void refreshJournalFeedForCurrentDay(owner); };
    const arm = () => {
      if (disposed || !ownerIsLive(owner)) return;
      const now = new Date();
      timer = window.setTimeout(() => {
        // One-shot rather than 24h arithmetic (DST-safe).  Re-arm after every
        // trigger, including a deferred/error response, while this owner lives.
        void refreshJournalFeedForCurrentDay(owner).finally(() => { if (!disposed && ownerIsLive(owner)) arm(); });
      }, localDayRolloverDelay(now));
    };
    arm();
    const onFocus = () => {
      if (journalAsOfDay !== localDayKey() || pendingFeedRestart) restart();
    };
    const onVisibility = () => { if (!document.hidden) onFocus(); };
    window.addEventListener("focus", onFocus);
    document.addEventListener("visibilitychange", onVisibility);
    onCleanup(() => {
      disposed = true;
      if (timer !== undefined) window.clearTimeout(timer);
      window.removeEventListener("focus", onFocus);
      document.removeEventListener("visibilitychange", onVisibility);
    });
  });

  // A deferred watcher/midnight refresh is retried only when the existing edit
  // lifecycle has advanced; it never replaces a dirty feed with a stale window.
  createEffect(() => {
    const route = currentRoute();
    if (route.kind !== "journals") return;
    editingId();
    dataRev();
    // Subscribe to every page-scoped safety gate even before a watcher marks a
    // retry pending.  That makes the corresponding release event sufficient;
    // no unrelated graph change is needed to wake a deferred feed refresh.
    const unsafe = feedHasActiveEdit();
    if (pendingFeedRestart && !unsafe) {
      // This effect deliberately tracks the edit/conflict/save lifecycle.  Do
      // not untrack it with the initial route loader: it is the pending retry.
      void refreshJournalFeedForCurrentDay(journalOwner(route));
    }
  });

  createEffect(() => {
    if (currentRoute().kind !== "journals") return;
    const extender = async () => {
      const before = feedNames().length;
      await loadMore();
      return feedNames().length > before;
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
    if (!isLoaded() || feedNames().length === 0) return; // re-runs when the feed populates
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
    const r = loadedRoute() ?? currentRoute();
    if (r.kind === "journals") return mainPages();
    if (r.kind !== "page") return [];
    const p = pageByName(r.name);
    const target = pageTargetFromRoute(r);
    return p && target && pageTargetMatchesLoaded(target, p) ? [p] : [];
  };
  const zoomValid = () => {
    const r = currentRoute();
    if (r.kind !== "page" || !r.block) return null;
    return resolveBlockRef({
      uuid: r.block,
      page: r.name,
      pageKind: r.pageKind,
      ...(r.path ? { path: r.path } : {}),
    });
  };
  const contentReady = () => {
    const r = loadedRoute();
    return !!r && ready() && sameRoute(r, currentRoute()) && (r.kind !== "journals" || isLoaded());
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
    <Show when={contentReady()} fallback={
      <div class="page-loading" role="status" aria-live="polite">
        <span class="page-loading-spinner" aria-hidden="true" />
        <span>Loading page…</span>
      </div>
    }>
      <Show when={zoomValid()} fallback={
        <div class="page">
          <For each={pagesToRender()}>
            {(p, i) => (
              <PageSection page={p}>
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
              </PageSection>
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
  const pane = paneContextFromContext();
  const router = pane.router;
  const ancestors = (): string[] => {
    const out: string[] = [];
    let p = docNode(props.id)?.parent ?? null;
    while (p !== null) {
      out.unshift(p);
      p = docNode(p).parent;
    }
    return out;
  };
  const pageName = () => docNode(props.id)?.page ?? "";
  const pageKind = () => loadedPage(pageName())?.kind ?? "page";
  const pageTarget = () => {
    const owner = pageByName(pageName());
    return owner ? pageTargetFromFeedPage(owner) : { name: pageName(), pageKind: pageKind() };
  };
  const crumb = (id: string) => visibleBody(docNode(id).raw)[0] || "…";
  const editSurface = () => pane.paneId === "main" ? "main" : `pane:${pane.paneId}`;
  const focusTrailing = () => {
    const root = docNode(props.id);
    if (!root || pageByName(root.page)?.readOnly || pageByName(root.page)?.guide) return;
    // GH #158: always append a fresh child (never reuse the trailing empty leaf), so
    // the affordance can always add a new last block even when the current last one
    // is an empty, possibly deeper-nested, bullet.
    const id = insertEmptyChildBlock(props.id, root.children.length);
    if (id) startEditing(id, 0, null, editSurface());
  };

  return (
    <div class="page zoomed-page">
      <div class="zoom-breadcrumb">
        <a
          class="crumb crumb-page"
          onClick={() => router.openPageTarget(pageTarget())}
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
        <OutlineScopeContext.Provider value={{ roots: [props.id], forceExpandedRoot: props.id }}>
          <Block id={props.id} forceExpanded />
        </OutlineScopeContext.Provider>
      </div>
      <Show when={!pageByName(pageName())?.readOnly && !pageByName(pageName())?.guide}>
        <TrailingBlockTarget onActivate={focusTrailing} />
      </Show>
    </div>
  );
}

function PageSection(props: { page: FeedPage; children?: JSX.Element }): JSX.Element {
  const pane = paneContextFromContext();
  const router = pane.router;
  const [renaming, setRenaming] = createSignal(false);
  const [newName, setNewName] = createSignal("");
  let renameInFlight = false;
  let renameSubmitted = false;
  let renameCancelled = false;
  let pageActionsTrigger: HTMLButtonElement | undefined;
  const pageTarget = () => pageTargetFromFeedPage(props.page);
  const pageActionsOpen = () => {
    const menu = contextMenu();
    return menu?.kind === "page"
      && menu.name === props.page.name
      && menu.pageKind === props.page.kind
      && !!menu.fileActions
      && menu.focusOwner === pageActionsTrigger;
  };
  const firstPropertiesId = () => {
    if (props.page.format !== "md") return null;
    const id = props.page.roots[0];
    // Keep the first block mounted until editing ends. `alias::` already parses
    // as a properties-only block before its value is typed; hiding it at the
    // second colon unmounted the textarea and discarded the rest of the user's
    // keystrokes (GH #62's regression after the GH #86 presentation change).
    return id && editingId() !== id && docNode(id) && isPropertiesOnly(docNode(id).raw) ? id : null;
  };
  // The page header shows the same answerer the properties panel lists; a first
  // root rendered as an ordinary block (being edited, or not a header) is excluded.
  const headerProperties = () => pageHeaderProperties(props.page, firstPropertiesId() ? null : props.page.roots[0] ?? null);
  const rootsToRender = () => firstPropertiesId() ? props.page.roots.slice(1) : props.page.roots;
  const preambleContent = () => props.page.format === "md" ? splitPagePreamble(props.page.preBlock).content : null;
  const editSurface = () => pane.paneId === "main" ? "main" : `pane:${pane.paneId}`;
  const editPreamble = () => {
    const id = promotePagePreamble(props.page.name);
    if (id) startEditing(id, docNode(id).raw.length);
  };
  const editPageHeader = (event?: MouseEvent) => {
    if (event?.target instanceof Element && event.target.closest("a, button")) return;
    const id = beginPageHeaderEdit(props.page.name);
    if (id) startEditing(id, docNode(id).raw.length, null, editSurface());
  };
  const startRename = () => {
    if (renameInFlight) return;
    if (props.page.guide || props.page.readOnly) return;
    if (props.page.kind !== "page") return; // journals are named by their date
    renameSubmitted = false;
    renameCancelled = false;
    setNewName(props.page.name);
    setRenaming(true);
  };
  const commitRename = async () => {
    if (renameSubmitted || renameCancelled || renameInFlight) return;
    const next = newName().trim();
    const from = props.page.name;
    const target = pageTarget();
    const route = router.route();
    const binding = captureBinding();
    const root = graphMeta()?.root;
    const tabId = router.activeId();
    const intentRevision = router.routeIntentRevision();
    // The rename's own refresh (`refreshAfterRename`) removes the renamed page
    // from every tab's history without a navigation intent, so a tab showing it
    // falls back to its previous entry — any page, not only the journals. That
    // move is ours, not the user's: while no navigation intent intervened, a tab
    // that showed the renamed page still belongs to this rename.
    const routeShowsRenamed = route.kind === "page" && route.name === target.name
      && route.pageKind === target.pageKind && (target.path === undefined || route.path === target.path);
    const stillOnRenameTab = () => {
      const current = router.route();
      return router.activeId() === tabId
        && router.routeIntentRevision() === intentRevision
        && binding.backendGeneration === captureBinding().backendGeneration
        && graphMeta()?.root === root
        && (routeShowsRenamed || sameRoute(current, route) || current.kind === "journals"
          || (current.kind === "page" && current.name === next && current.pageKind === "page"));
    };
    renameSubmitted = true;
    setRenaming(false);
    if (!next || next === from) return;
    renameInFlight = true;
    try {
      const outcome = await renameOrMergePage(from, next, target);
      if (outcome === "cancelled") return;
      const message = renameOutcomeMessage(outcome, from, next);
      if (message) {
        if (stillOnRenameTab()) pushToast(message, outcome === "unchanged" ? "info" : "error");
        return;
      }
      if (stillOnRenameTab()) router.openPage(next, "page");
    } catch (e) {
      if (stillOnRenameTab()) alert(`Rename failed: ${String(e)}`);
    } finally {
      renameInFlight = false;
    }
  };

  // Theme API 0.2 (master 1488588b8): the editorial header and the compact
  // task summary apply to today's journal only. og marks the title row, not
  // the section (whose opening tag the I-20 async-ownership guard anchors on).
  const isTodayJournal = () => props.page.kind === "journal"
    && props.page.name === journalTitle(localDateFromDayKey(currentDayKey()));
  return (
    <div class="page-section">
      <Show when={props.page.kind === "page"}>
        <NamespaceCrumb name={props.page.name} />
      </Show>
      <div class="page-title-row" classList={{ "journal-today": isTodayJournal() }}>
        <div class="page-title-main">
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
                else if (e.key === "Escape") {
                  renameCancelled = true;
                  setRenaming(false);
                }
              }}
              onBlur={() => void commitRename()}
            />
          }
        >
          <h1
            class="page-title"
            classList={{ "journal-title": props.page.kind === "journal" }}
            title={props.page.guide ? "Bundled Guide page" : props.page.kind === "page" ? "Double-click to rename (shift-click → sidebar, middle-click → new tab)" : "Shift-click to open in sidebar, middle-click → new tab"}
            onClick={(e) => {
              if (e.shiftKey && !props.page.guide) openPageInSidebar(pageTarget());
              else router.openPageTarget(pageTarget());
            }}
            onAuxClick={(e) => {
              if (e.button === 1) {
                e.preventDefault(); // middle-click → background tab, like a body link
                router.openPageTargetInNewTab(pageTarget());
              }
            }}
            onDblClick={startRename}
            onContextMenu={(e) => {
              if (props.page.guide) return;
              if (!shouldOpenTextContextMenu(e.target)) return;
              e.preventDefault();
              openPageContextMenu(e.clientX, e.clientY, pageTarget(), true);
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
              when={headerProperties()
                .find(([k]) => k.toLowerCase() === "icon")?.[1]
                ?.trim()}
            >
              {(icon) => (
                <span
                  class="page-icon page-title-icon"
                  onClick={(event) => {
                    event.preventDefault();
                    event.stopPropagation();
                    editPageHeader();
                  }}
                >
                  <EmojiText text={icon()} />
                </span>
              )}
            </Show>
            <EmojiText text={props.page.title} />
          </h1>
        </Show>
        <Show when={isTodayJournal() && selectedThemePresentation().todayTaskSummary === "compact"}>
          <TodayTaskSummary page={props.page} />
        </Show>
        </div>
        <div class="page-title-actions">
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
            ref={pageActionsTrigger}
            type="button"
            class="page-actions-trigger"
            data-page-actions-trigger
            title="Page actions"
            aria-label="Page actions"
            aria-haspopup="menu"
            aria-expanded={pageActionsOpen()}
            onClick={(e) => {
              const rect = e.currentTarget.getBoundingClientRect();
              openPageContextMenu(
                rect.left,
                rect.bottom + 4,
                pageTarget(),
                true,
                e.currentTarget,
              );
            }}
          >
            <span aria-hidden="true">⋯</span>
          </button>
          <button
            class="fav-star"
            classList={{ active: isFavorite(props.page.name, props.page.kind) }}
            title={isFavorite(props.page.name, props.page.kind) ? "Unfavorite" : "Add to favorites"}
            onClick={() => toggleFavorite(props.page.name, props.page.kind)}
          >
            <svg viewBox="0 0 24 24" class="star-icon" aria-hidden="true">
              <path
                d="M12 3.5l2.6 5.27 5.82.85-4.21 4.1.99 5.79L12 16.77l-5.2 2.73.99-5.79-4.21-4.1 5.82-.85z"
                fill={isFavorite(props.page.name, props.page.kind) ? "currentColor" : "none"}
                stroke="currentColor"
                stroke-width="1.6"
                stroke-linejoin="round"
              />
            </svg>
          </button>
        </Show>
        </div>
      </div>
      <Show when={aliasNamesOf(headerProperties()).length}>
        <div class="page-aliases" title="Also known as — other names that link here" onClick={editPageHeader}>
          <span class="page-aliases-label">aka</span>
          <For each={aliasNamesOf(headerProperties())}>
            {(a) => <span class="alias-chip"><PageRef name={a} alias={a} /></span>}
          </For>
        </div>
      </Show>
      <Show when={headerProperties().filter(([k]) => !PAGE_PROPS_HIDDEN.has(k.toLowerCase())).length}>
        <div class="page-properties" onClick={editPageHeader}>
          {/* `alias`/`icon` are surfaced elsewhere (chips / title icon) — see PAGE_PROPS_HIDDEN. */}
          <For each={headerProperties().filter(([k]) => !PAGE_PROPS_HIDDEN.has(k.toLowerCase()))}>
            {([key, value]) => (
              <div class="prop-row">
                <span class="prop-key">{key}</span>
                <span class="prop-value">
                  <PagePropertyValue propertyKey={key} value={value} format={props.page.format} />
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
      {/* Concord: a queued conflict is resolved AT the page, block by block. */}
      <Show when={conflictForPage(props.page.id)}>
        {(conflict) => <PageConflictResolution conflict={conflict()} />}
      </Show>
      <div class="page-blocks">
        <Show when={preambleContent()}>
          {(content) => (
            <div class="ls-block preamble-block" data-page-preamble={props.page.name}>
              <div class="block-main">
                <div class="block-controls">
                  <span class="collapse-toggle" />
                  <span class="bullet-container" title="Click the text to turn it into an editable block">
                    <span class="bullet" />
                  </span>
                </div>
                <div class="block-content-wrapper" onClick={editPreamble}>
                  <div class="block-content"><InlineText text={content()} format={props.page.format} /></div>
                </div>
              </div>
            </div>
          )}
        </Show>
        <For each={rootsToRender()}>{(id) => <Block id={id} />}</For>
      </div>
      {props.children}
      <PageTypingTarget page={() => props.page} surface={editSurface()} />
    </div>
  );
}

/** The one answer to "where does the caret go on this page".
 *
 *  Every surface that renders a page's roots renders this too. It re-seeds the
 *  phantom empty bullet whenever the body has nothing in it (the shape a
 *  brand-new day gets; non-dirty until the user types) and offers the trailing
 *  "+ Add block" for appending below the last block. GH #483 is what a surface
 *  without it looks like: a page created and never opened in the main pane, then
 *  opened in the right sidebar, rendered an empty box with nowhere to put a caret.
 *  `ensureEmptyBlock` is the emptiness authority (a page whose only root is its
 *  `key:: value` header counts as empty, and it returns null once a body exists),
 *  so no caller carries its own predicate. */
export function PageTypingTarget(props: {
  page: () => FeedPage | undefined;
  surface?: string | null;
}): JSX.Element {
  const focusTrailing = () => {
    const page = props.page();
    if (!page || page.readOnly || page.guide) return;
    const seeded = ensureEmptyBlock(page.name, { afterProperties: true });
    if (seeded) {
      startEditing(seeded, 0, null, props.surface ?? null);
      return;
    }
    // GH #158: always add a fresh root-level block (never reuse the trailing empty
    // leaf). Reuse stranded users whose last block is an empty *indented* bullet:
    // clicking could only ever re-focus that indented block, never give them a new
    // unindented last block. Stacking empty last blocks is intentionally allowed.
    const roots = page.roots;
    const id = insertOutlineAfter(roots[roots.length - 1], [{ raw: "", children: [] }]);
    if (id) startEditing(id, 0, null, props.surface ?? null);
    else pushToast("Could not add a block to this page.", "error");
  };
  // A page emptied of its last block (explicit Delete bypasses the Backspace
  // last-block guard) would render nothing to type into. `ensureEmptyBlock` is a
  // no-op once a body exists, so this only ever fires on a genuinely empty page.
  createEffect(() => {
    const page = props.page();
    if (!page || page.readOnly) return;
    page.roots.length; // track: a page emptied while rendered must re-seed
    ensureEmptyBlock(page.name, { afterProperties: true });
  });
  return (
    <Show when={props.page() && !props.page()!.readOnly && !props.page()!.guide}>
      <TrailingBlockTarget onActivate={focusTrailing} />
    </Show>
  );
}

function TrailingBlockTarget(props: { onActivate: () => void }): JSX.Element {
  return (
    <button
      type="button"
      class="page-trailing-block-target"
      aria-label="Focus or create a trailing block"
      title="Click to continue writing below this page"
      onClick={props.onActivate}
    >
      <span aria-hidden="true">+ Add block</span>
    </button>
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

async function tagTableGroups(pageName: string, owners: object): Promise<{ groups: RefGroup[]; error?: string } | undefined> {
  const owner = latestOwner(owners, "tag-table", graphOwner());
  try {
    const reading = await readOwned(owner, backend().parseQuery(tagQuery(pageName), "macro_query"));
    if (reading.kind === "stale") return undefined;
    const result = await readOwned(owner, backend().queryRun(reading.value.query, reading.value.view));
    if (result.kind === "stale") return undefined;
    const answer = result.value;
    const diagnostic = (answer.diagnostics ?? []).find((item) => !item.disabled);
    return diagnostic
      ? { groups: [], error: diagnostic.message }
      : { groups: answer.anchor === "block" ? answer.groups : [] };
  } catch (error) {
    return { groups: [], error: error instanceof Error ? error.message : String(error) };
  }
}

export function TagTableToggle(props: { page: FeedPage }): JSX.Element {
  const owners = {};
  const [groups] = createResource(
    () => (props.page.kind === "page" ? `${props.page.name}\0${dataRev()}` : null),
    () => tagTableGroups(props.page.name, owners)
  );
  const enabled = () => tagTableEnabled(props.page.name);
  const visible = () => props.page.kind === "page" && (enabled() || taggedCount(groups()?.groups) > 0);
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
  const owners = {};
  const [groups] = createResource(
    () => `${props.pageName}\0${dataRev()}`,
    () => tagTableGroups(props.pageName, owners)
  );
  const addRow = async () => {
    const ok = await appendToTodayJournal(`${tagRef(props.pageName)} `);
    if (!ok) return;
    const today = pageByName(journalTitle(new Date()));
    const id = today?.roots[today.roots.length - 1];
    if (id && docNode(id)) startEditing(id, docNode(id).raw.length);
  };
  return (
    <div class="tag-page-table">
      <Show when={!groups()?.error} fallback={<div role="alert">Tag table couldn't load: {groups()?.error}</div>}>
        <SheetTable
          ownerId={`tag-page:${encodeURIComponent(props.pageName)}`}
          rowSource="query"
          groups={groups()?.groups ?? []}
          addRow={addRow}
          addRowLabel={`Add ${tagRef(props.pageName)} row`}
          schemaPage={props.pageName}
        />
      </Show>
    </div>
  );
}

/** Journal carry controls; named pages render none. Rendering reads the local
 * day and saved preference in O(1). Previous-day selection fetches all content
 * days and sorts O(J log J); last-N starts N day-page lookups. A carry groups
 * saves of today and every source page that supplied tasks. Failed grouped
 * saves retain moved tasks in the editor and toast. Source-page and inventory
 * read failures also toast; no earlier day gives an info toast. N is not
 * validated. */
export function CarryActions(props: { page: FeedPage }): JSX.Element {
  const isJournal = () => props.page.kind === "journal";
  const isToday = () => isJournal() && props.page.name === journalTitle(localDateFromDayKey(currentDayKey()));
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
