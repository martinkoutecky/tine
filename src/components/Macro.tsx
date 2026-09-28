import { For, Show, Switch, Match, createMemo, createResource, createSignal, useContext, createUniqueId, onCleanup, onMount, type JSX } from "solid-js";
import { backend } from "../backend";
import { openPageTarget, openPageAtBlock, openPageTargetInNewTab } from "../router";
import { openPageInSidebar, openPageContextMenu, pageIdentityKey } from "../ui";
import { dataRev, graphEpoch, graphMeta } from "../graphSession";
import { advanceRevision, graphOwner, latestOwner, readOwned, revisionOwner, writeOwned, type Owned } from "../owned";
import { blockProperty, blockWritable, formatForPage, formatForBlock, pageByName, resolveGuidePageDto, setBlockProperty, setRaw, undo, undoTopTag, withUndoUnit, node as docNode } from "../document";
import { resolveBlockBatched } from "../resolveBatch";
import { shouldOpenTextContextMenu } from "../contextMenuPolicy";
import { LiveRefGroup } from "./LiveRefGroup";
import { QueryBuilder, type BuilderSession } from "./QueryBuilder";
import { CrossingNotice } from "./CrossingNotice";
import { SearchResultRow } from "./SearchResultRow";
import { quoteEdnString, unquoteEdnString } from "../editor/edn";
import { queryMacroExtents } from "../editor/queryMacro";
import { QUERY_MACRO_NAMES } from "../editor/queryMacroName";
import {
  macroPrintDialect,
  macroTextDialect,
  sourceOptions,
  sourceOriginal,
  sourcePrintDialect,
  type Diagnostic,
  type ExecutionContext,
  type ExplainEmptyResult,
  type PageRow,
  type ParsedQuery,
  type Query,
  type QueryPrintDialect,
  type QueryReport,
  type QueryStatistics,
  type Source,
  type ViewSettings,
} from "../editor/queryIr";
import { visibleBody } from "../render/block";
import { facetsOf } from "../render/facets";
import { sheetConfig } from "../sheet/config";
import { InlineText } from "../render/inline";
import { SheetTable } from "./SheetTable";
import { SheetBoard } from "./SheetBoard";
import { SheetContainer } from "./SheetContainer";
import { QueryPageRows, QueryStatisticsSummary, type QueryView } from "./QueryResultParts";
import type { PageKind, QueryExecution, QueryHit, RefGroup } from "../types";
import { sharedQueryResult } from "../queryResultCache";
import { declaresCurrentPageInput, queryCurrentPage } from "../queryCurrentPage";
import { savedDslToFriendlySearch } from "../editor/searchQuery";
import { LinkDepthContext, LinkDepthWarning, MAX_DEPTH_OF_LINKS } from "./linkDepth";
import { blockDtoExternalId } from "../blockIdentity";
import { QueryPrintRefusedError } from "../backend";
import { focusedRouter } from "../panes";
import { pushToast } from "../toasts";

const QUERY_VIEWS: QueryView[] = ["search", "list", "table", "board"];
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

interface Row {
  page: string;
  kind: PageKind;
  path?: string;
  text: string;
  props: Record<string, string>;
}

const errorText = (error: unknown): string => (error instanceof Error ? error.message : String(error));
const sameJson = <T,>(a: T, b: T) => JSON.stringify(a) === JSON.stringify(b);
const CURRENT_PAGE_RE = /<%\s*current page\s*%>/i;
const BLOCK_CHANGED = "The block changed while saving. Try this edit again.";

/** A bounded excerpt of the ENGINE-PRINTED text a crossing save wrote (master
 *  `boundedFeature`, I-22). It is labelled as an excerpt, never as "the
 *  unsupported feature": nothing in the engine answers that question. */
export function boundedFeature(message: string): string | null {
  const single = message.replace(/\s+/g, " ").trim();
  if (!single) return null;
  return single.length > 120 ? `${single.slice(0, 119)}…` : single;
}

/** Remove the block a query is written in from that query's own results
 *  (GH #469; OG `query/result.cljs` "exclude the current one, otherwise it'll
 *  loop forever"). Only the block goes; its children are ordinary results. */
export function withoutHostBlock(groups: RefGroup[], hostBlockId: string | undefined): RefGroup[] {
  if (!hostBlockId) return groups;
  const hosts = (group: RefGroup) => group.blocks.some((block) => block.id === hostBlockId);
  if (!groups.some(hosts)) return groups;
  return groups
    .map((group) => (hosts(group) ? { ...group, blocks: group.blocks.filter((block) => block.id !== hostBlockId) } : group))
    .filter((group) => group.blocks.length > 0);
}

