import { FailureBoundary } from "./FailureBoundary";
import { TableWrap } from "./TableWrap";
import { For, Show, Switch, Match, createMemo, createResource, createSignal, useContext, createUniqueId, onCleanup, onMount, type JSX } from "solid-js";
import { setBoardGroupBy } from "../sheet/mutations";
import { backend } from "../backend";
import { isPublishedExport } from "../publishedBackend";
import { openPageTarget, openPageAtBlock, openPageTargetInNewTab, openInNewTab } from "../router";
import { openPageInSidebar, openBlockInSidebar, pageIdentityKey, openQueryExport, switcherOpen } from "../ui";
import { dataRev, graphEpoch, graphMeta } from "../graphSession";
import { bindingOwner, advanceRevision, graphOwner, latestOwner, readOwned, revisionOwner, writeOwned, type Owned } from "../owned";
import { blockProperty, blockWritable, formatForBlock, graphRewriteFrozen, pageByName, resolveGuidePageDto, setBlockProperty, setRaw, undo, undoTopTag, withUndoUnit, node as docNode } from "../document";
import { resolveBlockBatched } from "../resolveBatch";
import { LiveRefGroup } from "./LiveRefGroup";
import { QueryGroups } from "./QueryGroup";
import { QueryBuilder, type BuilderSession } from "./QueryBuilder";
import { CrossingNotice } from "./CrossingNotice";
import { SearchResultRow } from "./SearchResultRow";
import { editEdnTitle, readEdnOptions } from "../editor/edn";
import { columnKey, compareCells, hostColumns, hostSort, type TableSort } from "./legacyQueryTable";
import { queryMacroExtent, queryMacroExtents, type MacroExtent } from "../editor/queryMacro";
import { QUERY_MACRO_NAMES } from "../editor/queryMacroName";
import {
  macroPrintDialect,
  macroTextDialect,
  sourceOptions,
  sourceOriginal,
  sourcePrintDialect,
  VIEW_KINDS,
  type ExecutionContext,
  type ExplainEmptyResult,
  type ParsedQuery,
  type Query,
  type QueryPrintDialect,
  type Source,
  type ViewSettings,
} from "../editor/queryIr";
import { visibleBody } from "../render/block";
import { facetsOf } from "../render/facets";
import { sheetConfig } from "../sheet/config";
import { SheetTable } from "./SheetTable";
import { SheetBoard } from "./SheetBoard";
import { SheetContainer } from "./SheetContainer";
import { QueryResultSections } from "./QueryResultSections";
import { QueryPageRows, QueryStatisticsSummary, type QueryView } from "./QueryResultParts";
import type { PageKind, QueryHit, RefGroup } from "../types";
import { sharedQueryResult } from "../queryResultCache";
import { bothFamilies } from "../queryTwin";
import { declaresCurrentPageInput, queryCurrentPage } from "../queryCurrentPage";
import { savedDslToFriendlySearch } from "../editor/searchQuery";
import { displayPropertyPatch, isLegacyBareColumnList, mergeQueryAggregateValue } from "../editor/queryViewProperties";
import { formulasOf } from "../sheet/formulaFields";
import { LinkDepthContext, LinkDepthWarning, MAX_DEPTH_OF_LINKS } from "./linkDepth";
import { blockDtoExternalId } from "../blockIdentity";
import { QueryPrintRefusedError } from "../backend";
import { readLatestOr, readOr } from "../resourceRead";
import { focusedRouter, openRouteInOtherPane } from "../panes";
import { internalLinkAuxClick, internalLinkDest, internalLinkMouseDown } from "../linkGesture";
import { pushToast, pushToastUnique } from "../toasts";
import { ExternalLink } from "./ExternalLink";
import {
  boundedFeature, cellText, withoutHostBlock, PAGES_AND_BLOCKS, RESULT_KINDS_PROPERTY,
  type BothFamilies, type QueryOperation, type Row,
} from "./queryMacroSupport";
export { boundedFeature, withoutHostBlock, PAGES_AND_BLOCKS, RESULT_KINDS_PROPERTY };
import { QueryLegacyTable } from "./QueryLegacyTable";
import { documentOf, mainWindow } from "../windowRealm";

const QUERY_VIEW_LABEL: Record<QueryView, string> = {
  search: "Search",
  list: "List",
  table: "Table",
  board: "Board",
};

// Collapsed state for query results, keyed by graph + rendered query identity.
// A raw query-string key made unrelated dashboards across pages/graphs collide.
const QCOLLAPSE_KEY = "logseq-claude.queryCollapsed";
function loadCollapsed(key: string): boolean | null {
  try {
    const m = JSON.parse(localStorage.getItem(QCOLLAPSE_KEY) ?? "{}");
    return typeof m[key] === "boolean" ? m[key] : null;
  } catch {
    return null;
  }
}
function saveCollapsed(key: string, v: boolean) {
  try {
    const m = JSON.parse(localStorage.getItem(QCOLLAPSE_KEY) ?? "{}");
    // Keep explicit false: it overrides a source `:collapsed? true` default on
    // remount. Deleting false made an expanded query re-collapse immediately.
    m[key] = v;
    localStorage.setItem(QCOLLAPSE_KEY, JSON.stringify(m));
  } catch {
    // ignore
  }
}

const errorText = (error: unknown): string => (error instanceof Error ? error.message : String(error));
const sameJson = <T,>(a: T, b: T) => JSON.stringify(a) === JSON.stringify(b);
const CURRENT_PAGE_RE = /<%\s*current page\s*%>/i;
const BLOCK_CHANGED = "The block changed while saving. Try this edit again.";

// Device-local dismissal keyed by graph (D-11): one read per graph binding,
// shared by every crossing in this window (I-12/I-13).
const CROSSING_NOTICE_KEY = "queryCrossingNoticeDismissed";
const [crossingNoticeDismissed, setCrossingNoticeDismissed] = createSignal<boolean | undefined>(undefined);
const crossingNoticePreference = {};
let crossingNoticePrimed: number | undefined;
function primeCrossingNoticePreference(): void {
  if (crossingNoticePrimed === graphEpoch()) return;
  crossingNoticePrimed = graphEpoch();
  setCrossingNoticeDismissed(undefined);
  const revision = advanceRevision(crossingNoticePreference);
  void readOwned(
    revisionOwner(crossingNoticePreference, revision, graphOwner()),
    backend().getAppBool(CROSSING_NOTICE_KEY, false),
  ).then(
    (read) => { if (read.kind === "current") setCrossingNoticeDismissed(read.value); },
    // Recovery over refusal: an unreadable preference costs one extra notice.
    () => setCrossingNoticeDismissed(false),
  );
}
function dismissCrossingNoticeForever(): void {
  const revision = advanceRevision(crossingNoticePreference);
  setCrossingNoticeDismissed(true);
  void writeOwned(
    revisionOwner(crossingNoticePreference, revision, bindingOwner()),
    backend().setAppBool(CROSSING_NOTICE_KEY, true),
  ).catch((error: unknown) => pushToast(`Couldn't save the notice preference: ${errorText(error)}`, "error"));
}
export function resetCrossingNoticeForTests(): void {
  crossingNoticePrimed = undefined;
  advanceRevision(crossingNoticePreference);
  setCrossingNoticeDismissed(undefined);
}

/** One parse request: the authored (or `<% current page %>`-substituted)
 *  argument, the macro name it was written under, and the host block's
 *  `tine.*` properties the engine merges into the view (§4.1). */
interface ReadingRequest {
  argument: string;
  name: string;
  properties: [string, string][];
  epoch: number;
}
interface Reading {
  request: ReadingRequest;
  reading: ParsedQuery;
}
/** The engine's reading of a macro (`query_parse`, I-12), owned by the newest
 *  request (I-20). */
function createQueryReading(request: () => ReadingRequest | undefined) {
  const owners = {};
  const [resource] = createResource(request, async (req): Promise<Reading | undefined> => {
    const landed = await readOwned(
      latestOwner(owners, "parse", graphOwner()),
      backend().parseQuery(req.argument, macroTextDialect(req.name), req.properties),
    );
    return landed.kind === "current" ? { request: req, reading: landed.value } : undefined;
  });
  return resource;
}

/** Present a query macro through the Rust parse/run/print seam. Every mounted,
 *  expanded block re-runs a graph-wide evaluation on each graph save; identical
 *  requests share one in-flight run. Collapsed blocks retain their count without
 *  re-running for unrelated saves. A `(search ...)` source uses bounded graph
 *  search instead. Saves write the printed macro and any view facts the printer
 *  cannot carry as `tine.*` properties in one undo unit. A refused print or
 *  changed source extent leaves the block untouched and shows an error. */