// "Don't show this again" for the §7.5 crossing notice is a DEVICE preference
// (D-11): one read per process, never per block render (I-13).
const CROSSING_NOTICE_KEY = "queryCrossingNoticeDismissed";
const [crossingNoticeDismissed, setCrossingNoticeDismissed] = createSignal<boolean | undefined>(undefined);
const crossingNoticePreference = {};
let crossingNoticePrimed = false;
function primeCrossingNoticePreference(): void {
  if (crossingNoticePrimed) return;
  crossingNoticePrimed = true;
  const revision = advanceRevision(crossingNoticePreference);
  void readOwned(
    revisionOwner(crossingNoticePreference, revision),
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
    revisionOwner(crossingNoticePreference, revision),
    backend().setAppBool(CROSSING_NOTICE_KEY, true),
  ).catch((error: unknown) => pushToast(`Couldn't save the notice preference: ${errorText(error)}`, "error"));
}
export function resetCrossingNoticeForTests(): void {
  crossingNoticePrimed = false;
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

interface QueryOperation {
  groups: RefGroup[];
  pages: PageRow[] | null;
  diagnostics: Diagnostic[];
  report: QueryReport | null;
  statistics?: QueryStatistics;
  search: QueryExecution | null;
  matchedTotal: number | null;
}

// A {{query …}} / {{tine-query …}} block. The ONE engine (I-12) reads the
// macro (`query_parse`), runs it (`query_run`) and prints every edit back
// (`query_print`); this component only presents answers and writes the bytes
// the engine returned. With `blockId` the builder sentence and sheet edit it.
export function QueryMacro(props: {
  body: string;
  blockId?: string;
  title?: string;
  /** Read-only query surfaces can supply page context without a blockId, which
   * would incorrectly enable editing controls. */
  currentPage?: string;
  /** BEGIN_QUERY must never execute a partially understood query or expose its
   * authored payload in an error. */
  strictAdvanced?: boolean;
  unsupportedLabel?: string;
  // Render nothing when there are no results (the app-inserted journal agenda).
  hideWhenEmpty?: boolean;
}): JSX.Element {
  const linkDepth = useContext(LinkDepthContext);
  if (linkDepth > MAX_DEPTH_OF_LINKS) return <LinkDepthWarning />;

  // The macro name this query was AUTHORED under (§7.9): `query` or `tine-query`.
  const macroName = (): string =>
    QUERY_MACRO_NAMES.find((name) => new RegExp(`^${name}(\\s|$)`, "i").test(props.body.trim()))
    ?? QUERY_MACRO_NAMES[0];
  const arg = () => props.body.trim().replace(new RegExp(`^${macroName()}\\s*`, "i"), "").trim();
  const hostProperties = createMemo<[string, string][]>(() => {
    const id = props.blockId;
    const node = id ? docNode(id) : undefined;
    if (!id || !node) return [];
    return facetsOf(node.raw, formatForBlock(id)).properties.filter(([key]) => key.startsWith("tine."));
  }, [], { equals: sameJson });
  const parseRequest = createMemo<ReadingRequest>(
    () => ({ argument: arg(), name: macroName(), properties: hostProperties(), epoch: graphEpoch() }),
    { argument: "", name: "", properties: [], epoch: -1 },
    { equals: sameJson },
  );
  const parsed = createQueryReading(parseRequest);
  /** The authoring reading. Every display and editing derivation uses it. */
  const reading = (): ParsedQuery | undefined => (parsed.error === undefined ? parsed.latest?.reading : undefined);
  const source = (): Source | undefined => reading()?.query.source;
  const form = () => { const s = source(); return (s ? sourceOriginal(s) : null) ?? ""; };
  const opts = () => { const s = source(); return s ? sourceOptions(s) : ""; };
  // `:title` / `:collapsed?` / `:table-view?` are read out of the OPAQUE options
  // map, which the engine carries verbatim and does not interpret (§4.3, Y2).
  const titleOption = (): string | undefined => {
    const m = /:title\s+"((?:[^"\\]|\\.)*)"/.exec(opts());
    return m ? unquoteEdnString(m[1]) : undefined;
  };
  const isAdvanced = () => source()?.kind === "advanced";

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
    return argument === null ? undefined : { ...parseRequest(), argument };
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
    const page = isAdvanced() && declaresCurrentPageInput(form()) ? queryCurrentPage() : props.blockId ? docNode(props.blockId)?.page : undefined;
    return page ? { current_page: page } : undefined;
  };
  // A saved `(search "…")` query presents search hits with their evidence.
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
  const sheetFace = () => currentView() === "table" || currentView() === "board";
  const legacyTable = () => currentView() === "list" && /:table-view\?\s+true/.test(opts());
  const setQueryView = (next: QueryView) => {
    const blockId = props.blockId;
    const node = blockId ? docNode(blockId) : undefined;
    if (!blockId || !node) return;
    const storedView = blockProperty(blockId, "tine.view");
    if ((next === "list" && storedView === null) || (next !== "list" && storedView === next)) return;
    withUndoUnit(`query:view:${next}`, [node.page], () => {
      if (next === "list") {
        setBlockProperty(blockId, "tine.view", null);
        return;
      }
      setBlockProperty(blockId, "tine.view", next);
      if (next === "board" && blockProperty(blockId, "tine.group-by") === null) {
        setBlockProperty(blockId, "tine.group-by", "state");
      }
    });
  };

  const currentPage = () => props.currentPage ?? (props.blockId ? docNode(props.blockId)?.page : undefined);
  const collapseKey = () => JSON.stringify([graphMeta()?.root ?? "", props.blockId ?? currentPage() ?? "global", arg()]);
  const [collapseOverride, setCollapseOverride] = createSignal(loadCollapsed(collapseKey()));
  const collapsed = () => collapseOverride() ?? /:collapsed\?\s+true/.test(opts());
  const toggleCollapsed = () => {
    const v = !collapsed();
    setCollapseOverride(v);
    saveCollapsed(collapseKey(), v);
  };

  // Re-run when the reading changes OR after any save lands (dataRev). A
  // COLLAPSED query fetches once for its count and does not re-run a
  // whole-graph evaluation on every save while hidden.
  const runRequest = createMemo(() => {
    const query = runnable();
    if (!query) return undefined;
    const context = executionContext();
    const search = friendlySearch();
    const key = JSON.stringify([
      graphEpoch(), query.query, query.view, context ?? null, search, collapsed() ? "collapsed" : dataRev(),
    ]);
    return { query, context, search, key };
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
        () => backend().runGraphSearch(
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
        groups: [...grouped.values()], pages: null, diagnostics: [],
        report: null, search: hits.length === landed.value.hits.length ? landed.value : { ...landed.value, hits }, matchedTotal: null,
      };
    }
    const landed = await readOwned(owner, sharedQueryResult(
      scope,
      `ir\0${request.key}`,
      () => backend().queryRun(request.query.query, request.query.view, request.context),
    ));
    if (landed.kind === "stale") return undefined;
    const result = landed.value;
    return {
      groups: result.anchor === "block" ? withoutHostBlock(result.groups, props.blockId) : [],
      pages: result.anchor === "page" ? result.pages : null,
      diagnostics: result.diagnostics ?? [],
      report: result.report,
      statistics: result.statistics,
      search: null,
      matchedTotal: result.matched_total ?? null,
    };
  });
  /** The last coherent answer; an errored run shows its error, not old rows. */
  const displayed = (): QueryOperation | undefined => (operation.error === undefined ? operation.latest : undefined);
  const groups = () => displayed()?.groups ?? [];
  const pageRows = () => displayed()?.pages ?? null;
  // The run's OWN diagnostics (I-9): an invalid query returns zero rows plus
  // these, so they must render — "No results" alone would report a broken query
  // as an empty graph.
  const blockingDiagnostics = () => (displayed()?.diagnostics ?? []).filter((d) => !d.disabled);
  const advInfo = () => (isAdvanced() ? displayed()?.report ?? null : null);
  const readError = () => parsed.error ?? executionParsed.error;
  const loadError = () => operation.error ?? readError();
  // Presentation never changes membership: ordinary DSL results adapt into
  // evidence-free search rows for the Search presentation.
  const searchPresentationHits = createMemo<QueryHit[]>(() => {
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
    const pages = pageRows();
    if (pages) return displayed()?.matchedTotal ?? pages.length;
    if (friendlySearch() !== null || currentView() === "search") return searchPresentationHits().length;
    return groups().reduce((a, g) => a + g.blocks.length, 0);
  };
  const ranEmpty = () =>
    !!displayed() && !operation.loading && total() === 0 && blockingDiagnostics().length === 0;
  const emptyMessage = () => (!displayed() && !loadError() ? "Loading query results…" : "No results");

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
  const explanation = () => (explained.error === undefined ? explained() : undefined);
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
  // Rewrite just THIS macro inside the owning block, targeted by the extent's
  // own recovered name+argument (a block can hold more than one query).
  const rewriteMacro = (newMacro: string) => {
    if (!props.blockId) return;
    const raw = docNode(props.blockId)?.raw ?? "";
    const extents = queryMacroExtents(raw);
    if (!extents.length) return;
    const norm = (s: string) => s.replace(/\s+/g, " ").trim();
    const mine = norm(props.body);
    const target = extents.find((e) => norm(`${e.name} ${e.argument}`) === mine)
      ?? extents.find((e) => norm(raw.slice(e.start + 2, e.end - 2)) === mine)
      ?? extents[0];
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
  const applyEdit = async (next: BuilderSession): Promise<boolean> => {
    const blockId = props.blockId;
    if (!blockId || !docNode(blockId)) return false;
    const rawAtStart = docNode(blockId).raw;
    const owner = graphOwner(() => docNode(blockId)?.raw === rawAtStart);
    const current = macroName();
    let name = current;
    let dialect = macroPrintDialect(name);
    let printed: Owned<string>;
    try {
      const expressible = await readOwned(owner, backend().queryOgExpressible(next.query, next.view));
      if (expressible.kind === "stale") {
        setPrintError(BLOCK_CHANGED);
        return false;
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
      setPrintError(BLOCK_CHANGED);
      return false;
    }
    // A page that turned read-only while the print was in flight refuses the
    // write; report "not saved" so the sheet arms no focus (master 93ff682a3).
    if (!blockWritable(blockId)) return false;
    setPrintError(null);
    const argument = printed.value;
    const crossing = name.toLowerCase() !== current.toLowerCase();
    // ONE undo unit for the whole save; the tag is what the notice's Undo
    // recognises, so only a CROSSING save carries the crossing tag.
    withUndoUnit(crossing ? `query:cross:${blockId}` : `query:save:${blockId}`, [node.page], () => {
      rewriteMacro(`{{${name} ${argument}}}`);
      materializeView(blockId, next.view, dialect);
    });
    if (crossing) setCrossed(blockId, boundedFeature(argument));
    return true;
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
    const inner = opts().replace(/^\{|\}$/g, "").trim();
    const rest = inner.replace(/:title\s+"(?:[^"\\]|\\.)*"\s*/, "").trim();
    const title = t.trim().replace(/[\r\n{}]/g, "");
    const parts = [title ? `:title "${quoteEdnString(title)}"` : "", rest].filter(Boolean);
    const nextOptions = parts.length ? `{${parts.join(" ")}}` : "";
    if (nextOptions === opts()) return;
    const nextQuery: Query = { ...current.query, source: { ...current.query.source, og_options: nextOptions } as Source };
    const rawAtStart = docNode(blockId)?.raw;
    try {
      const printed = await readOwned(
        graphOwner(() => docNode(blockId)?.raw === rawAtStart),
        backend().printQuery(nextQuery, current.view, sourcePrintDialect(current.query.source), true),
      );
      if (printed.kind === "stale") {
        setPrintError(BLOCK_CHANGED);
        return;
      }
      setPrintError(null);
      const node = docNode(blockId);
      if (node) withUndoUnit(`query:title:${blockId}`, [node.page], () => rewriteMacro(`{{${macroName()} ${printed.value}}}`));
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
  const showCrossingNotice = () => !!crossedTag() && crossingNoticeDismissed() === false;
  // Undo is offered only while the entry `undo()` would take back IS the crossing save.
  const crossingIsStillUndoable = () => {
    const tag = crossedTag();
    return !!tag && undoTopTag() === tag;
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
        undo();
        dismissCrossing();
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
  const [sortCol, setSortCol] = createSignal<string>("");
  const [sortDir, setSortDir] = createSignal(1);
  const rows = createMemo<Row[]>(() =>
    groups().flatMap((g) =>
      g.blocks.map((b) => {
        const props: Record<string, string> = {};
        for (const [k, val] of b.properties ?? []) props[k] = val;
        return { page: g.page, kind: g.kind, path: g.path, text: visibleBody(b.raw).join(" "), props };
      })
    )
  );
  const cols = createMemo(() => {
    const keys = new Set<string>();
    for (const r of rows()) for (const k of Object.keys(r.props)) keys.add(k);
    return Array.from(keys);
  });
  const sorted = createMemo(() => {
    const c = sortCol();
    if (!c) return rows();
    const val = (r: Row) => (c === "page" ? r.page : c === "content" ? r.text : r.props[c] ?? "");
    return [...rows()].sort((a, b) => val(a).localeCompare(val(b)) * sortDir());
  });
  const sortBy = (c: string) => {
    if (sortCol() === c) setSortDir(-sortDir());
    else {
      setSortCol(c);
      setSortDir(1);
    }
  };
  // Clicks on query controls must not bubble to the block's onClick.
  const stop = (e: MouseEvent) => e.stopPropagation();
  const arrow = (c: string) => (sortCol() === c ? (sortDir() > 0 ? " ▲" : " ▼") : "");

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
            <table class="md-table query-why-empty-table">
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
            </table>
          </Show>
        </div>
      </Show>
    </Show>
  );
  const empty = () => (
    <div class="query-empty">
      {emptyMessage()} {whyEmpty()}
    </div>
  );

  return (
    <Show when={!hidden()}>
      <div class="query-block" classList={{ "query-sheet-block": sheetFace(), "query-stale": paneStale() }}>
        <Switch>
          <Match when={unsupportedAdvanced()}>
            <div class="query-unsupported" role={props.unsupportedLabel ? "alert" : undefined}>
              <Show
                when={props.unsupportedLabel}
                fallback={<>Advanced (datalog) query: no supported clauses. <code>{`{{${props.body}}}`}</code></>}
              >
                {(label) => <>{label()}: query contains unsupported clauses.</>}
              </Show>
            </div>
          </Match>
          <Match when={true}>
            <Show when={isAdvanced() && advInfo()?.supported}>
              <div class="query-adv-note">
                Partial datalog — ran: {(advInfo()!.ran ?? []).join(", ") || "—"}
                <Show when={(advInfo()!.ignored ?? []).length > 0}>
                  {` · ignored: ${advInfo()!.ignored!.join(", ")}`}
                </Show>
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
              <Show when={props.blockId}>
                <div class="query-view-switcher" role="group" aria-label="Query view" onClick={stop}>
                  <For each={QUERY_VIEWS}>
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
                paneDialect="tql"
                blockId={props.blockId}
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
            {/* The part below was not understood, so the query returned nothing
                — not "ignored", which would imply the rest ran (I-9). */}
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
                {(statistics) => <QueryStatisticsSummary statistics={statistics()} />}
              </Show>
              <Switch>
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
                    <Show when={(sheet()?.view === "table" || sheet()?.view === "board") && props.blockId}>
                      <SheetContainer>
                        <Switch>
                          <Match when={sheet()?.view === "table"}>
                            <SheetTable ownerId={props.blockId!} rowSource="query" groups={groups()} />
                          </Match>
                          <Match when={sheet()?.view === "board"}>
                            <SheetBoard ownerId={props.blockId!} rowSource="query" groupBy={sheet()?.groupBy} groups={groups()} />
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
                                onClick={() => openPageTarget({
                                  name: hit.page.name,
                                  pageKind: hit.page.kind,
                                  ...(hit.page.path ? { path: hit.page.path } : {}),
                                })}
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
                                onClick={() => openPageAtBlock({
                                  name: blockHit().page,
                                  pageKind: blockHit().kind,
                                  block: blockDtoExternalId(blockHit().block),
                                  ...(blockHit().path ? { path: blockHit().path } : {}),
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
                            <For each={[...groupedQueryByKey().keys()]}>
                              {(key) => <QueryGroup group={() => groupedQueryByKey().get(key)} />}
                            </For>
                          }
                        >
                          {/* Sorted: the engine's flat global order, one group per run of rows. */}
                          <For each={[...flatQueryByKey().keys()]}>
                            {(key) => <QueryGroup group={() => flatQueryByKey().get(key)} flat />}
                          </For>
                        </Show>
                      }
                    >
                      <table class="md-table query-table">
                        <thead>
                          <tr onClick={stop}>
                            <th onClick={() => sortBy("content")}>Content{arrow("content")}</th>
                            <th onClick={() => sortBy("page")}>Page{arrow("page")}</th>
                            <For each={cols()}>
                              {(c) => <th onClick={() => sortBy(c)}>{c}{arrow(c)}</th>}
                            </For>
                          </tr>
                        </thead>
                        <tbody>
                          <For each={sorted()}>
                            {(r) => (
                              <tr>
                                <td>
                                  <InlineText text={r.text} format={formatForPage(r.page)} />
                                </td>
                                <td
                                  class="qt-page"
                                  onClick={(e) => {
                                    e.stopPropagation();
                                    const target = { name: r.page, pageKind: r.kind, ...(r.path ? { path: r.path } : {}) };
                                    if (e.shiftKey) openPageInSidebar(target);
                                    else openPageTarget(target);
                                  }}
                                  onAuxClick={(e) => {
                                    if (e.button === 1) {
                                      e.preventDefault();
                                      e.stopPropagation();
                                      openPageTargetInNewTab({ name: r.page, pageKind: r.kind, ...(r.path ? { path: r.path } : {}) });
                                    }
                                  }}
                                  onContextMenu={(e) => {
                                    if (!shouldOpenTextContextMenu(e.target)) return;
                                    e.preventDefault();
                                    e.stopPropagation();
                                    openPageContextMenu(e.clientX, e.clientY, { name: r.page, pageKind: r.kind, ...(r.path ? { path: r.path } : {}) });
                                  }}
                                >
                                  {r.page}
                                </td>
                                <For each={cols()}>{(c) => <td>{r.props[c] ?? ""}</td>}</For>
                              </tr>
                            )}
                          </For>
                        </tbody>
                      </table>
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

// One page's query results, rendered as LIVE editable blocks. The result page
// is loaded into the shared working set on demand; each result is the same
// <Block> the main view uses (so editing a result edits the real block and
// saves to its page). Until the page is loaded, a read-only block stands in.
//
// Keyed by page name (outer <For>) and block uuid (inner <For>) so a reactive
// re-query that returns the same membership reuses the existing rows — it never
// re-mounts a block you're editing in a result and yanks the caret out.
function QueryGroup(props: { group: () => RefGroup | undefined; flat?: boolean }): JSX.Element {
  const kind = (): PageKind => props.group()?.kind ?? "page";
  const page = () => props.group()?.page ?? "";
  const target = () => ({ name: page(), pageKind: kind(), ...(props.group()?.path ? { path: props.group()!.path } : {}) });
  return (
    <Show when={props.group()}>
      {(g) => (
        <div class="query-group" classList={{ "query-group-flat": props.flat }}>
          <div
            class={props.flat ? "query-crumb" : "query-page"}
            onClick={(e) => {
              e.stopPropagation();
              if (e.shiftKey) openPageInSidebar(target());
              else openPageTarget(target());
            }}
            onAuxClick={(e) => {
              if (e.button === 1) {
                e.preventDefault();
                e.stopPropagation();
                openPageTargetInNewTab(target());
              }
            }}
            onContextMenu={(e) => {
              if (!shouldOpenTextContextMenu(e.target)) return;
              e.preventDefault();
              e.stopPropagation();
              openPageContextMenu(e.clientX, e.clientY, target());
            }}
          >
            {page()}
          </div>
          <LiveRefGroup page={page()} kind={kind()} path={g().path} blocks={g().blocks} surface="query" showBreadcrumb />
        </div>
      )}
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
    let script = document.getElementById(YOUTUBE_API_SCRIPT_ID) as HTMLScriptElement | null;
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
  const iframe = Array.from(document.getElementsByTagName("iframe"))
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
    // `?enablejsapi=1` matches OG (youtube.cljs:58) and, together with the
    // referrerpolicy below, is what makes the embed play under WebKitGTK — a bare
    // src with no referrer is rejected by YouTube's player as error 153.
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
  // YouTube (youtube.cljs:54-70) sends a `strict-origin-when-cross-origin`
  // referrer so the app origin reaches YouTube — without a referrer the player
  // fails with error 153. Vimeo (block.cljs:1290-1305) gets the same `allow` list
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
            ? <a class="external-link" href={safeUrl()} target="_blank" rel="noreferrer">{url()}</a>
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
      <a class="external-link tweet-link" href={safeUrl()} target="_blank" rel="noreferrer">
        🐦 {url()}
      </a>
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

  return (
    <div class="embed-block">
      <Show when={!selfPageEmbed()}>
        <Show when={data()} fallback={<div class="embed-missing">{`{{${props.body}}}`}</div>}>
          <LiveRefGroup page={data()!.page} kind={data()!.kind} blocks={data()!.blocks} embedId={data()!.embedId} surface="embed" />
        </Show>
      </Show>
    </div>
  );
}