/** Unexpected render/resource failures stay within this query; Retry remounts it. */
export function QueryMacro(props: {
  body: string;
  blockId?: string;
  /** The parsed node's exact raw extent. Every write must still target these bytes. */
  sourceExtent?: MacroExtent;
  sourceRaw?: string;
  title?: string;
  /** Owner page context for read-only surfaces such as BEGIN_QUERY. */
  currentPage?: string;
  /** For advanced sources, hide rows if the report has ignored clauses. (The
   *  engine now refuses a query with any unlowerable clause, so `ignored` is
   *  empty whenever `supported`; kept as a belt-and-braces display guard.) Pair
   *  with `unsupportedLabel` to avoid showing the source. */
  strictAdvanced?: boolean;
  unsupportedLabel?: string;
  // Render nothing when there are no results (the app-inserted journal agenda).
  hideWhenEmpty?: boolean;
}): JSX.Element {
  return <FailureBoundary region="Query results"><QueryMacroContent {...props} /></FailureBoundary>;
}

function QueryMacroContent(props: Parameters<typeof QueryMacro>[0]): JSX.Element {
  const linkDepth = useContext(LinkDepthContext);
  if (linkDepth > MAX_DEPTH_OF_LINKS) return <LinkDepthWarning />;

  // The macro name this query was AUTHORED under (§7.9): `query` or `tine-query`.
  const macroBody = createMemo(() => queryMacroExtent(`{{${props.body.trim()}}}`));
  const sourceError = () => macroBody() === null ? "Query source could not be parsed. Edit the block text to repair the macro." : undefined;
  const macroName = (): string => macroBody()?.name ?? QUERY_MACRO_NAMES[0];
  const arg = () => macroBody()?.argument.trim() ?? "";
  const hostProperties = createMemo<[string, string][]>(() => {
    const id = props.blockId;
    const node = id ? docNode(id) : undefined;
    if (!id || !node) return [];
    return facetsOf(node.raw, formatForBlock(id)).properties.filter(([key]) => key.startsWith("tine.") || key === "query-table");
  }, [], { equals: sameJson });
  const parseRequest = createMemo<ReadingRequest | undefined>(
    () => sourceError() ? undefined : ({ argument: arg(), name: macroName(), properties: hostProperties(), epoch: graphEpoch() }),
    undefined,
    { equals: sameJson },
  );
  const parsed = createQueryReading(parseRequest);
  /** The authoring reading. Every display and editing derivation uses it. */
  const reading = (): ParsedQuery | undefined => (!sourceError() && parsed.error === undefined ? parsed.latest?.reading : undefined);
  const source = (): Source | undefined => reading()?.query.source;
  const form = () => { const s = source(); return (s ? sourceOriginal(s) : null) ?? ""; };
  const opts = () => { const s = source(); return s ? sourceOptions(s) : ""; };
  // Title and collapse are read from the source options map.
  const optionValues = createMemo(() => readEdnOptions(opts()));
  const titleOption = (): string | undefined => optionValues()?.title ?? undefined;
  const isAdvanced = () => source()?.kind === "advanced";
  const bothKinds = (): boolean =>
    hostProperties().some(([key, value]) => key.toLowerCase() === RESULT_KINDS_PROPERTY && value.trim().toLowerCase() === PAGES_AND_BLOCKS);
  const setBothKinds = (on: boolean) => {
    const blockId = props.blockId;
    const node = blockId ? docNode(blockId) : undefined;
    if (!blockId || !node || bothKinds() === on) return;
    withUndoUnit(on ? "query:result-kinds:both" : "query:result-kinds:one", [node.page], () => {
      setBlockProperty(blockId, RESULT_KINDS_PROPERTY, on ? PAGES_AND_BLOCKS : null);
    });
  };

  // GH #301: `<% current page %>` binds the FOCUSED pane's route page and re-runs
  // on navigation. Substitution is execution-only: the builder keeps the dyvar.
  const executionArg = createMemo<string | null>(() => {
    if (!CURRENT_PAGE_RE.test(arg())) return null;
    const route = focusedRouter().route();
    const pageName = route.kind === "page" ? route.name : undefined;
    if (!pageName) return null; // no focused page: leave verbatim, like templates
    return arg().replace(new RegExp(CURRENT_PAGE_RE.source, "gi"), () => `[[${pageName}]]`);
  });
  const executionRequest = createMemo<ReadingRequest | undefined>(() => {
    const argument = executionArg();
    const request = parseRequest();
    return argument === null || !request ? undefined : { ...request, argument };
  }, undefined, { equals: sameJson });
  const executionParsed = createQueryReading(executionRequest);
  /** The reading the EXECUTION runs — for a substituted argument, only the
   *  reading of THAT argument, never the previous page's (I-20). */
  const runnable = (): ParsedQuery | undefined => {
    if (executionArg() === null) return reading();
    if (executionParsed.error !== undefined) return undefined;
    const landed = executionParsed.latest;
    return landed && landed.request.argument === executionArg() ? landed.reading : undefined;
  };
  // A typed advanced `:current-page` input binds OG's current page (#301).
  // Master: a typed `:current-page` input binds the focused pane's page; every
  // other query runs bound to the page the query block is on.
  const executionContext = (): ExecutionContext | undefined => {
    const page = isAdvanced() && declaresCurrentPageInput(form())
      ? queryCurrentPage()
      : props.currentPage ?? (props.blockId ? docNode(props.blockId)?.page : undefined);
    return page ? { current_page: page } : undefined;
  };
  const friendlySearch = createMemo(() => {
    const s = runnable()?.query.source;
    return s?.kind === "og" ? savedDslToFriendlySearch(s.original) : null;
  });

  const sheet = createMemo(() => {
    if (!props.blockId || !docNode(props.blockId)) return null;
    return sheetConfig(facetsOf(docNode(props.blockId).raw, formatForBlock(props.blockId)).properties);
  });
  const currentView = (): QueryView => {
    if (!props.blockId) return "list";
    const view = blockProperty(props.blockId, "tine.view");
    return view === "search" || view === "table" || view === "board" ? view : "list";
  };
  const blockFace = (): QueryView => runnable()?.block_presentation ?? currentView();
  const sheetFace = () => blockFace() === "table" || blockFace() === "board";
  const legacyTable = () => blockFace() === "list"
    && (!props.blockId || blockProperty(props.blockId, "tine.view") === null)
    && (reading()?.legacy_table ?? false);
  const setQueryView = (next: QueryView) => {
    const blockId = props.blockId;
    const node = blockId ? docNode(blockId) : undefined;
    if (!blockId || !node) return;
    const storedView = blockProperty(blockId, "tine.view");
    if ((next === "list" && storedView === null && !legacyTable()) || (next !== "list" && storedView === next)) return;
    withUndoUnit(`query:view:${next}`, [node.page], () => {
      if (next === "list") {
        setBlockProperty(blockId, "tine.view", reading()?.legacy_table ? "list" : null);
        return;
      }
      setBlockProperty(blockId, "tine.view", next);
      if (next === "board" && blockProperty(blockId, "tine.group-by") === null
        && blockProperty(blockId, "tine.group-field") === null) {
        setBlockProperty(blockId, "tine.group-by", "state");
      }
    });
  };

  const currentPage = () => props.currentPage ?? (props.blockId ? docNode(props.blockId)?.page : undefined);
  const collapseKey = () => JSON.stringify([graphMeta()?.root ?? "", props.blockId ?? currentPage() ?? "global", arg()]);
  const [collapseOverride, setCollapseOverride] = createSignal(loadCollapsed(collapseKey()));
  const collapsed = () => collapseOverride() ?? (optionValues()?.collapsed ?? false);
  const toggleCollapsed = () => {
    const v = !collapsed();
    setCollapseOverride(v);
    saveCollapsed(collapseKey(), v);
  };

  // A save anywhere in the graph bumps dataRev after 700 ms of quiet. Each
  // distinct expanded request costs one whole-graph evaluation per revision;
  // sharedQueryResult coalesces identical open blocks into one IPC (including
  // the 20-block case), though each host still owns its rendered rows. A
  // collapsed query fetches once for its count and skips save reruns. The
  // 10k-graph 3/20-block save measurements are in RECEIPT-14Q4A.md.
  const runRequest = createMemo(() => {
    const query = runnable();
    if (!query) return undefined;
    const context = executionContext();
    const search = friendlySearch();
    const both = search === null && bothKinds();
    const displayKey = JSON.stringify([graphEpoch(), query.query, query.view, context ?? null, search, collapsed(), both]);
    const key = `${displayKey}\0${collapsed() ? "collapsed" : dataRev()}`;
    return { query, context, search, displayKey, key, both };
  }, undefined, { equals: (a, b) => a?.key === b?.key });
  const runOwners = {};
  const [operation] = createResource(runRequest, async (request): Promise<QueryOperation | undefined> => {
    const owner = latestOwner(runOwners, "run", graphOwner());
    const scope = `${graphMeta()?.root ?? ""}\0${graphEpoch()}`;
    if (request.search !== null) {
      const searchSource = request.search;
      const landed = await readOwned(owner, sharedQueryResult(
        scope,
        `friendly-search\0${request.key}`,
        () => request.query.page_match_scope
          ? backend().runGraphSearch(
            searchSource, 500, 5_000, `inline-query:${props.blockId ?? currentPage() ?? "global"}`, false,
            undefined, request.query.page_match_scope,
          )
          : backend().runGraphSearch(
            searchSource, 500, 5_000, `inline-query:${props.blockId ?? currentPage() ?? "global"}`, false,
          ),
      ));
      if (landed.kind === "stale") return undefined;
      // The Search presentation renders these hits directly, so the host block
      // comes out here too — the exclusion `withoutHostBlock` makes (GH #469).
      const hits = landed.value.hits.filter((hit) => !(hit.entity === "block" && hit.block.id === props.blockId));
      const grouped = new Map<string, RefGroup>();
      for (const hit of hits) {
        if (hit.entity !== "block") continue;
        const key = `${hit.kind}\0${hit.page}\0${hit.path ?? ""}`;
        const group = grouped.get(key) ?? { page: hit.page, kind: hit.kind, path: hit.path, blocks: [] };
        group.blocks.push(hit.block);
        grouped.set(key, group);
      }
      return {
        requestKey: request.displayKey,
        groups: [...grouped.values()], pages: null, diagnostics: [],
        report: null, search: hits.length === landed.value.hits.length ? landed.value : { ...landed.value, hits }, matchedTotal: null,
        both: null,
      };
    }
    const landed = await readOwned(owner, sharedQueryResult(
      scope,
      `ir\0${request.key}`,
      () => backend().queryRun(request.query.query, request.query.view, request.context),
    ));
    if (landed.kind === "stale") return undefined;
    const result = landed.value;
    let both: BothFamilies | null = null;
    let twinGroups: RefGroup[] = [];
    if (request.both) {
      const twin = await bothFamilies(owner, {
        scope, key: request.key, query: request.query.query, view: request.query.view,
        context: request.context, own: result, hostBlockId: props.blockId, hostProperties: hostProperties(),
      });
      if (!twin) return undefined;
      both = twin.both;
      twinGroups = twin.blockGroups;
    }
    return {
      requestKey: request.displayKey,
      groups: both ? twinGroups : result.anchor === "block" ? withoutHostBlock(result.groups, props.blockId) : [],
      pages: both ? null : result.anchor === "page" ? result.pages : null,
      diagnostics: result.diagnostics ?? [],
      report: result.report,
      statistics: result.statistics,
      search: null,
      matchedTotal: result.matched_total ?? null,
      both,
    };
  });
  /** The last coherent answer; an errored run shows its error, not old rows. */
  const displayed = (): QueryOperation | undefined => {
    const current = runRequest();
    const landed = readLatestOr(operation, undefined, "query run");
    return current && landed?.requestKey === current.displayKey ? landed : undefined;
  };
  const groups = () => displayed()?.groups ?? [];
  const pageRows = () => displayed()?.pages ?? null;
  // The run's OWN diagnostics (I-9): an invalid query returns zero rows plus
  // these, so they must render — "No results" alone would report a broken query
  // as an empty graph.
  const blockingDiagnostics = () => (displayed()?.diagnostics ?? []).filter((d) => !d.disabled);
  const advInfo = () => (isAdvanced() ? displayed()?.report ?? null : null);
  const readError = () => sourceError() ?? parsed.error ?? executionParsed.error;
  const loadError = () => operation.error ?? readError();
  // Presentation never changes membership: ordinary DSL results adapt into
  // evidence-free search rows for the Search presentation.
  const searchPresentationHits = createMemo<QueryHit[]>(() => {
    if (currentView() !== "search" && friendlySearch() === null) return [];
    const search = displayed()?.search;
    if (search) return search.hits;
    return groups().flatMap((group) => group.blocks.map((block) => ({
      entity: "block" as const,
      page: group.page,
      kind: group.kind,
      block,
      display_text: visibleBody(block.raw).join(" "),
      evidence: [],
    })));
  });
  const total = () => {
    const both = displayed()?.both;
    if (both) return both.pageTotal + groups().reduce((a, g) => a + g.blocks.length, 0);
    const pages = pageRows();
    if (pages) return displayed()?.matchedTotal ?? pages.length;
    if (friendlySearch() !== null || currentView() === "search") return searchPresentationHits().length;
    return groups().reduce((a, g) => a + g.blocks.length, 0);
  };
  const ranEmpty = () =>
    !!displayed() && !operation.loading && total() === 0 && blockingDiagnostics().length === 0
    && displayed()?.report?.supported !== false;
  const emptyMessage = () => (ranEmpty() ? "No results" : "Loading query results…");

  // Why empty? (Q14, N19): which top-level conjunct emptied the query, asked
  // only once the run actually came back empty.
  const [explainOpen, setExplainOpen] = createSignal(false);
  const explainRequest = createMemo(() => {
    const request = runRequest();
    return explainOpen() && request && request.search === null && ranEmpty() ? request : undefined;
  }, undefined, { equals: (a, b) => a?.key === b?.key });
  const explainOwners = {};
  const [explained] = createResource(explainRequest, async (request): Promise<ExplainEmptyResult | undefined> => {
    const landed = await readOwned(
      latestOwner(explainOwners, "explain", graphOwner()),
      backend().queryExplainEmpty(request.query.query, request.query.view, request.context),
    );
    return landed.kind === "current" ? landed.value : undefined;
  });
  const explanation = () => readOr(explained, undefined, "query explain");
  const explainNotice = (): string | null => {
    if (explained.error !== undefined) return errorText(explained.error);
    const answer = explanation();
    if (!answer) return null;
    const blocking = (answer.diagnostics ?? []).filter((d) => !d.disabled);
    if (blocking.length) return blocking.map((d) => d.message).join(" · ");
    if (!answer.report.supported) return "This query has no clauses Tine can run, so nothing was evaluated.";
    if (!answer.rows.length) return "Nothing in this graph matches this query.";
    return null;
  };

  // -- editing: every edit is printed by the engine and written back as bytes --
  const [printError, setPrintError] = createSignal<string | null>(null);
  const refuseChanged = () => {
    setPrintError(BLOCK_CHANGED);
    pushToastUnique(BLOCK_CHANGED, "error");
    return false;
  };
  // Rewrite just THIS macro inside the owning block, targeted by the extent's
  // own recovered name+argument (a block can hold more than one query).
  const targetMacro = (raw: string): MacroExtent | null => {
    const source = props.sourceExtent;
    if (!source) return null;
    const current = queryMacroExtents(raw).find((extent) => extent.start === source.start && extent.end === source.end);
    if (!current || raw.slice(current.start, current.end) !== props.sourceRaw
      || current.name !== source.name || current.argument !== source.argument) return null;
    return current;
  };
  const rewriteMacro = (newMacro: string, target: MacroExtent) => {
    if (!props.blockId) return;
    const raw = docNode(props.blockId)?.raw ?? "";
    setRaw(props.blockId, raw.slice(0, target.start) + newMacro + raw.slice(target.end));
  };
  /** OG text re-emits only `(sort-by …)` and `(sample …)`; TQL text keeps no
   *  view at all. A save must not drop the view facts its reprint loses, so
   *  they are written to the block's `tine.*` properties in the same undo unit
   *  (§4.3 Y2). Only facts the block does not already spell are written. */
  const materializeView = (blockId: string, view: ViewSettings, dialect: QueryPrintDialect) => {
    const spelled = new Set(hostProperties().map(([key]) => key.toLowerCase().replace(/^tine\.group-by$/, "tine.group-field")));
    const textKeeps = dialect === "og";
    const writes: [string, string | undefined][] = [
      ["tine.sort", !textKeeps && view.sort?.length ? view.sort.map(([f, d]) => `${f} ${d}`).join("; ") : undefined],
      ["tine.sample", !textKeeps && view.sample != null ? String(view.sample) : undefined],
      ["tine.group-field", view.group_by || undefined],
      ["tine.col-aggregates", view.aggregates?.length
        ? view.aggregates.map(([f, fn]) => (f ? `${f}=${fn}` : fn)).join(";")
        : undefined],
    ];
    for (const [key, value] of writes) if (value !== undefined && !spelled.has(key)) setBlockProperty(blockId, key, value);
  };
  // **The save path (§4.3).** `query_og_expressible` first; an edit OG cannot
  // express crosses to `{{tine-query}}` (and says so, §7.5). The bytes written
  // are the engine's; a refused print writes nothing and says why (I-4).
  const applyEdit = async (next: BuilderSession, displayEdit = false): Promise<boolean> => {
    const blockId = props.blockId;
    if (!blockId || !docNode(blockId)) return false;
    const rawAtStart = docNode(blockId).raw;
    const previousDisplay = displayEdit ? new Map(displayPropertyPatch(reading()?.view ?? {})) : null;
    const previousGroup = displayEdit ? reading()?.view.group_by : undefined;
    const owner = bindingOwner(() => docNode(blockId)?.raw === rawAtStart);
    const current = macroName();
    let name = current;
    let dialect = macroPrintDialect(name);
    let printed: Owned<string>;
    try {
      const expressible = await readOwned(owner, backend().queryOgExpressible(next.query, next.view));
      if (expressible.kind === "stale") {
        return refuseChanged();
      }
      name = expressible.value ? current : QUERY_MACRO_NAMES[1];
      dialect = macroPrintDialect(name);
      try {
        printed = await readOwned(owner, backend().printQuery(next.query, next.view, dialect));
      } catch (error) {
        // `og_expressible` said yes and the printer said no: the entitled answer
        // is the other dialect, not a refusal shown to the user.
        if (!(error instanceof QueryPrintRefusedError && error.isNotApplicable && dialect === "og")) throw error;
        name = QUERY_MACRO_NAMES[1];
        dialect = macroPrintDialect(name);
        printed = await readOwned(owner, backend().printQuery(next.query, next.view, dialect));
      }
    } catch (error) {
      setPrintError(errorText(error));
      return false;
    }
    const node = docNode(blockId);
    if (printed.kind === "stale" || !node) {
      return refuseChanged();
    }
    // A page that turned read-only while the print was in flight refuses the
    // write; report "not saved" so the sheet arms no focus (master 93ff682a3).
    if (!blockWritable(blockId)) {
      setPrintError("This block is read-only. The query was not changed.");
      return false;
    }
    const target = targetMacro(node.raw);
    if (!target) {
      return refuseChanged();
    }
    setPrintError(null);
    const argument = printed.value;
    const crossing = name.toLowerCase() !== current.toLowerCase();
    // ONE undo unit for the whole save; the tag is what the notice's Undo
    // recognises, so only a CROSSING save carries the crossing tag.
    const saved = withUndoUnit(crossing ? `query:cross:${blockId}` : `query:save:${blockId}`, [node.page], () => {
      rewriteMacro(`{{${name} ${argument}}}`, target);
      materializeView(blockId, next.view, dialect);
      if (previousDisplay) for (const [key, value] of displayPropertyPatch(next.view)) {
        if (previousDisplay.get(key) !== value || key === "tine.group-field"
          && previousGroup === undefined && next.view.group_by === "") {
          if (key === "tine.col-aggregates") {
            const merged = mergeQueryAggregateValue(blockProperty(blockId, key), next.view.aggregates ?? []);
            if (merged !== undefined) setBlockProperty(blockId, key, merged);
          } else setBlockProperty(blockId, key, value);
          if (key === "tine.group-field" && blockProperty(blockId, "tine.group-by")?.trim())
            setBlockProperty(blockId, "tine.group-by", null);
          if (key === "tine.columns" && isLegacyBareColumnList(blockProperty(blockId, "tine.fields")))
            setBlockProperty(blockId, "tine.fields", null);
        }
      }
      return true;
    });
    if (saved !== true) {
      setPrintError("A graph rewrite is in progress. The query was not changed.");
      return false;
    }
    if (crossing) setCrossed(blockId, boundedFeature(argument));
    return true;
  };
  /** Display uses the same guarded print and undo unit as a filter edit. */
  const applyDisplay = (view: ViewSettings): Promise<boolean> => {
    const current = reading();
    if (current && view.view !== current.view.view
      && JSON.stringify({ ...view, view: undefined }) === JSON.stringify({ ...current.view, view: undefined })) {
      setQueryView(view.view ?? "list");
      return Promise.resolve(true);
    }
    return current ? applyEdit({ query: current.query, view }, true) : Promise.resolve(false);
  };

  // **A title edit is not a filter conversion (§4.3.1).** The new options map
  // goes back through the printer with `preserveForm`, which re-emits the
  // authored source verbatim, so renaming a partly understood query cannot
  // rewrite its filter.
  const [editingTitle, setEditingTitle] = createSignal(false);
  const titleText = () => props.title ?? titleOption() ?? "Query";
  const titleEditable = () => !!props.blockId && props.title === undefined && !!reading();
  const setTitle = async (t: string) => {
    const blockId = props.blockId;
    const current = reading();
    if (!blockId || !current) return;
    const rawAtStart = docNode(blockId)?.raw;
    try {
      const title = t.trim().replace(/[\r\n{}]/g, "");
      const nextOptions = editEdnTitle(opts(), title);
      if (nextOptions === opts()) return;
      const nextQuery: Query = { ...current.query, source: { ...current.query.source, og_options: nextOptions } as Source };
      const printed = await readOwned(
        bindingOwner(() => docNode(blockId)?.raw === rawAtStart),
        backend().printQuery(nextQuery, current.view, sourcePrintDialect(current.query.source), true),
      );
      if (printed.kind === "stale") {
        refuseChanged();
        return;
      }
      const node = docNode(blockId);
      const target = node && targetMacro(node.raw);
      if (!node || !target || !blockWritable(blockId)) {
        refuseChanged();
        return;
      }
      const saved = withUndoUnit(`query:title:${blockId}`, [node.page], () => {
        rewriteMacro(`{{${macroName()} ${printed.value}}}`, target);
        return true;
      });
      setPrintError(saved === true ? null : "A graph rewrite is in progress. The query was not changed.");
    } catch (error) {
      // I-4: a refused print is never swallowed; nothing is written.
      setPrintError(errorText(error));
    }
  };

  // -- the §7.5 crossing notice ----------------------------------------------
  const [crossedTag, setCrossedTag] = createSignal<string | null>(null);
  const [crossedText, setCrossedText] = createSignal<string | null>(null);
  // The notice moves between two hosts; its UI state lives here so a
  // re-parented notice keeps its checkbox and does not grab focus again (N3).
  const [noticeDontShow, setNoticeDontShow] = createSignal(false);
  const [noticeFocused, setNoticeFocused] = createSignal(false);
  const [sheetOpen, setSheetOpen] = createSignal(false);
  const setCrossed = (blockId: string, changed: string | null) => {
    primeCrossingNoticePreference();
    setCrossedText(changed);
    setNoticeDontShow(false);
    setNoticeFocused(false);
    setCrossedTag(`query:cross:${blockId}`);
  };
  const dismissCrossing = () => {
    if (noticeDontShow()) dismissCrossingNoticeForever();
    setCrossedTag(null);
  };
  // Shown only once this device's answer is KNOWN to be "not dismissed".
  const showCrossingNotice = () => !!crossedTag() && crossingNoticePrimed === graphEpoch() && crossingNoticeDismissed() === false;
  // Undo is offered only while the entry `undo()` would take back IS the crossing save.
  const crossingIsStillUndoable = () => {
    const tag = crossedTag();
    return !!tag && !graphRewriteFrozen() && undoTopTag() === tag;
  };
  const crossingNotice = () => (
    <CrossingNotice
      canUndo={crossingIsStillUndoable()}
      changed={crossedText() ?? undefined}
      dontShow={noticeDontShow()}
      onDontShowChange={setNoticeDontShow}
      autoFocus={!noticeFocused()}
      onFocused={() => setNoticeFocused(true)}
      onUndo={() => {
        if (undo()) {
          dismissCrossing();
          return true;
        }
        pushToastUnique("Undo is unavailable while the graph is being rewritten.", "error");
        return false;
      }}
      onKeep={dismissCrossing}
      onDontShowAgain={dismissCrossingNoticeForever}
    />
  );
  /** What the builder edits: the AUTHORING reading, never the substituted one. */
  const builderSession = (): BuilderSession | undefined => {
    const current = reading();
    return current ? { query: current.query, view: current.view } : undefined;
  };
  const showBuilder = () => !!props.blockId && !isAdvanced() && !!builderSession();
  const [paneStale, setPaneStale] = createSignal(false);

  const globalSort = createMemo(() => (runnable()?.view.sort ?? []).length > 0);
  const queryGroupKey = (group: RefGroup, flat: boolean) =>
    flat
      ? `${group.kind}\0${group.page}\0${group.path ?? ""}\0${group.blocks.map((block) => block.id).join("\0")}`
      : `${group.kind}\0${group.page}\0${group.path ?? ""}`;
  const groupedQueryByKey = createMemo(() => new Map(groups().map((group) => [queryGroupKey(group, false), group] as const)));
  const flatQueryByKey = createMemo(() => new Map(groups().map((group) => [queryGroupKey(group, true), group] as const)));
  // The legacy table's columns and initial sort come from the HOST block's own
  // properties, like OG's query_table.cljs (audit #9); a header click then
  // overrides them for this view, as OG's click does (it also persists the
  // choice on the block; Tine keeps the click local and never rewrites the file).
  const hostSortState = createMemo(() =>
    props.blockId
      ? hostSort(blockProperty(props.blockId, "query-sort-by"), blockProperty(props.blockId, "query-sort-desc"))
      : null);
  const [sortOverride, setSortOverride] = createSignal<TableSort | null>(null);
  const sortState = (): TableSort | null => sortOverride() ?? hostSortState();
  const rows = createMemo<Row[]>(() =>
    !legacyTable() || collapsed() ? [] : groups().flatMap((g) =>
      g.blocks.map((b) => {
        const props: Record<string, string> = {};
        const byKey: Record<string, string> = {};
        for (const [k, val] of b.properties ?? []) {
          props[k] = val;
          byKey[columnKey(k)] ??= val;
        }
        return { page: g.page, kind: g.kind, path: g.path, text: visibleBody(b.raw).join(" "), props, byKey };
      })
    )
  );
  const hostCols = createMemo(() =>
    props.blockId ? hostColumns(blockProperty(props.blockId, "query-properties")) : null);
  /** Column ids in display order: `block`, `page`, then property keys. */
  const cols = createMemo(() => {
    const named = hostCols();
    if (named) return named;
    const keys = new Set<string>();
    for (const r of rows()) for (const k of Object.keys(r.props)) keys.add(k);
    return ["block", "page", ...keys];
  });
  const cell = cellText;
  const sorted = createMemo(() => {
    const s = sortState();
    if (!s) return rows();
    const key = (r: Row) => cell(r, s.column);
    return [...rows()].sort((a, b) => (s.desc ? compareCells(key(b), key(a)) : compareCells(key(a), key(b))));
  });
  const sortBy = (c: string) => {
    const cur = sortState();
    setSortOverride({ column: columnKey(c), desc: !(cur?.desc ?? true) });
  };
  // Clicks on query controls must not bubble to the block's onClick.
  const stop = (e: MouseEvent) => e.stopPropagation();
  const arrow = (c: string) => (sortState()?.column === columnKey(c) ? (sortState()!.desc ? " ▼" : " ▲") : "");

  const hidden = () =>
    props.hideWhenEmpty && !isAdvanced() && !!displayed() && total() === 0 && blockingDiagnostics().length === 0;
  const unsupportedAdvanced = () => {
    const info = advInfo();
    return isAdvanced() && info && (!info.supported || (props.strictAdvanced === true && (info.ignored ?? []).length > 0));
  };
  const simpleView = (): QueryView => (legacyTable() ? "table" : currentView());

  const whyEmpty = () => (
    <Show when={ranEmpty() && friendlySearch() === null}>
      <button
        type="button"
        class="query-why-empty"
        onClick={(e) => { e.stopPropagation(); setExplainOpen(!explainOpen()); }}
      >
        {explainOpen() ? "hide" : "why empty?"}
      </button>
      <Show when={explainOpen()}>
        <div class="query-why-empty-panel" onClick={stop}>
          <Show when={explained.loading}>
            <span class="query-why-empty-pending">Checking…</span>
          </Show>
          <Show when={explainNotice()}>
            {(notice) => <div class="query-why-empty-notice">{notice()}</div>}
          </Show>
          <Show when={(explanation()?.rows.length ?? 0) > 0}>
            <TableWrap><table class="md-table query-why-empty-table">
              <thead>
                <tr><th>Condition</th><th>Alone</th><th>Without it</th></tr>
              </thead>
              <tbody>
                <For each={explanation()!.rows}>
                  {(row) => (
                    <tr classList={{ "query-why-empty-culprit": row.alone === 0 }}>
                      <td><code>{row.conjunct}</code></td>
                      <td>{row.alone}</td>
                      <td>{row.without ?? "—"}</td>
                    </tr>
                  )}
                </For>
              </tbody>
            </table></TableWrap>
          </Show>
        </div>
      </Show>
    </Show>
  );
  const empty = () => (
    <Show when={!loadError() && blockingDiagnostics().length === 0 && (!displayed() || ranEmpty())}>
      <div class="query-empty">
        {emptyMessage()} {whyEmpty()}
      </div>
    </Show>
  );

  return (
    <Show when={!hidden()}>
      <div class="query-block" classList={{ "query-sheet-block": sheetFace(), "query-stale": paneStale() }}>
        <Switch>
          <Match when={unsupportedAdvanced()}>
            <div class="query-unsupported" role={props.unsupportedLabel ? "alert" : undefined}>
              <Show
                when={props.unsupportedLabel}
                fallback={
                  <>
                    Advanced (datalog) query not run: Tine cannot run{" "}
                    {(advInfo()?.ignored ?? []).length > 0
                      ? `these clauses (${(advInfo()?.ignored ?? []).join(", ")})`
                      : "some of its clauses"}
                    , and a partial answer would be wrong. <code>{`{{${props.body}}}`}</code>
                  </>
                }
              >
                {(label) => <>{label()}: query contains unsupported clauses.</>}
              </Show>
              <Show when={blockingDiagnostics().length > 0}>
                <span class="query-diagnostics">{" "}{blockingDiagnostics().map((d) => d.message).join(" · ")}</span>
              </Show>
            </div>
          </Match>
          <Match when={true}>
            <Show when={isAdvanced() && advInfo()?.supported}>
              <div class="query-adv-note">
                Advanced query — ran: {(advInfo()!.ran ?? []).join(", ") || "—"}
              </div>
            </Show>
            <div class="query-header">
              <span
                class="query-collapse"
                classList={{ collapsed: collapsed() }}
                title={collapsed() ? "Expand results" : "Collapse results"}
                onClick={(e) => {
                  e.stopPropagation();
                  toggleCollapsed();
                }}
              >
                <svg viewBox="0 0 24 24" class="triangle">
                  <path d="M8 5l8 7-8 7z" />
                </svg>
              </span>
              <Show
                when={editingTitle()}
                fallback={
                  <span
                    class="query-title"
                    classList={{ "query-title-editable": titleEditable() }}
                    title={titleEditable() ? "Click to rename this query" : undefined}
                    onClick={(e) => {
                      if (titleEditable()) {
                        e.stopPropagation();
                        setEditingTitle(true);
                      }
                    }}
                  >
                    {titleText()}
                  </span>
                }
              >
                {(() => {
                  let canceled = false;
                  return (
                    <input
                      class="query-title-input"
                      autofocus
                      value={titleOption() ?? ""}
                      placeholder="Query title"
                      onClick={(e) => e.stopPropagation()}
                      onKeyDown={(e) => {
                        e.stopPropagation();
                        if (e.key === "Enter") {
                          canceled = true;
                          void setTitle(e.currentTarget.value);
                          setEditingTitle(false);
                        } else if (e.key === "Escape") {
                          canceled = true;
                          setEditingTitle(false);
                        }
                      }}
                      onBlur={(e) => {
                        if (!canceled) void setTitle(e.currentTarget.value);
                        setEditingTitle(false);
                      }}
                    />
                  );
                })()}
              </Show>{" "}
              {/* The builder sentence carries the count beside it; queries with
                  no builder keep it here so the count never disappears. */}
              <Show when={!showBuilder()}>
                <span class="query-count">{total()}</span>
              </Show>
              <Show when={!isPublishedExport() && runnable() && displayed() && total() > 0}>
                <button type="button" class="query-export-action" title="Export query results"
                  onClick={(event) => {
                    event.stopPropagation();
                    openQueryExport({
                      argument: executionArg() ?? arg(),
                      dialect: macroTextDialect(macroName()),
                      properties: hostProperties(),
                      currentPage: executionContext()?.current_page,
                      hostBlockId: props.blockId,
                      name: titleText(),
                    });
                  }}>Export…</button>
              </Show>
              <Show when={props.blockId && !isPublishedExport()}>
                <div class="query-view-switcher" role="group" aria-label="Query view" onClick={stop}>
                  <For each={VIEW_KINDS}>
                    {(view) => (
                      <button
                        type="button"
                        classList={{ active: currentView() === view }}
                        onClick={(e) => {
                          e.stopPropagation();
                          setQueryView(view);
                        }}
                      >
                        {QUERY_VIEW_LABEL[view]}
                      </button>
                    )}
                  </For>
                </div>
              </Show>
            </div>
            {/* The builder edits a FILTER; an authored advanced query keeps its
                own editing path (raw text) — converting one is out of scope. */}
            <Show when={showBuilder()}>
              <QueryBuilder
                session={builderSession}
                onChange={applyEdit}
                display={{ view: () => reading()?.view ?? {}, apply: applyDisplay,
                  formulas: () => [...formulasOf(hostProperties()).keys()] }}
                paneDialect="tql"
                blockId={props.blockId}
                previewContext={executionContext}
                both={props.blockId ? { on: bothKinds, set: setBothKinds } : undefined}
                total={<span class="query-count">{total()}</span>}
                onStale={setPaneStale}
                onOpenChange={setSheetOpen}
                notice={showCrossingNotice() && sheetOpen() ? crossingNotice : undefined}
              />
            </Show>
            {/* §7.5, N3: one notice, inline while the sheet is shut and inside
                the text pane while it is open — never both. */}
            <Show when={showCrossingNotice() && !sheetOpen()}>{crossingNotice()}</Show>
            <Show when={printError()}>
              {(message) => (
                <div class="query-unsupported query-print-refused" role="alert">
                  The query wasn't changed: {message()}
                </div>
              )}
            </Show>
            <Show when={loadError()}>
              {(error) => (
                <div class="query-unsupported" role="alert">
                  Query couldn't be loaded: {errorText(error())}
                </div>
              )}
            </Show>
            <Show when={blockingDiagnostics().length > 0}>
              <div class="query-unsupported query-diagnostics" role="alert">
                <span class="query-diagnostics-lead">
                  Tine didn't understand part of this query, so it returned no results:
                </span>{" "}
                {blockingDiagnostics().map((d) => d.message).join(" · ")}
              </div>
            </Show>
            <Show when={!collapsed()}>
              <Show when={displayed()?.statistics}>
                {(statistics) => <QueryStatisticsSummary statistics={statistics()} onClearGrouping={props.blockId && blockWritable(props.blockId) && !isPublishedExport()
                  ? () => setBoardGroupBy(props.blockId!, "") : undefined} />}
              </Show>
              <Switch>
                <Match when={displayed()?.both}>
                  {(both) => (
                    <QueryResultSections
                      pending={false}
                      failure={null}
                      families={[
                        {
                          kind: "page",
                          hits: both().pages.length,
                          hasMore: both().pageMore,
                          note: both().pageNote ?? undefined,
                          body: (
                            <QueryPageRows
                              rows={both().pages}
                              view={simpleView()}
                              groupBy={runnable()?.view.group_by ?? blockProperty(props.blockId ?? "", "tine.group-by") ?? undefined}
                              columns={runnable()?.view.columns}
                            />
                          ),
                        },
                        {
                          kind: "block",
                          hits: groups().reduce((a, g) => a + g.blocks.length, 0),
                          hasMore: both().blockMore,
                          note: both().blockNote ?? undefined,
                          // The block presentation (list / table / board) is the one the blocks
                          // family has on its own; the sheet footer's statistics belong to the
                          // macro's OWN anchor, so a page-anchored macro's blocks get none.
                          body: (
                            <Switch fallback={
                              <Show when={globalSort()} fallback={<QueryGroups groups={groupedQueryByKey} paused={switcherOpen()} />}>
                                <QueryGroups groups={flatQueryByKey} flat paused={switcherOpen()} />
                              </Show>
                            }>
                              <Match when={sheetFace() && !!props.blockId && blockFace() === "table"}>
                                <SheetContainer>
                                  <SheetTable ownerId={props.blockId!} rowSource="query" groups={groups()}
                                    queryDisplay={{ view: reading()?.view ?? {},
                                      ...(both().ownAnchor === "block" ? { statistics: displayed()?.statistics, statisticsView: runnable()?.view } : {}),
                                      apply: (next) => void applyDisplay(next) }} />
                                </SheetContainer>
                              </Match>
                              <Match when={sheetFace() && !!props.blockId && blockFace() === "board"}>
                                <SheetContainer>
                                  <SheetBoard ownerId={props.blockId!} rowSource="query" groupBy={runnable()?.view.group_by ?? sheet()?.groupBy} groups={groups()} />
                                </SheetContainer>
                              </Match>
                            </Switch>
                          ),
                        },
                      ]}
                    />
                  )}
                </Match>
                <Match when={pageRows()}>
                  {(pages) => (
                    <Show when={pages().length > 0} fallback={empty()}>
                      <QueryPageRows
                        rows={pages()}
                        view={simpleView()}
                        groupBy={runnable()?.view.group_by ?? blockProperty(props.blockId ?? "", "tine.group-by") ?? undefined}
                        columns={runnable()?.view.columns}
                      />
                    </Show>
                  )}
                </Match>
                <Match when={sheetFace()}>
                  <Show when={groups().length > 0} fallback={empty()}>
                    <Show when={!!props.blockId}>
                      <SheetContainer>
                        <Switch>
                          <Match when={blockFace() === "table"}>
                            <SheetTable ownerId={props.blockId!} rowSource="query" groups={groups()}
                              queryDisplay={{ view: reading()?.view ?? {}, statistics: displayed()?.statistics, statisticsView: runnable()?.view, apply: (next) => void applyDisplay(next) }} />
                          </Match>
                          <Match when={blockFace() === "board"}>
                            <SheetBoard ownerId={props.blockId!} rowSource="query" groupBy={runnable()?.view.group_by ?? sheet()?.groupBy} groups={groups()} />
                          </Match>
                        </Switch>
                      </SheetContainer>
                    </Show>
                  </Show>
                </Match>
                <Match when={currentView() === "search"}>
                  <div class="query-search-results" role="list" aria-label="Search results" onClick={stop}>
                    <Show when={searchPresentationHits().length > 0} fallback={empty()}>
                      <For each={searchPresentationHits()}>
                        {(hit) => (
                          <Show
                            when={hit.entity === "block" ? hit : null}
                            fallback={hit.entity === "page" ? (
                              <button
                                type="button"
                                class="query-search-page"
                                onMouseDown={internalLinkMouseDown}
                                onClick={(e) => {
                                  const target = {
                                    name: hit.page.name,
                                    pageKind: hit.page.kind,
                                    ...(hit.page.path ? { path: hit.page.path } : {}),
                                  };
                                  const dest = internalLinkDest(e);
                                  if (dest === "sidebar") openPageInSidebar(target);
                                  else if (dest === "background") openPageTargetInNewTab(target);
                                  else if (dest === "pane") openRouteInOtherPane({ kind: "page", ...target });
                                  else openPageTarget(target);
                                }}
                                onAuxClick={(e) => internalLinkAuxClick(e, () => openPageTargetInNewTab({
                                  name: hit.page.name,
                                  pageKind: hit.page.kind,
                                  ...(hit.page.path ? { path: hit.page.path } : {}),
                                }))}
                              >
                                <span class="switcher-kind">{hit.page.kind}</span>
                                <span>{hit.display_text}</span>
                              </button>
                            ) : null}
                          >
                            {(blockHit) => (
                              <button
                                type="button"
                                class="query-search-hit switcher-row block-result"
                                onMouseDown={internalLinkMouseDown}
                                onClick={(e) => {
                                  const bh = blockHit();
                                  const uuid = blockDtoExternalId(bh.block);
                                  const route = { kind: "page" as const, name: bh.page, pageKind: bh.kind, block: uuid, ...(bh.path ? { path: bh.path } : {}) };
                                  const dest = internalLinkDest(e);
                                  if (dest === "sidebar") openBlockInSidebar({ uuid, page: bh.page, pageKind: bh.kind, ...(bh.path ? { path: bh.path } : {}) });
                                  else if (dest === "background") openInNewTab(route);
                                  else if (dest === "pane") openRouteInOtherPane(route);
                                  else openPageAtBlock({ name: bh.page, pageKind: bh.kind, block: uuid, ...(bh.path ? { path: bh.path } : {}) });
                                }}
                                onAuxClick={(e) => internalLinkAuxClick(e, () => {
                                  const bh = blockHit();
                                  openInNewTab({ kind: "page", name: bh.page, pageKind: bh.kind, block: blockDtoExternalId(bh.block), ...(bh.path ? { path: bh.path } : {}) });
                                })}
                              >
                                <SearchResultRow
                                  page={blockHit().page}
                                  breadcrumb={blockHit().block.breadcrumb ?? []}
                                  text={blockHit().display_text}
                                  spans={blockHit().evidence.flatMap((evidence) => evidence.spans)}
                                />
                              </button>
                            )}
                          </Show>
                        )}
                      </For>
                    </Show>
                  </div>
                </Match>
                <Match when={true}>
                  <Show when={groups().length > 0} fallback={empty()}>
                    <Show
                      when={legacyTable()}
                      fallback={
                        <Show
                          when={globalSort()}
                          fallback={
                            <QueryGroups groups={groupedQueryByKey} paused={switcherOpen()} />
                          }
                        >
                          {/* Sorted: the engine's flat global order, one group per run of rows. */}
                          <QueryGroups groups={flatQueryByKey} flat paused={switcherOpen()} />
                        </Show>
                      }
                    >
                      <QueryLegacyTable cols={cols()} rows={sorted()} sortBy={sortBy} arrow={arrow} />
                    </Show>
                  </Show>
                </Match>
              </Switch>
            </Show>
          </Match>
        </Switch>
      </div>
    </Show>
  );
}

interface YoutubePlayer {
  seekTo(seconds: number, allowSeekAhead: boolean): void;
  getCurrentTime(): number;
  destroy?(): void;
}

interface YoutubeApi {
  Player: new (iframeId: string, options: { events?: { onReady?: () => void } }) => YoutubePlayer;
}

type YoutubeWindow = Window & {
  YT?: YoutubeApi;
  onYouTubeIframeAPIReady?: () => void;
};

const youtubePlayers = new Map<string, YoutubePlayer>();
let youtubeApiLoading: Promise<YoutubeApi | null> | null = null;
const YOUTUBE_API_SCRIPT_ID = "tine-youtube-iframe-api";

// The API is intentionally fetched only from a mounted YouTube embed. OG does
// the same mount-time load/register sequence (og-1.0.0 6e7afa8eb,
// extensions/video/youtube.cljs:20-27, :45-53).
function loadYoutubeApi(): Promise<YoutubeApi | null> {
  if (typeof window === "undefined" || typeof document === "undefined") return Promise.resolve(null);
  const ytWindow = window as YoutubeWindow;
  if (ytWindow.YT?.Player) return Promise.resolve(ytWindow.YT);
  if (youtubeApiLoading) return youtubeApiLoading;

  youtubeApiLoading = new Promise((resolve) => {
    let settled = false;
    const settle = (api: YoutubeApi | undefined) => {
      if (settled) return;
      settled = true;
      resolve(api?.Player ? api : null);
    };
    const priorReady = ytWindow.onYouTubeIframeAPIReady;
    ytWindow.onYouTubeIframeAPIReady = () => {
      try {
        priorReady?.();
      } finally {
        settle(ytWindow.YT);
      }
    };
    // The IFrame API is one process-wide script in the main realm's <head>.
    let script = mainWindow.document.getElementById(YOUTUBE_API_SCRIPT_ID) as HTMLScriptElement | null;
    if (!script) {
      script = document.createElement("script");
      script.id = YOUTUBE_API_SCRIPT_ID;
      script.async = true;
      script.src = "https://www.youtube.com/iframe_api";
      document.head.appendChild(script);
    }
    script.addEventListener("error", () => settle(undefined), { once: true });
  });
  return youtubeApiLoading;
}

// OG's get-player selects the last YouTube iframe whose DOM position precedes
// the target (compareDocumentPosition(..., target) has FOLLOWING set), then
// looks up that iframe's registered handle (og-1.0.0 6e7afa8eb,
// extensions/video/youtube.cljs:85-101).
export function youtubePlayerForTarget(target: Node): YoutubePlayer | undefined {
  if (typeof document === "undefined" || typeof Node === "undefined") return undefined;
  // Players before the target in the TARGET's own window (OG-MULTIWINDOW P1).
  const iframe = Array.from(documentOf(target).getElementsByTagName("iframe"))
    .filter((node) => node.src.includes("youtube.com"))
    .filter((node) => (node.compareDocumentPosition(target) & Node.DOCUMENT_POSITION_FOLLOWING) !== 0)
    .at(-1);
  return iframe ? youtubePlayers.get(iframe.id) : undefined;
}

// The OG generator floors getCurrentTime before it formats the macro; with no
// registered/ready player it produces NOTHING — the command is a no-op, no
// macro is inserted (og-1.0.0 6e7afa8eb, extensions/video/youtube.cljs:113-122).
export function youtubeTimestampMacroFor(target: Node): string | null {
  const seconds = youtubePlayerForTarget(target)?.getCurrentTime();
  if (typeof seconds !== "number" || !Number.isFinite(seconds)) return null;
  return `{{youtube-timestamp ${Math.max(0, Math.floor(seconds))}}}`;
}

function httpUrl(value: string): string | undefined {
  if (!/^https?:\/\//i.test(value)) return undefined;
  try {
    const url = new URL(value);
    return (url.protocol === "http:" || url.protocol === "https:") && url.hostname ? value : undefined;
  } catch {
    return undefined;
  }
}

// A {{video}} / {{youtube}} / {{vimeo}} / {{bilibili}} macro: embeds YouTube,
// Vimeo or Bilibili as an iframe and direct media files as a <video>. Each of the
// provider-named macros also accepts a bare id (e.g. `{{vimeo 12345}}`), matching
// OG; the generic `{{video URL}}` sniffs the provider from the URL. Falls back to a
// link. (`youtube-timestamp` is a SEPARATE macro — handled before this one.)
export function VideoMacro(props: { body: string }): JSX.Element {
  const iframeId = `youtube-player-${createUniqueId()}`;
  const parsed = () => {
    const m = /^(\w+)\s*([\s\S]*)$/.exec(props.body.trim());
    const name = (m?.[1] ?? "video").toLowerCase();
    const arg = (m?.[2] ?? "").trim().replace(/^\[\[|\]\]$/g, "");
    return { name, arg };
  };
  const url = () => parsed().arg;
  const safeUrl = () => httpUrl(url());
  const embed = () => {
    const { name, arg } = parsed();
    // `?enablejsapi=1` matches OG (youtube.cljs:58) and enables timestamps.
    const yt = safeUrl() && /(?:youtube\.com\/(?:watch\?v=|embed\/)|youtu\.be\/)([\w-]{11})/.exec(arg);
    if (yt) return `https://www.youtube.com/embed/${yt[1]}?enablejsapi=1`;
    if (name === "youtube" && /^[\w-]{11}$/.test(arg)) return `https://www.youtube.com/embed/${arg}?enablejsapi=1`;
    const vimeo = safeUrl() && /vimeo\.com\/(\d+)/.exec(arg);
    if (vimeo) return `https://player.vimeo.com/video/${vimeo[1]}`;
    if (name === "vimeo" && /^\d+$/.test(arg)) return `https://player.vimeo.com/video/${arg}`;
    const bili = safeUrl() && /bilibili\.com\/video\/(BV[0-9A-Za-z]+)/i.exec(arg);
    const bvid = bili ? bili[1] : name === "bilibili" && /^BV[0-9A-Za-z]+$/.test(arg) ? arg : null;
    if (bvid) return `https://player.bilibili.com/player.html?bvid=${bvid}&high_quality=1`;
    return null;
  };
  // OG parity (og-1.0.0 6e7afa8eb): the embed iframe's `allow`/`referrerpolicy`.
  // YouTube (youtube.cljs:54-70) preserves an HTTP(S) parent's origin referrer.
  // A custom-protocol desktop parent still needs native WebView identification
  // (GH #600); this policy cannot manufacture a missing HTTP Referer header.
  // https://developers.google.com/youtube/terms/required-minimum-functionality
  // Vimeo (block.cljs:1290-1305) gets the same `allow` list
  // minus picture-in-picture/web-share and NO referrerpolicy; bilibili sets
  // neither (the `.embed-iframe` class already removes the border).
  const embedAttrs = (): Record<string, string> => {
    const src = embed() ?? "";
    if (/(?:^|\/\/)(?:www\.)?(?:youtube\.com|youtube-nocookie\.com)\/embed\//.test(src))
      return {
        allow: "accelerometer; autoplay; clipboard-write; encrypted-media; gyroscope; picture-in-picture; web-share",
        referrerpolicy: "strict-origin-when-cross-origin",
      };
    if (src.includes("player.vimeo.com"))
      return { allow: "accelerometer; autoplay; clipboard-write; encrypted-media; gyroscope" };
    return {};
  };
  const isYoutubeEmbed = () => /(?:^|\/\/)(?:www\.)?(?:youtube\.com|youtube-nocookie\.com)\/embed\//.test(embed() ?? "");
  let player: YoutubePlayer | undefined;
  onMount(() => {
    if (!isYoutubeEmbed()) return;
    let mounted = true;
    void loadYoutubeApi().then((api) => {
      if (!mounted || !api) return;
      try {
        const registered = new api.Player(iframeId, { events: { onReady: () => undefined } });
        if (!mounted) {
          registered.destroy?.();
          return;
        }
        player = registered;
        youtubePlayers.set(iframeId, registered);
      } catch {
        // Offline, blocked, or malformed API responses leave the timestamp label usable.
      }
    });
    onCleanup(() => {
      mounted = false;
      if (youtubePlayers.get(iframeId) === player) youtubePlayers.delete(iframeId);
      player?.destroy?.();
    });
  });
  return (
    <Show
      when={embed()}
      fallback={
        <Show
          when={safeUrl() && /\.(mp4|webm|ogg)(\?|$)/i.test(url())}
          fallback={safeUrl()
            ? <ExternalLink dest={safeUrl()!}>{url()}</ExternalLink>
            : <span>{url()}</span>}
        >
          <video class="embed-video" src={safeUrl()} controls />
        </Show>
      }
    >
      <div class="embed-iframe-wrap">
        <iframe id={isYoutubeEmbed() ? iframeId : undefined} class="embed-iframe" src={embed()!} allowfullscreen title="video" {...embedAttrs()} />
      </div>
    </Show>
  );
}

// A {{tweet URL}} / {{twitter URL}} macro (`twitter` is OG's alias for `tweet`) —
// rendered as a link (no third-party script embedding).
export function TweetMacro(props: { body: string }): JSX.Element {
  const url = () => props.body.replace(/^(tweet|twitter)\s*/i, "").trim();
  const safeUrl = () => httpUrl(url());
  return (
    <Show when={safeUrl()} fallback={<span>🐦 {url()}</span>}>
      <ExternalLink class="external-link tweet-link" dest={safeUrl()!}>🐦 {url()}</ExternalLink>
    </Show>
  );
}

// `{{youtube-timestamp <seconds>}}` seeks the OG-selected on-page YouTube player.
export function YoutubeTimestamp(props: { body: string }): JSX.Element {
  const secs = () => {
    const raw = props.body.replace(/^youtube-timestamp\s*/i, "").trim();
    const n = parseInt(raw, 10);
    return Number.isFinite(n) ? n : 0;
  };
  const label = () => {
    const s = Math.max(0, secs());
    const h = Math.floor(s / 3600);
    const m = Math.floor((s % 3600) / 60);
    const sec = s % 60;
    const pad = (x: number) => String(x).padStart(2, "0");
    return h > 0 ? `${h}:${pad(m)}:${pad(sec)}` : `${m}:${pad(sec)}`;
  };
  return (
    <a
      class="youtube-ts"
      title="Seek the preceding YouTube video"
      onClick={(event) => {
        event.preventDefault();
        event.stopPropagation();
        // OG timestamp click stops the event and calls seekTo(seconds, true)
        // on get-player's result (og-1.0.0 6e7afa8eb,
        // extensions/video/youtube.cljs:103-111).
        youtubePlayerForTarget(event.currentTarget)?.seekTo(secs(), true);
      }}
    >
      ⏱ {label()}
    </a>
  );
}

// `{{cloze answer}}` (optionally `{{cloze answer\\cue}}`) — in OG this is hidden
// only inside the SRS flashcard-review loop. Tine has no SRS engine, so we degrade
// to a click-to-reveal: shows the cue (or `[...]`) until clicked, then the answer.
export function ClozeMacro(props: { body: string }): JSX.Element {
  const [revealed, setRevealed] = createSignal(false);
  const parts = () => props.body.replace(/^cloze\s*/i, "").trim().split(/\\\\/);
  const answer = () => (parts()[0] ?? "").trim();
  const cue = () => parts()[1]?.trim();
  return (
    <span
      class="cloze"
      classList={{ revealed: revealed() }}
      title={revealed() ? "Click to hide" : "Click to reveal"}
      onClick={(e) => {
        e.stopPropagation();
        setRevealed((v) => !v);
      }}
    >
      {revealed() ? answer() : (cue() ?? "[...]")}
    </span>
  );
}

// `{{zotero-imported-file ...}}` / `{{zotero-linked-file ...}}` — OG resolves the
// Zotero item-key to a real attachment via its Zotero connector (data dir + item
// metadata + storage config). Tine has no Zotero integration, so resolving would
// yield a dead link; we degrade to a muted, non-navigating label rather than a
// broken link. Flagged as a known parity gap (niche).
export function ZoteroMacro(props: { body: string }): JSX.Element {
  const arg = () => props.body.replace(/^zotero-(imported|linked)-file\s*/i, "").trim();
  return (
    <span class="zotero-ref" title="Zotero integration isn't supported in Tine">
      📎 {arg() || "Zotero attachment"}
    </span>
  );
}

// A {{embed ((uuid))}} or {{embed [[Page]]}} block.
export function EmbedMacro(props: { body: string; blockId?: string }): JSX.Element {
  const linkDepth = useContext(LinkDepthContext);
  if (linkDepth > MAX_DEPTH_OF_LINKS) return <LinkDepthWarning />;

  const target = () => props.body.replace(/^embed\s*/i, "").trim();
  const pageTarget = () => /^\[\[([^\]]+)\]\]$/.exec(target())?.[1];
  const selfPageEmbed = () => {
    const sourcePage = props.blockId ? docNode(props.blockId)?.page : undefined;
    const targetPage = pageTarget();
    return !!sourcePage
      && pageByName(sourcePage)?.kind === "page"
      && !!targetPage
      && pageIdentityKey(sourcePage) === pageIdentityKey(targetPage);
  };

  const [data] = createResource(
    () => selfPageEmbed() ? null : `${target()} ${graphEpoch()} ${dataRev()}`,
    async () => {
    const t = target();
    const blockRef = /^\(\(([^)]+)\)\)$/.exec(t);
    if (blockRef) {
      const g = await resolveBlockBatched(blockRef[1]);
      // embedId = the embedded block's own id, so its ref-count badge is hidden
      // inside the embed (OG hide-block-refs-count?); its children keep theirs.
      return g ? { page: g.page, kind: g.kind, blocks: g.blocks, embedId: g.blocks[0]?.id } : null;
    }
    const pageRef = /^\[\[([^\]]+)\]\]$/.exec(t);
    if (pageRef) {
      // Backend miss → the virtual in-app Guide, matched by bare title (the embed
      // carries no source context to remap the name). No-op for real graphs.
      const result = await readOwned(graphOwner(), backend().getPage(pageRef[1], "page"));
      if (result.kind === "stale") return null;
      const p = result.value ?? resolveGuidePageDto(pageRef[1]);
      return p ? { page: p.name, kind: "page" as PageKind, blocks: p.blocks, embedId: undefined } : null;
    }
    return null;
  });

  const embedded = () => readOr(data, undefined, "embed");
  return (
    <div class="embed-block">
      <Show when={!selfPageEmbed()}>
        <Show when={embedded()} fallback={<div class="embed-missing">{`{{${props.body}}}`}</div>}>
          <LiveRefGroup page={embedded()!.page} kind={embedded()!.kind} blocks={embedded()!.blocks} embedId={embedded()!.embedId} hostBlockId={props.blockId} surface="embed" />
        </Show>
      </Show>
    </div>
  );
}
