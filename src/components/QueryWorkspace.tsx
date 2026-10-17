import { shortcutKey } from "../shortcutKey";
import { createMembershipResource } from "../resultMembership";
import {
  For,
  Match,
  Show,
  Switch,
  createEffect,
  createMemo,
  createResource,
  createSignal,
  createUniqueId,
  onCleanup,
  on,
  type JSX,
} from "solid-js";
import { backend, isTauri } from "../backend";
import { pinyinMatchingEnabled } from "../launcherRanking";
import { bindingIdentity, captureBinding } from "../binding";
import { bindingOwner, advanceRevision, currentRevision, graphOwner, latestOwner, ownedWhen, readOwned, revisionOwner, writeOwned } from "../owned";
import { pushToast } from "../toasts";
import { errorFamily } from "../errorFamily";
import {
  friendlySearchToDsl,
  friendlySearchToSavedDsl,
  parseSearchQuery,
} from "../editor/searchQuery";
import type { PaneRouter, QueryPresentation, QueryRoute, Route } from "../router";
import type {
  MatchSpan,
  PageDto,
  ResolvedPage,
  QueryDiagnostic,
  QueryExecution,
  QueryExplainNode,
  QueryHit,
  QueryPageScope,
} from "../types";
import { VIEW_KINDS, type ParsedQuery, type Query, type QueryResult, type ViewSettings, type ExplainEmptyResult, type FriendlyPageMatchScope } from "../editor/queryIr";
import { queryDisplaySettings, type QueryDisplayDraft } from "../editor/queryDisplayDraft";
import { displayPropertyPatch, queryScopedDisplayPropertyPatch } from "../editor/queryViewProperties";
import { QueryBuilder, type BuilderSession } from "./QueryBuilder";
import { createQueryRegistryAccess } from "./QueryBuilder";
import { QueryDisplay } from "./QueryDisplay";
import { queryRecordColumns } from "../queryResultColumns";
import { QueryResultsSelection } from "./QueryResultsSelection";
import { SourceTaskCheckbox } from "./SourceTaskCheckbox";
import { QUERY_MACRO_NAMES } from "../editor/queryMacroName";
import { queryWorkspaceDsl } from "./queryWorkspaceDsl";
import { SearchResultRow } from "./SearchResultRow";
import { QueryPageResults, type QueryPageHit } from "./QueryPageResults";
import { QueryResultSections } from "./QueryResultSections";
import { registerTransientLayer } from "../transientLayers";
import { bumpPageInventoryRev } from "../graphSession";
import { blockDtoExternalId } from "../blockIdentity";
import { createPage, CreatePageRefusal, queryWorkspacePage } from "../document";
import { internalLinkDest, internalLinkMouseDown, internalLinkAuxClick } from "../linkGesture";
import { steppedResultIndex } from "../resultNavigation";
import { openRouteInOtherPane } from "../panes";
import { openPageInSidebar, openBlockInSidebar, openPageContextMenu } from "../ui";
import { shouldOpenTextContextMenu } from "../contextMenuPolicy";
import { readLatestOr, readOr } from "../resourceRead";
import { documentOf } from "../windowRealm";
import type { QueryTextDialect } from "../editor/queryIr";
import { QueryOgHint } from "./QueryResultParts";
import { t, msg, render } from "../i18n";

const PAGE_LIMIT = 40;
const BLOCK_LIMIT = 100;

export interface MaterializeQueryInput {
  title: string;
  sourceKind: QueryRoute["sourceKind"];
  source: string;
  presentation: QueryPresentation;
  pageMatchScope?: FriendlyPageMatchScope;
  /** Explicit scoped choices to persist with the ordinary query block. */
  pagePresentation?: QueryPresentation;
  blockPresentation?: QueryPresentation;
  pageDisplay?: QueryDisplayDraft;
  blockDisplay?: QueryDisplayDraft;
  /** Stable workspace identity: also bounds the native validation cancellation lane. */
  routeId: string;
}

export interface MaterializeQueryDependencies {
  serializeDsl(source: string): Promise<{ query: string; properties: Array<[string, string]> }>;
  /** The one name answerer: an existing file, an alias, or where a new page goes. */
  resolvePage(name: string, kind: "page"): Promise<ResolvedPage>;
  /** Write the new page through the page host (`createPage`); resolves its file revision when known. */
  createPage(page: PageDto, bindingGeneration: number): Promise<string | null>;
  /** Rust-authoritative friendly-search validation; required before every nonblank friendly save. */
  /** One graph-scale search; optional page membership is independent of the
   * physical-page restriction. Effective views order and sample their own
   * sections before bounding. Native refusal rejects; callers show the error. */
  runGraphSearch(source: string, pageLimit: number, blockLimit: number, lane: string, explain: boolean, scope?: QueryPageScope, pageMatchScope?: FriendlyPageMatchScope, views?: { page: ViewSettings; block: ViewSettings }): Promise<QueryExecution>;
}

export type MaterializeQueryResult =
  | { ok: true; name: string; page: PageDto; rev: string | null }
  | {
      ok: false;
      kind: "invalid-name" | "empty-query" | "invalid-query" | "exists" | "conflict" | "error" | "superseded";
      message: string;
    };

export interface QueryWorkspaceDependencies extends MaterializeQueryDependencies {
  /** Close this surface's two native cancellation lanes; O(1), no graph write. */
  closeSearchWorkspace?(workspace: string, bindingGeneration: number): Promise<void>;
  parseQuery(source: string, dialect: QueryTextDialect): Promise<ParsedQuery>;
  queryRun(query: Query, view: ViewSettings): Promise<QueryResult>;
  queryExplainEmpty(query: Query, view: ViewSettings): Promise<ExplainEmptyResult>;
}

export interface QueryWorkspaceProps {
  route: QueryRoute;
  router: PaneRouter;
  /** Dependency injection keeps create/save races and rendering testable without IPC. */
  deps?: QueryWorkspaceDependencies;
  focusSource?: boolean;
}

/** The query block's text and its properties, in the order they have always been written. */
async function savedQuery(input: Pick<MaterializeQueryInput, "source" | "sourceKind" | "presentation" | "pageMatchScope" | "pagePresentation" | "blockPresentation" | "pageDisplay" | "blockDisplay">, deps: MaterializeQueryDependencies): Promise<{ query: string; properties: Array<[string, string]> }> {
  const source = input.source.trim();
  const dsl = input.sourceKind === "search" ? friendlySearchToSavedDsl(source) : source;
  const properties: Array<[string, string]> = [];
  if (input.presentation !== "list") properties.push(["tine.view", input.presentation]);
  if (input.sourceKind === "search" && input.pageMatchScope) properties.push(["tine.page-match-scope", input.pageMatchScope]);
  for (const kind of ["page", "block"] as const) {
    for (const [key, value] of queryScopedDisplayPropertyPatch({
      scope: kind,
      presentation: input[kind === "page" ? "pagePresentation" : "blockPresentation"],
      display: input[kind === "page" ? "pageDisplay" : "blockDisplay"],
      properties: [],
    })) if (value !== null) properties.push([key, value]);
  }
  if (input.sourceKind === "search") return { query: `{{${QUERY_MACRO_NAMES[0]} ${dsl}}}`, properties };
  const saved = await readOwned(bindingOwner(), deps.serializeDsl(dsl));
  if (saved.kind === "stale") throw new Error(render(SUPERSEDED_MESSAGE));
  // A converted DSL workspace becomes one ordinary query block. Its effective
  // display must also be singular so the saved block keeps its sort/sample.
  if (input.blockDisplay && queryWorkspaceDsl(dsl).dialect === "macro_tql") {
    for (const [key, value] of displayPropertyPatch({ ...input.blockDisplay, view: input.blockPresentation ?? input.presentation })) {
      if (value !== null) properties.push([key, value]);
    }
  }
  return { query: saved.value.query, properties: [...saved.value.properties, ...properties] };
}

/** Preserve authored OG text when expressible; convert Tine-only conditions
 * through the engine's printer before the guarded page save. */
export async function serializeWorkspaceDsl(source: string): Promise<{ query: string; properties: Array<[string, string]> }> {
  const api = backend();
  const owner = graphOwner();
  const input = queryWorkspaceDsl(source);
  const parsed = await readOwned(owner, api.parseQuery(input.argument, input.dialect));
  if (parsed.kind === "stale") throw new Error(render(SUPERSEDED_MESSAGE));
  const { query, view } = parsed.value;
  const expressible = query.source.kind === "advanced" ? null : await readOwned(owner, api.queryOgExpressible(query, view));
  if (expressible?.kind === "stale") throw new Error(render(SUPERSEDED_MESSAGE));
  if (query.source.kind === "advanced" || expressible?.value) {
    // The argument is in the dialect it was typed in: a Tine macro stays one.
    return { query: input.dialect === "macro_tql" ? input.macro : `{{${QUERY_MACRO_NAMES[0]} ${input.argument}}}`, properties: [] };
  }
  const printed = await readOwned(owner, api.printQuery(query, view, "tql_macro"));
  if (printed.kind === "stale") throw new Error(render(SUPERSEDED_MESSAGE));
  // TQL keeps conditions in text and view directives on the owning block.
  const properties: Array<[string, string]> = [];
  if (view.sort?.length) properties.push(["tine.sort", view.sort.map(([field, direction]) => `${field} ${direction}`).join("; ")]);
  if (view.sample != null) properties.push(["tine.sample", String(view.sample)]);
  return { query: `{{${QUERY_MACRO_NAMES[1]} ${printed.value}}}`, properties };
}

/** Is the input this attempt captured still the one the user is looking at?
 *  Supplied by the component that owns the workspace; a direct caller is unguarded. */
export type IsCurrentInput = () => boolean;

/** A LOCAL refusal: nothing was written, nothing was undone; saving again is the remedy. */
const SUPERSEDED_MESSAGE = msg("query-save-superseded");

/**
 * Materialize a virtual workspace as exactly one ordinary query block.
 *
 * The preflight existence check provides a friendly error. An explicit Page match
 * scope is stored as `tine.page-match-scope` on the query block. The race guard
 * is the audited no-baseline save (`null`, never force): if another
 * writer creates the page between the two calls, the backend rejects it as a
 * conflict and this workspace remains virtual.
 */
export async function materializeQueryWorkspace(
  input: MaterializeQueryInput,
  deps: MaterializeQueryDependencies,
  isCurrent: IsCurrentInput = () => true
): Promise<MaterializeQueryResult> {
  input = { ...input };
  const superseded = (): MaterializeQueryResult => ({ ok: false, kind: "superseded", message: render(SUPERSEDED_MESSAGE) });
  if (!isCurrent()) return superseded();
  const binding = captureBinding();
  const owner = bindingOwner();
  const name = input.title.trim();
  if (!name) {
    return { ok: false, kind: "invalid-name", message: t("query-save-enter-title") };
  }
  if (!input.source.trim()) {
    return { ok: false, kind: "empty-query", message: t("query-save-enter-query") };
  }
  if (input.sourceKind === "search") {
    try {
      const search = await readOwned(owner, deps.runGraphSearch(input.source.trim(), 0, 0, `query-workspace:${input.routeId}:materialize`, true));
      if (search.kind === "stale") return { ok: false, kind: "error", message: t("query-save-graph-changed") };
      const execution = search.value;
      if (!isCurrent()) return superseded();
      if (execution.cancelled) return { ok: false, kind: "invalid-query", message: t("query-save-validation-superseded") };
      if (execution.diagnostics.length) return { ok: false, kind: "invalid-query", message: execution.diagnostics.map((item) => item.message).join(" · ") };
      if (!execution.explanation.branches.length) return { ok: false, kind: "empty-query", message: t("query-save-included-term") };
    } catch (error) {
      if (!isCurrent()) return superseded();
      const detail = error instanceof Error ? error.message : String(error);
      return { ok: false, kind: "invalid-query", message: detail ? t("query-save-validation-detail", { detail }) : t("query-save-validation-failed") };
    }
  }

  try {
    const resolution = await readOwned(owner, deps.resolvePage(name, "page"));
    if (resolution.kind === "stale") return { ok: false, kind: "error", message: t("query-save-graph-changed") };
    const resolved = resolution.value;
    // An answer about a title the workspace has moved on from no longer
    // licenses a write; serialization below rechecks the same captured input.
    if (!isCurrent()) return superseded();
    if (resolved.kind === "existing") {
      return {
        ok: false,
        kind: "exists",
        message: t("query-save-title-exists", { name }),
      };
    }
    // An alias names another page: refuse, write nothing, keep the query here.
    if (resolved.kind === "alias") {
      return {
        ok: false,
        kind: "exists",
        message: t("query-save-title-alias", { name }),
      };
    }

    // The new page's format is its resolved id's extension (the graph's preferred
    // format); the save path writes by that extension, so the block must be
    // spelled for it.
    const serialized = await readOwned(owner, savedQuery(input, deps));
    if (serialized.kind === "stale") return { ok: false, kind: "error", message: t("query-save-graph-changed") };
    if (!isCurrent()) return superseded();
    const { query, properties } = serialized.value;
    const page = queryWorkspacePage(name, query, properties, /\.org$/i.test(resolved.id) ? "org" : "md");
    const saved = await writeOwned(owner, deps.createPage(page, binding.backendGeneration));
    if (saved.kind === "stale") return { ok: false, kind: "error", message: t("query-save-graph-changed") };
    const rev = saved.value;
    if (!owner()) return { ok: false, kind: "error", message: t("query-save-graph-changed") };
    bumpPageInventoryRev();
    return { ok: true, name, page, rev };
  } catch (error) {
    const detail = error instanceof Error ? error.message : String(error);
    if (!isCurrent()) return superseded();
    if (!owner()) {
      pushToast(msg("query-query-workspace-could-not-save", { value1: name, value2: detail }), "error");
      return { ok: false, kind: "error", message: t("query-save-graph-changed") };
    }
    if (error instanceof CreatePageRefusal) {
      if (error.reason === "graph-changed" || error.reason === "stale-binding")
        return { ok: false, kind: "error", message: t("query-save-graph-changed") };
      if (error.reason === "alias")
        return { ok: false, kind: "exists", message: t("query-save-title-alias", { name }) };
      return { ok: false, kind: "error", message: t("query-save-local-state-changed", { name }) };
    }
    if (errorFamily(error) === "conflict") {
      return {
        ok: false,
        kind: "conflict",
        message: t("query-save-conflict", { name }),
      };
    }
    return {
      ok: false,
      kind: "error",
      message: detail ? t("query-save-failed-detail", { name, detail }) : t("query-save-failed", { name }),
    };
  }
}

function defaultDependencies(): QueryWorkspaceDependencies {
  const api = backend();
  return {
    serializeDsl: serializeWorkspaceDsl,
    closeSearchWorkspace: async (workspace: string, bindingGeneration: number) => {
      if (!isTauri()) return;
      const owner = graphOwner();
      const imported = await readOwned(owner, import("@tauri-apps/api/core"));
      if (imported.kind === "stale" || !owner()) return;
      await imported.value.invoke<void>("close_search_workspace", { workspace, bindingGeneration });
    },
    resolvePage: (name, kind) => api.resolvePage(name, kind),
    createPage: (page, bindingGeneration) => createPage(page.name, page, { bindingGeneration }),
    runGraphSearch: (source, pageLimit, blockLimit, lane, explain, scope, pageMatchScope, views) =>
      api.runGraphSearch(source, pageLimit, blockLimit, lane, explain, scope, pageMatchScope, views, pinyinMatchingEnabled()),
    parseQuery: (source, dialect) => api.parseQuery(source, dialect),
    queryRun: (query, view) => api.queryRun(query, view),
    queryExplainEmpty: (query, view) => api.queryExplainEmpty(query, view),
  };
}

function irToExecution(result: QueryResult, explanation?: ExplainEmptyResult): QueryExecution {
  const hits: QueryHit[] = result.anchor === "page"
    ? result.pages.map((page) => ({
      entity: "page" as const,
      page: { name: page.name, kind: page.kind, path: page.path, date_key: null },
      row: page,
      display_text: page.name,
      evidence: [],
      score: 0,
    }))
    : result.groups.flatMap((group) => group.blocks.map((block) => ({
    entity: "block" as const,
    page: group.page,
    kind: group.kind,
    path: group.path,
    block,
    display_text: block.raw,
    evidence: [],
  })));
  const diagnostics: QueryDiagnostic[] = (result.diagnostics ?? [])
    .filter((item) => !item.disabled)
    .map((item) => ({ code: item.kind, message: item.message }));
  for (const clause of result.report.ignored ?? []) diagnostics.push({
    code: "unsupported_clause", message: t("query-unsupported-clause", { clause }),
  });
  return {
    hits,
    diagnostics,
    cancelled: false,
    explanation: {
      branches: explanation?.rows.map((row) => ({
        get description() { return t("query-workspace-explanation", { conjunct: row.conjunct, alone: row.alone, without: row.without ?? 0, hasWithout: row.without == null ? 0 : 1 }); },
        children: [],
      })) ?? [],
    },
  };
}

function hitSpans(hit: QueryHit): MatchSpan[] {
  const field = hit.entity === "page" ? "page_name" : "visible_content";
  return hit.evidence.filter((item) => item.field === field).flatMap((item) => item.spans);
}

function hitPage(hit: QueryHit): string {
  return hit.entity === "page" ? hit.page.name : hit.page;
}

function hitKind(hit: QueryHit): string {
  return hit.entity === "page" ? t("query-prose-page") : t("query-prose-block");
}

function MarkedText(props: { text: string; spans: MatchSpan[] }): JSX.Element {
  const segments = () => {
    const spans = props.spans
      .map((span) => ({
        start: Math.max(0, Math.min(props.text.length, span.start)),
        end: Math.max(0, Math.min(props.text.length, span.end)),
      }))
      .filter((span) => span.end > span.start)
      .sort((a, b) => a.start - b.start || a.end - b.end);
    const merged: MatchSpan[] = [];
    for (const span of spans) {
      const previous = merged[merged.length - 1];
      if (previous && span.start <= previous.end) previous.end = Math.max(previous.end, span.end);
      else merged.push({ ...span });
    }
    const out: { text: string; marked: boolean }[] = [];
    let cursor = 0;
    for (const span of merged) {
      if (span.start > cursor) out.push({ text: props.text.slice(cursor, span.start), marked: false });
      out.push({ text: props.text.slice(span.start, span.end), marked: true });
      cursor = span.end;
    }
    if (cursor < props.text.length) out.push({ text: props.text.slice(cursor), marked: false });
    return out;
  };
  return (
    <For each={segments()}>{(segment) => segment.marked
      ? <mark>{segment.text}</mark>
      : segment.text}</For>
  );
}

function ExplainTree(props: { nodes: QueryExplainNode[] }): JSX.Element {
  return (
    <ul class="query-explain-tree">
      <For each={props.nodes}>{(node) => (
        <li>
          <span>{node.description}</span>
          <Show when={node.children.length}>
            <ExplainTree nodes={node.children} />
          </Show>
        </li>
      )}</For>
    </ul>
  );
}

function friendlySummary(source: string): string {
  const parsed = parseSearchQuery(source);
  if (parsed.kind === "empty") return t("query-prose-type-to-search-page-names-and-block-text");
  if (parsed.kind === "invalid") return t("query-prose-invalid-regex", { error: parsed.error });
  if (parsed.kind === "regex") return t("query-prose-regex-summary", { pattern: parsed.pattern });
  const describeGroup = (group: typeof parsed.groups[number]) => group.map(term =>
    t("query-prose-search-term", {
      kind: term.negated ? (term.quoted ? "exclude-exact" : "exclude") : (term.quoted ? "exact" : "contains"),
      value: term.text,
    })
  ).reduce((left, right) => t("query-prose-search-and", { left, right }));
  const groups = parsed.groups.map(describeGroup);
  return groups.length === 1
    ? t("query-prose-search-summary", { group: groups[0] })
    : t("query-prose-search-groups-summary", { groups: groups.reduce((left, right) => t("query-prose-search-or", { left, right })) });
}

function filterWords(value: string): string[] {
  return value.trim().split(/\s+/).filter(Boolean);
}

interface FriendlyFields {
  all: string;
  any: string;
  exact: string;
  exclude: string;
  regex: string;
}

function buildFriendlyFilterSource(fields: FriendlyFields): { source: string; error: string | null } {
  const regex = fields.regex.trim();
  const hasOther = [fields.all, fields.any, fields.exact, fields.exclude].some((value) => value.trim());
  if (regex) {
    if (hasOther) {
      return { source: "", error: t("query-fields-regex-warning") };
    }
    const source = `/${regex}/`;
    const parsed = parseSearchQuery(source);
    return parsed.kind === "invalid" ? { source: "", error: parsed.error } : { source, error: null };
  }

  if ([fields.all, fields.any, fields.exact, fields.exclude].some((value) => value.includes('"'))) {
    return { source: "", error: t("query-fields-quotation-warning") };
  }
  const common = [
    ...filterWords(fields.all),
    ...(fields.exact.trim() ? [`"${fields.exact.trim()}"`] : []),
    ...filterWords(fields.exclude).map((term) => `-${term}`),
  ];
  const alternatives = filterWords(fields.any);
  if (!common.some((term) => !term.startsWith("-")) && !alternatives.length) {
    return { source: "", error: t("query-fields-include-warning") };
  }
  const branches = alternatives.length
    ? alternatives.map((term) => [...common, term].join(" "))
    : [common.join(" ")];
  return { source: branches.join(" OR "), error: null };
}

/** Split the friendly grammar into Gmail-like fields only when that is lossless. */
function friendlyFieldsFromSource(source: string): FriendlyFields | null {
  const parsed = parseSearchQuery(source);
  const empty: FriendlyFields = { all: "", any: "", exact: "", exclude: "", regex: "" };
  if (parsed.kind === "empty") return empty;
  if (parsed.kind === "regex") return { ...empty, regex: parsed.pattern };
  if (parsed.kind !== "boolean") return null;

  const termKey = (term: typeof parsed.groups[number][number]) =>
    `${term.negated ? "-" : "+"}\0${term.quoted ? "q" : "w"}\0${term.text}`;
  const commonKeys = new Set(parsed.groups[0].map(termKey));
  for (const group of parsed.groups.slice(1)) {
    const keys = new Set(group.map(termKey));
    for (const key of [...commonKeys]) if (!keys.has(key)) commonKeys.delete(key);
  }
  const common = parsed.groups[0].filter((term) => commonKeys.has(termKey(term)));
  const remainder = parsed.groups.map((group) => group.filter((term) => !commonKeys.has(termKey(term))));
  const alternatives = remainder.every((group) => group.length === 0)
    ? []
    : remainder.every((group) => group.length === 1 && !group[0].negated && !group[0].quoted)
      ? remainder.map((group) => group[0].text)
      : null;
  const exact = common.filter((term) => !term.negated && term.quoted);
  if (alternatives === null || exact.length > 1 || common.some((term) => term.negated && term.quoted)) {
    return null;
  }
  return {
    all: common.filter((term) => !term.negated && !term.quoted).map((term) => term.text).join(" "),
    any: alternatives.join(" "),
    exact: exact[0]?.text ?? "",
    exclude: common.filter((term) => term.negated).map((term) => term.text).join(" "),
    regex: "",
  };
}

function focusableElements(root: HTMLElement): HTMLElement[] {
  return [...root.querySelectorAll<HTMLElement>(
    'button:not([disabled]), input:not([disabled]), textarea:not([disabled]), select:not([disabled]), summary, [href], [tabindex]:not([tabindex="-1"])'
  )].filter((element) => !element.hasAttribute("hidden") && !element.closest("details:not([open])"));
}

function AdvancedModal(props: {
  source: () => string;
  sourceKind: () => QueryRoute["sourceKind"];
  onApply: (source: string, sourceKind: QueryRoute["sourceKind"]) => void;
  onClose: () => void;
  layerId: string;
  trigger: () => HTMLElement | null;
}): JSX.Element {
  const initialFields = props.sourceKind() === "search" ? friendlyFieldsFromSource(props.source()) : null;
  const [all, setAll] = createSignal(initialFields?.all ?? "");
  const [any, setAny] = createSignal(initialFields?.any ?? "");
  const [exact, setExact] = createSignal(initialFields?.exact ?? "");
  const [exclude, setExclude] = createSignal(initialFields?.exclude ?? "");
  const [regex, setRegex] = createSignal(initialFields?.regex ?? "");
  const [rawFriendly, setRawFriendly] = createSignal(props.sourceKind() === "search" ? props.source() : "");
  const [structuredFriendly] = createSignal(initialFields !== null);
  const [friendlyDirty, setFriendlyDirty] = createSignal(false);
  const [draftKind, setDraftKind] = createSignal<QueryRoute["sourceKind"]>(props.sourceKind());
  const [dsl, setDsl] = createSignal(props.sourceKind() === "dsl" ? props.source() : "");
  const [error, setError] = createSignal<string | null>(null);
  // The engine reads and prints the draft in the grammar its envelope carries.
  let mounted = true;
  onCleanup(() => { mounted = false; });
  const edits = {};
  const [printing, setPrinting] = createSignal(false);
  const [builderSession, { mutate }] = createResource(dsl, async (text): Promise<BuilderSession | undefined> => {
    const owner = graphOwner(() => mounted && dsl() === text, revisionOwner(edits, currentRevision(edits)));
    const source = queryWorkspaceDsl(text);
    const parsed = await readOwned(owner, backend().parseQuery(source.argument, source.dialect));
    return parsed.kind === "current" && owner() ? { query: parsed.value.query, view: parsed.value.view } : readLatestOr(builderSession, undefined, "query text");
  });
  const applyBuilderEdit = async (next: BuilderSession) => {
    // Publish the accepted tree before printing: the next gesture composes with
    // it. Only the latest print may replace DSL; older parses cannot erase edits.
    advanceRevision(edits);
    mutate(next);
    setPrinting(true);
    const owner = latestOwner(edits, "print", graphOwner(() => mounted));
    try {
      const source = queryWorkspaceDsl(dsl());
      if (source.dialect === "macro_tql") {
        const printed = await readOwned(owner, backend().printQuery(next.query, next.view, "tql_macro"));
        if (printed.kind === "stale" || !owner()) return;
        setDsl(`{{${source.name} ${printed.value}}}`);
      } else {
        // OG text stays OG when it can say the edit; otherwise it becomes a
        // Tine query (D-18) rather than silently losing the condition.
        const expressible = await readOwned(owner, backend().queryOgExpressible(next.query, next.view));
        if (expressible.kind === "stale" || !owner()) return;
        const printed = await readOwned(owner, backend().printQuery(next.query, next.view, expressible.value ? "og" : "tql_macro"));
        if (printed.kind === "stale" || !owner()) return;
        setDsl(!expressible.value ? `{{${QUERY_MACRO_NAMES[1]} ${printed.value}}}`
          : source.wrapped ? `{{${source.name} ${printed.value}}}` : printed.value);
      }
      setError(null);
    } catch (failure) {
      // The printer owns the refusal; leave the draft unchanged on failure.
      if (owner()) setError(failure instanceof Error ? failure.message : String(failure));
    } finally { if (owner()) setPrinting(false); }
  };
  let dialog!: HTMLDivElement;
  let firstField: HTMLElement | undefined;

  createEffect(() => {
    const unregister = registerTransientLayer({
      id: props.layerId,
      root: () => dialog ?? null,
      trigger: props.trigger,
      dismiss: () => { props.onClose(); return true; },
    });
    onCleanup(unregister);
  });

  queueMicrotask(() => (firstField ?? dialog)?.focus());

  const apply = () => {
    if (draftKind() === "dsl") {
      if (printing() || error()) return;
      if (!dsl().trim()) {
        setError(t("query-dsl-empty-warning"));
        return;
      }
      props.onApply(dsl().trim(), "dsl");
      return;
    }
    const rawValidation = friendlySearchToDsl(rawFriendly());
    const built = !friendlyDirty()
      ? { source: props.source().trim(), error: friendlySearchToDsl(props.source()).error }
      : structuredFriendly()
        ? buildFriendlyFilterSource({ all: all(), any: any(), exact: exact(), exclude: exclude(), regex: regex() })
        : rawValidation.error
          ? { source: "", error: rawValidation.error }
          : { source: rawFriendly().trim(), error: null };
    if (built.error) {
      setError(built.error);
      return;
    }
    props.onApply(built.source, "search");
  };

  const switchToDsl = () => {
    const rawValidation = friendlySearchToDsl(rawFriendly());
    const friendly = !friendlyDirty()
      ? { source: props.source().trim(), error: friendlySearchToDsl(props.source()).error }
      : structuredFriendly()
        ? buildFriendlyFilterSource({ all: all(), any: any(), exact: exact(), exclude: exclude(), regex: regex() })
        : { source: rawFriendly().trim(), error: rawValidation.error };
    if (friendly.error) {
      setError(friendly.error);
      return;
    }
    const converted = friendlySearchToDsl(friendly.source);
    if (converted.error) {
      setError(converted.error);
      return;
    }
    setDsl(converted.dsl);
    setDraftKind("dsl");
    setError(null);
  };

  return (
    <div class="modal-overlay query-advanced-overlay" onMouseDown={(event) => {
      if (event.target === event.currentTarget) props.onClose();
    }}>
      <div
        ref={dialog}
        class="modal query-advanced-modal"
        role="dialog"
        aria-modal="true"
        aria-labelledby="query-advanced-title"
        tabIndex={-1}
        onKeyDown={(event) => {
          if (event.key === "Tab") {
            const focusable = focusableElements(dialog);
            const first = focusable[0];
            const last = focusable[focusable.length - 1];
            if (!first || !last) return;
            if (event.shiftKey && documentOf(event).activeElement === first) {
              event.preventDefault();
              last.focus();
            } else if (!event.shiftKey && documentOf(event).activeElement === last) {
              event.preventDefault();
              first.focus();
            }
          }
        }}
      >
        <header class="query-advanced-header">
          <div>
            <h2 id="query-advanced-title">{t("query-query-workspace-filters-and-advanced-query")}</h2>
            <p>{t("query-query-workspace-use-the-friendly-fields-or-switch-losslessly-to-the-visual")}</p>
          </div>
          <button type="button" aria-label={t("query-query-workspace-close-filters")} onClick={props.onClose}>×</button>
        </header>

        <Show when={draftKind() === "search"} fallback={
          <div class="query-dsl-editor">
            <QueryBuilder
              session={() => (readLatestOr(builderSession, undefined, "query text"))}
              onChange={(next) => void applyBuilderEdit(next)}
              paneDialect={queryWorkspaceDsl(dsl()).dialect === "macro_tql" ? "tql" : "og"}
              sheetAlwaysOpen
              parentTransientId={props.layerId}
            />
            <p class="query-advanced-note">{t("query-query-workspace-switching-back-to-friendly-fields-is-offered-only-when-it")}</p>
          </div>
        }>
          <Show when={structuredFriendly()} fallback={
            <div class="query-friendly-raw">
              <p>{t("query-query-workspace-this-search-uses-a-combination-that-cannot-be-split-into")}</p>
              <label>
                {t("query-query-workspace-friendly-search-syntax")}<textarea
                  ref={(element) => { firstField = element; }}
                  rows={4}
                  value={rawFriendly()}
                  onInput={(event) => { setRawFriendly(event.currentTarget.value); setFriendlyDirty(true); setError(null); }}
                  spellcheck={false}
                />
              </label>
              <button type="button" class="query-switch-to-dsl" onClick={switchToDsl}>
                {t("query-query-workspace-edit-as-visual-query")}</button>
            </div>
          }>
          <div class="query-friendly-fields">
            <label>
              {t("query-query-workspace-all-of-these-words")}<input ref={(element) => { firstField = element; }} value={all()} onInput={(event) => { setAll(event.currentTarget.value); setFriendlyDirty(true); }} />
            </label>
            <label>
              {t("query-query-workspace-any-of-these-words")}<input value={any()} onInput={(event) => { setAny(event.currentTarget.value); setFriendlyDirty(true); }} />
            </label>
            <label>
              {t("query-query-workspace-this-exact-phrase")}<input value={exact()} onInput={(event) => { setExact(event.currentTarget.value); setFriendlyDirty(true); }} />
            </label>
            <label>
              {t("query-query-workspace-exclude-these-words")}<input value={exclude()} onInput={(event) => { setExclude(event.currentTarget.value); setFriendlyDirty(true); }} />
            </label>
            <label>
              {t("query-query-workspace-case-sensitive-regular-expression")}<input value={regex()} onInput={(event) => { setRegex(event.currentTarget.value); setFriendlyDirty(true); }} placeholder={t("query-query-workspace-pattern-without")} />
            </label>
            <button type="button" class="query-switch-to-dsl" onClick={switchToDsl}>
              {t("query-query-workspace-edit-as-visual-query")}</button>
          </div>
          </Show>
        </Show>

        <Show when={error()}>
          <p class="query-advanced-error" role="alert">{error()}</p>
        </Show>
        <footer class="query-advanced-actions">
          <button type="button" onClick={props.onClose}>{t("query-query-sheet-parts-cancel")}</button>
          <button type="button" class="primary" disabled={printing()} onClick={apply}>{t("query-query-sheet-parts-apply")}</button>
        </footer>
      </div>
    </div>
  );
}

export function QueryWorkspace(props: QueryWorkspaceProps): JSX.Element {
  const deps = () => props.deps ?? defaultDependencies();
  const [source, setSource] = createSignal(props.route.source);
  const [sourceKind, setSourceKind] = createSignal(props.route.sourceKind);
  const [presentation, setPresentation] = createSignal(props.route.presentation);
  const [pagePresentation, setPagePresentation] = createSignal(props.route.pagePresentation);
  const [blockPresentation, setBlockPresentation] = createSignal(props.route.blockPresentation);
  const [pageDisplay, setPageDisplay] = createSignal(props.route.pageDisplay);
  const [blockDisplay, setBlockDisplay] = createSignal(props.route.blockDisplay);
  const [pageMatchScope, setPageMatchScope] = createSignal<FriendlyPageMatchScope>(props.route.pageMatchScope ?? "names");
  const [pageMatchScopeExplicit, setPageMatchScopeExplicit] = createSignal(props.route.pageMatchScope !== undefined);
  const [explain, setExplain] = createSignal(false);
  const [advancedOpen, setAdvancedOpen] = createSignal(false);
  const [title, setTitle] = createSignal("");
  const [saveError, setSaveError] = createSignal<string | null>(null);
  /** A stale save that COMMITTED. Not an error: the page exists, and saying so is
   *  the only honest thing left once the write has landed. */
  const [saveNotice, setSaveNotice] = createSignal<string | null>(null);
  const [saving, setSaving] = createSignal(false);
  const [dslReading, setDslReading] = createSignal<{ source: string; parsed: ParsedQuery }>();
  const [conversionUndo, setConversionUndo] = createSignal<{ source: string; converted: string; pageDisplay?: QueryDisplayDraft; blockDisplay?: QueryDisplayDraft }>();
  const ogHint = () => {
    const reading = dslReading();
    const hint = reading?.parsed.og_hint;
    return sourceKind() === "dsl" && reading?.source === source().trim() && hint?.date_windows ? hint : undefined;
  };
  // The workspace under an in-flight save is not frozen: the user can retype the
  // search, change the view, rename it, switch tab or graph. Every completion
  // has to get past these before it may touch routing, messages or `saving`.
  let alive = true;
  let saveToken = 0;
  // A changed-and-restored value is still a newer edit; the route object is
  // included so its non-presentation Display draft takes part in the revision.
  const inputRevision = createMemo((previous: number) => {
    props.route; source(); sourceKind(); presentation(); title();
    pageMatchScope(); pageMatchScopeExplicit();
    pagePresentation(); blockPresentation(); pageDisplay(); blockDisplay();
    return previous + 1;
  }, 0);
  onCleanup(() => { alive = false; });
  // Route objects are replaced on source and Display edits. Own cancellation
  // by the stable workspace identity so those edits cannot close its new run.
  const workspaceIdentity = createMemo(() => props.route.id);
  createEffect(on(workspaceIdentity, (workspace) => {
    const binding = captureBinding();
    const identity = bindingIdentity();
    const owner = ownedWhen(() => bindingIdentity() === identity);
    const close = deps().closeSearchWorkspace;
    onCleanup(() => {
      if (owner() && close) void close(workspace, binding.backendGeneration)
        .catch(() => { if (owner()) pushToast(msg("query-query-workspace-couldn-t-stop-this-workspace-s-search"), "warn"); });
    });
  }));
  let advancedButton!: HTMLButtonElement;
  const advancedLayerId = `query-advanced-${createUniqueId()}`;
  let sourceInput: HTMLInputElement | undefined;
  // Props replace the whole route object on source/presentation edits. Keep an
  // explicit identity latch so that replacement cannot re-run the focus work.
  let lastFocusRouteId: string | undefined;
  let previouslyFocused = false;

  createEffect(() => {
    props.route.id;
    setSource(props.route.source);
    setSourceKind(props.route.sourceKind);
    setPresentation(props.route.presentation);
    setPagePresentation(props.route.pagePresentation);
    setBlockPresentation(props.route.blockPresentation);
    setPageDisplay(props.route.pageDisplay);
    setBlockDisplay(props.route.blockDisplay);
    setPageMatchScope(props.route.pageMatchScope ?? "names");
    setPageMatchScopeExplicit(props.route.pageMatchScope !== undefined);
  });
  createEffect(() => {
    const routeId = props.route.id;
    const focusSource = !!props.focusSource;
    const shouldFocus = focusSource && (routeId !== lastFocusRouteId || !previouslyFocused);
    lastFocusRouteId = routeId;
    previouslyFocused = focusSource;
    if (!shouldFocus) return;
    queueMicrotask(() => {
      if (!props.router.route) return;
      const active = props.router.route();
      if (props.focusSource && active.kind === "query" && active.id === routeId) sourceInput?.focus();
    });
  });

  const pageView = createMemo(() => queryDisplaySettings(pageDisplay(), {}, pagePresentation() ?? presentation()));
  const blockView = createMemo(() => queryDisplaySettings(blockDisplay(), {}, blockPresentation() ?? presentation()));
  const executionScope = {};
  const [execution] = createMembershipResource(
    () => ({
      id: props.route.id,
      source: source().trim(),
      sourceKind: sourceKind(),
      explain: explain(),
      pageMatchScope: pageMatchScope(),
      pageView: pageView(),
      blockView: blockView(),
      transliteration: sourceKind() === "search" && pinyinMatchingEnabled(),
    }),
    () => JSON.stringify([props.route.id, source().trim(), sourceKind(), explain(), pageMatchScope(), pageView(), blockView(), sourceKind() === "search" && pinyinMatchingEnabled()]),
    async (request): Promise<QueryExecution> => {
      if (!request.source) {
        return { hits: [], diagnostics: [], explanation: { branches: [] }, cancelled: false };
      }
      const owner = latestOwner(executionScope, "run", graphOwner(() => alive && props.route.id === request.id));
      const cancelled: QueryExecution = { hits: [], diagnostics: [], explanation: { branches: [] }, cancelled: true };
      if (request.sourceKind === "search") {
        const result = await readOwned(owner, deps().runGraphSearch(
          request.source,
          PAGE_LIMIT,
          BLOCK_LIMIT,
          `query-workspace:${request.id}`,
          request.explain,
          undefined,
          request.pageMatchScope,
          { page: request.pageView, block: request.blockView }
        ));
        return result.kind === "current" && owner() ? result.value : cancelled;
      }
      const source = queryWorkspaceDsl(request.source);
      const parsed = await readOwned(owner, deps().parseQuery(source.argument, source.dialect));
      if (parsed.kind === "stale" || !owner()) return cancelled;
      setDslReading({ source: request.source, parsed: parsed.value });
      const result = await readOwned(owner, deps().queryRun(parsed.value.query, queryDisplaySettings(blockDisplay(), parsed.value.view, presentation())));
      if (result.kind === "stale" || !owner()) return cancelled;
      const explanation = request.explain && result.value.total === 0 && !(result.value.diagnostics ?? []).some((item) => !item.disabled)
        ? await readOwned(owner, deps().queryExplainEmpty(parsed.value.query, parsed.value.view))
        : undefined;
      if (explanation?.kind === "stale" || !owner()) return cancelled;
      return irToExecution(result.value, explanation?.value);
    },
    (value, id) => value.hits.some((hit) => hit.entity === "block" && hit.block.id === id),
    () => true,
    true,
  );

  // Failure is drawn from `execution.error` below; a read never throws into render.
  const executed = () => {
    const result = readOr(execution, undefined, "search");
    return pinyinMatchingEnabled() || !result ? result : {
      ...result, hits: result.hits.filter(hit => hit.match_class !== "transliteration"),
    };
  };
  const hits = () => executed()?.hits ?? [];
  const pageHits = () => hits().filter((hit): hit is QueryPageHit => hit.entity === "page");
  const blockHits = () => hits().filter((hit): hit is Extract<QueryHit, { entity: "block" }> => hit.entity === "block");
  const boardGroups = createMemo(() => {
    const grouped: [string, QueryHit[]][] = [];
    for (const hit of blockHits()) {
      const page = hitPage(hit);
      const last = grouped[grouped.length - 1];
      if (last && last[0] === page) last[1].push(hit);
      else grouped.push([page, [hit]]);
    }
    return grouped;
  });
  const [pageDisplayOpen, setPageDisplayOpen] = createSignal(false);
  const [blockDisplayOpen, setBlockDisplayOpen] = createSignal(false);
  const registry = createQueryRegistryAccess(() => pageDisplayOpen() || blockDisplayOpen());
  const pageColumns = createMemo(() => [...new Set(["kind", ...pageHits().flatMap(hit =>
    hit.row?.properties.map(([key]) => key) ?? [])])]);
  const blockColumns = createMemo(() => queryRecordColumns(blockHits().map(hit => ({
    id: hit.block.id, page: hit.page, kind: hit.kind, path: hit.path, dto: hit.block,
  }))));
  const sectionControl = (kind: "page" | "block") => <QueryDisplay
    rowKind={() => kind}
    defaultColumns={() => kind === "page" ? ["kind"] : []}
    availableColumns={kind === "page" ? pageColumns : blockColumns}
    registry={registry}
    view={kind === "page" ? pageView : blockView}
    onOpenChange={kind === "page" ? setPageDisplayOpen : setBlockDisplayOpen}
    apply={(next) => {
      const { view, ...draft } = next;
      if (kind === "page") {
        setPagePresentation(view ?? "list"); setPageDisplay(draft);
        props.router.updateActiveQuery({ pagePresentation: view ?? "list", pageDisplay: draft });
      } else {
        setBlockPresentation(view ?? "list"); setBlockDisplay(draft);
        props.router.updateActiveQuery({ blockPresentation: view ?? "list", blockDisplay: draft });
      }
    }}
  />;

  const updateSource = (next: string, kind = sourceKind()) => {
    setSource(next);
    setSourceKind(kind);
    props.router.updateActiveQuery({ source: next, sourceKind: kind });
  };
  const convertDsl = async (query: Query) => {
    const before = source();
    const reading = dslReading();
    if (!reading || reading.source !== before.trim()) return;
    const revision = inputRevision();
    const owner = graphOwner(() => alive && inputRevision() === revision);
    try {
      const printed = await readOwned(owner, backend().printQuery(query, reading.parsed.view, "tql_macro"));
      if (printed.kind === "stale" || !owner()) return;
      const converted = `{{${QUERY_MACRO_NAMES[1]} ${printed.value}}}`;
      setConversionUndo({ source: before, converted, pageDisplay: pageDisplay(), blockDisplay: blockDisplay() });
      const { view: _view, ...draft } = reading.parsed.view;
      const pages = { ...draft, ...pageDisplay() }, blocks = { ...draft, ...blockDisplay() };
      setPageDisplay(pages); setBlockDisplay(blocks);
      props.router.updateActiveQuery({ pageDisplay: pages, blockDisplay: blocks });
      updateSource(converted, "dsl");
    } catch (error) { if (owner()) setSaveError(error instanceof Error ? error.message : String(error)); }
  };
  const updatePresentation = (next: QueryPresentation) => {
    setPresentation(next);
    props.router.updateActiveQuery({ presentation: next });
  };
  const closeAdvanced = () => {
    setAdvancedOpen(false);
    queueMicrotask(() => advancedButton?.focus());
  };
  const hitRoute = (hit: QueryHit): Extract<Route, { kind: "page" }> => hit.entity === "page"
    ? { kind: "page", name: hit.page.name, pageKind: hit.page.kind, path: hit.page.path || undefined }
    : { kind: "page", name: hit.page, pageKind: hit.kind, path: hit.path || undefined, block: blockDtoExternalId(hit.block) };
  const openHit = (hit: QueryHit, event?: Pick<MouseEvent, "shiftKey" | "ctrlKey" | "metaKey" | "altKey">) => {
    const target = hitRoute(hit);
    const dest = event ? internalLinkDest(event) : "default";
    if (dest === "sidebar") {
      if (target.block) openBlockInSidebar({ uuid: target.block, page: target.name, pageKind: target.pageKind, path: target.path });
      else openPageInSidebar(target.name, target.pageKind, target.path);
    } else if (dest === "pane") openRouteInOtherPane(target);
    else props.router.openInNewTab(target, dest !== "background");
  };
  const [activeResult, setActiveResult] = createSignal<string | null>(null);
  // Navigation identity includes the physical owner: same-named page results
  // can point at different files, and Enter must open the actual focused row.
  const keyboardHitKey = (hit: QueryHit): string => JSON.stringify(hit.entity === "page"
    ? [hit.entity, hit.page.kind, hit.page.path || hit.page.name]
    : [hit.entity, hit.kind, hit.path || hit.page, hit.block.id]);
  let workspace!: HTMLElement;
  let focusedResult: HTMLElement | null = null;
  createEffect(() => {
    hits(); pageView(); blockView();
    const previous = focusedResult;
    // Result publication can remove a focused row. Once the DOM settles, keep
    // keyboard focus in this Search tab; never reclaim it from another control.
    queueMicrotask(() => {
      if (alive && previous && !previous.isConnected && workspace?.isConnected
        && previous.ownerDocument.activeElement === previous.ownerDocument.body) {
        setActiveResult(null);
        sourceInput?.focus();
      }
    });
  });
  // Roving focus keeps real buttons accessible in every presentation, including
  // tables and boards. Read displayed rows so section order stays authoritative.
  const resultKeyDown = (event: KeyboardEvent) => {
    if (event.isComposing || event.keyCode === 229) return;
    const target = event.target as HTMLElement;
    const fromInput = target === sourceInput;
    const key = target.dataset.queryResult;
    if (!fromInput && !key) return;
    const direction = event.key === "ArrowDown" || (event.ctrlKey && shortcutKey(event) === "n") ? 1
      : event.key === "ArrowUp" || (event.ctrlKey && shortcutKey(event) === "p") ? -1 : 0;
    if (direction) {
      const rows = [...workspace.querySelectorAll<HTMLButtonElement>("button[data-query-result]")];
      const index = steppedResultIndex(fromInput ? -1 : rows.indexOf(target as HTMLButtonElement), direction, rows.length);
      if (index === null) {
        if (!fromInput) { event.preventDefault(); event.stopPropagation(); }
        return;
      }
      event.preventDefault(); event.stopPropagation();
      if (index < 0) {
        setActiveResult(null);
        sourceInput?.focus();
        return;
      }
      rows[index].focus();
      rows[index].scrollIntoView({ block: "nearest" });
    } else if (event.key === "Enter" && key) {
      const hit = hits().find((hit) => keyboardHitKey(hit) === key);
      if (!hit) return;
      event.preventDefault(); event.stopPropagation();
      openHit(hit, event);
    }
  };
  const resultActions = (hit: QueryHit, keyboard = true): JSX.ButtonHTMLAttributes<HTMLButtonElement> & { "data-query-result"?: string } => ({
    ...(keyboard ? {
      "data-query-result": keyboardHitKey(hit),
      tabIndex: activeResult() === keyboardHitKey(hit) ? 0 : -1,
      classList: { active: activeResult() === keyboardHitKey(hit) },
      onFocus: () => setActiveResult(keyboardHitKey(hit)),
    } : {}),
    onMouseDown: internalLinkMouseDown,
    onClick: (event) => openHit(hit, event),
    onAuxClick: (event) => internalLinkAuxClick(event, () => props.router.openInNewTab(hitRoute(hit))),
    onContextMenu: (event) => {
      if (hit.entity !== "page" || !shouldOpenTextContextMenu(event)) return;
      event.preventDefault(); event.stopPropagation();
      const { kind: _kind, ...target } = hitRoute(hit);
      openPageContextMenu(event.clientX, event.clientY, target);
    },
  });
  const hitSurfaceId = (hit: QueryHit) =>
    `query:${props.route.id}:${hit.entity}:${hit.entity === "page" ? hit.page.name : hit.block.id}`;
  const task = (hit: Extract<QueryHit, { entity: "block" }>) => <SourceTaskCheckbox marker={hit.block.marker}
    target={() => ({ uuid: blockDtoExternalId(hit.block), page: hit.page, pageKind: hit.kind, path: hit.path })}
    result={{ id: hit.block.id, page: hit.page, kind: hit.kind, path: hit.path, dto: hit.block }} />;
  /** Everything one save attempt publishes, frozen at submit. */
  const captureSave = () => ({
    token: ++saveToken,
    inputRevision: inputRevision(),
    routeId: props.route.id,
    owner: graphOwner(),
    input: {
      title: title(), sourceKind: sourceKind(), source: source(), presentation: presentation(),
      pagePresentation: pagePresentation(), blockPresentation: blockPresentation(),
      pageDisplay: pageDisplay(), blockDisplay: blockDisplay(),
      pageMatchScope: pageMatchScopeExplicit() ? pageMatchScope() : undefined,
    },
  });
  type CapturedSave = ReturnType<typeof captureSave>;
  /** Same component, same graph binding (I-20), same ACTIVE route; only the
   *  newest attempt owns the shared `saving` flag. */
  const sameWorkspace = (captured: CapturedSave): boolean => {
    if (!alive || captured.token !== saveToken || !captured.owner()) return false;
    const active = props.router.route?.();
    if (active && (active.kind !== "query" || active.id !== captured.routeId)) return false;
    return props.route.id === captured.routeId;
  };
  /** …and does it still say what this attempt captured? A route id is not an
   *  input: an edit under the SAME id is a different publication. */
  const sameInput = (captured: CapturedSave): boolean =>
    sameWorkspace(captured) && inputRevision() === captured.inputRevision;
  const save = async (event: SubmitEvent) => {
    event.preventDefault();
    if (saving()) return;
    const captured = captureSave();
    setSaving(true);
    setSaveError(null);
    setSaveNotice(null);
    try {
      const result = await materializeQueryWorkspace({ ...captured.input, routeId: captured.routeId }, deps(), () => sameInput(captured));
      // Every branch below is about the LOCAL surface: a workspace that has moved
      // on gets nothing written into it.
      if (!sameWorkspace(captured)) return;
      if (!result.ok) {
        setSaveError(result.message);
        return;
      }
      if (!sameInput(captured)) {
        // The write had already begun when the input changed: the page is real
        // and is not undone, but it is no longer what this workspace shows.
        setSaveNotice(t("query-save-earlier-search", { name: result.name }));
        return;
      }
      props.router.replaceActiveRoute({ kind: "page", name: result.name, pageKind: "page" });
    } finally {
      // A superseded attempt may not re-enable a button a newer one is using.
      if (alive && captured.token === saveToken) setSaving(false);
    }
  };

  const resultButton = (hit: QueryHit, body: JSX.Element) => (
    <button
      type="button"
      class="query-result-row switcher-row"
      data-inpage-find-surface={hitSurfaceId(hit)}
      {...resultActions(hit)}
    >
      {body}
    </button>
  );

  return (
    <section ref={workspace} onKeyDown={resultKeyDown} onFocusIn={(event) => {
      const target = event.target as HTMLElement;
      focusedResult = target.dataset.queryResult ? target : null;
    }} class="query-workspace" data-query-route-id={props.route.id} aria-label={t("query-query-workspace-search-and-query-workspace")}>
      <header class="query-workspace-header">
        <div class="query-workspace-search-row">
          <label class="query-workspace-source-label">
            <span class="sr-only">{sourceKind() === "search" ? t("query-query-workspace-search") : t("query-query-workspace-query-dsl")}</span>
            <input
              ref={sourceInput}
              class="query-workspace-source"
              type="search"
              value={source()}
              onInput={(event) => updateSource(event.currentTarget.value)}
              placeholder={sourceKind() === "search" ? t("query-query-workspace-search-pages-and-blocks") : t("query-query-workspace-query-dsl")}
              aria-describedby="query-workspace-summary"
              spellcheck={false}
            />
          </label>
          <button
            ref={advancedButton}
            type="button"
            class="query-advanced-toggle"
            aria-haspopup="dialog"
            aria-expanded={advancedOpen()}
            onClick={() => setAdvancedOpen(true)}
          >
            {t("query-query-workspace-filters-advanced")}</button>
        </div>
        <p id="query-workspace-summary" class="query-workspace-summary">
          {sourceKind() === "search" ? friendlySummary(source()) : t("query-query-workspace-runs-the-saved-query-expression-against-blocks-its-presentation-is")}
        </p>

        <div class="query-workspace-controls">
          <Show when={sourceKind() === "search"}>
            <label class="query-page-match-scope">{t("query-query-workspace-pages-match")}<select aria-label={t("query-query-workspace-pages-match")} value={pageMatchScope()} onChange={(event) => {
                const next = event.currentTarget.value as FriendlyPageMatchScope;
                setPageMatchScope(next);
                setPageMatchScopeExplicit(true);
                props.router.updateActiveQuery({ pageMatchScope: next });
              }}>
                <option value="names">{t("query-query-workspace-names-and-aliases")}</option>
                <option value="content">{t("query-query-workspace-block-content")}</option>
                <option value="both">{t("query-query-workspace-names-or-content")}</option>
              </select>
            </label>
          </Show>
          <div class="query-presentations" role="group" aria-label={t("query-query-workspace-result-presentation")}>
            <For each={VIEW_KINDS}>
              {(view) => (
                <button
                  type="button"
                  classList={{ active: presentation() === view }}
                  aria-pressed={presentation() === view}
                  onClick={() => updatePresentation(view)}
                >
                  {t(`query-workspace-view-${view}`)}
                </button>
              )}
            </For>
          </div>
          <button
            type="button"
            class="query-explain-toggle"
            aria-pressed={explain()}
            onClick={() => setExplain((value) => !value)}
          >
            {explain() ? t("query-query-workspace-hide-explanation") : t("query-query-workspace-explain-query")}
          </button>
        </div>

        <form class="query-workspace-save" onSubmit={save}>
          <label>
            <span class="sr-only">{t("query-query-workspace-page-title")}</span>
            <input
              value={title()}
              onInput={(event) => { setTitle(event.currentTarget.value); setSaveError(null); setSaveNotice(null); }}
              placeholder={t("query-query-workspace-name-this-search-to-save-it-as-a-page")}
              aria-invalid={!!saveError()}
            />
          </label>
          <button type="submit" disabled={saving()}>{saving() ? t("query-query-workspace-saving") : t("query-query-workspace-save-page")}</button>
        </form>
        <Show when={saveError()}>
          <p class="query-workspace-save-error" role="alert">{saveError()}</p>
        </Show>
        <Show when={saveNotice()}>
          <p class="query-workspace-save-notice" role="status">{saveNotice()}</p>
        </Show>
      </header>

      <section class="query-workspace-status" aria-live="polite" aria-atomic="true">
        <Show when={!source().trim()}>{sourceKind() === "search" ? t("query-query-workspace-enter-a-search-to-begin") : t("query-query-workspace-enter-a-query-to-begin")}</Show>
        <Show when={!!source().trim() && execution.loading}>{t("query-query-workspace-searching")}</Show>
        <Show when={!!source().trim() && !execution.loading && execution.error}>
          {t("query-workspace-search-failed", { detail: execution.error instanceof Error ? execution.error.message : String(execution.error) })}
        </Show>
        <Show when={!!source().trim() && !execution.loading && !execution.error && executed()?.cancelled}>
          {t("query-query-workspace-search-superseded-by-a-newer-request")}</Show>
        <Show when={!!source().trim() && !execution.loading && !execution.error && executed() && !executed()?.cancelled && !ogHint()?.date_windows}>
          {t("query-workspace-result-count", { count: hits().length })}
        </Show>
      </section>

      <QueryResultsSelection hits={hits()} />

      <Show when={ogHint()}>{(hint) => <QueryOgHint hint={hint()} onRewrite={(query) => { void convertDsl(query); }} />}</Show>
      <Show when={conversionUndo()?.converted === source()}>
        <button type="button" class="query-conversion-undo" onClick={() => {
          const prior = conversionUndo(); if (!prior) return;
          setPageDisplay(prior.pageDisplay); setBlockDisplay(prior.blockDisplay);
          props.router.updateActiveQuery({ pageDisplay: prior.pageDisplay, blockDisplay: prior.blockDisplay });
          updateSource(prior.source, "dsl"); setConversionUndo(undefined);
        }}>{t("query-undo-conversion")}</button>
      </Show>

      <Show when={(executed()?.diagnostics.length ?? 0) > 0 && (!ogHint()?.date_windows || !ogHint()?.rewrites.length)}>
        <ul class="query-workspace-diagnostics" aria-label={t("query-query-workspace-query-diagnostics")}>
          <For each={executed()?.diagnostics ?? []}>{(diagnostic) => (
            <li role="alert" data-code={diagnostic.code}>{diagnostic.message}</li>
          )}</For>
        </ul>
      </Show>

      <Show when={explain() && (executed()?.explanation.branches.length ?? 0) > 0}>
        <section class="query-workspace-explanation" aria-label={t("query-query-workspace-query-explanation")}>
          <h2>{t("query-query-workspace-how-this-query-works")}</h2>
          <ExplainTree nodes={executed()?.explanation.branches ?? []} />
        </section>
      </Show>

      <Show when={source().trim() && !ogHint()?.date_windows}>
      <QueryResultSections
        pending={execution.loading}
        failure={execution.error ? `Search failed: ${execution.error instanceof Error ? execution.error.message : String(execution.error)}` : null}
        families={[
          { kind: "page", hits: pageHits().length, hasMore: !!executed()?.has_more?.pages,
            control: sectionControl("page"),
            body: <QueryPageResults hits={pageHits()} presentation={pageView().view ?? "list"} view={pageView()} surfaceId={hitSurfaceId} onOpen={openHit} actions={resultActions} /> },
          { kind: "block", hits: blockHits().length, hasMore: !!executed()?.has_more?.blocks,
            control: sectionControl("block"),
            body: <Switch>
        <Match when={blockView().view === "search"}>
          <div class="query-results-search" role="list" aria-label={t("query-query-workspace-block-results")}>
            <For each={blockHits()}>{(hit) => (
              <div role="listitem">
                {task(hit)}
                {resultButton(hit, <SearchResultRow
                    page={hit.page}
                    breadcrumb={hit.block.breadcrumb ?? []}
                    text={hit.display_text}
                    spans={hitSpans(hit)}
                  />)}
              </div>
            )}</For>
          </div>
        </Match>

        <Match when={blockView().view === "list"}>
          <ul class="query-results-list" aria-label={t("query-query-workspace-query-results")}>
            <For each={blockHits()}>{(hit) => (
              <li>
                {task(hit)}
                <button type="button" data-inpage-find-surface={hitSurfaceId(hit)} {...resultActions(hit)}>
                  <span class="query-list-context">{hitPage(hit)}</span>
                  <span class="query-list-text"><MarkedText text={hit.display_text} spans={hitSpans(hit)} /></span>
                </button>
              </li>
            )}</For>
          </ul>
        </Match>

        <Match when={blockView().view === "table"}>
          <div class="query-results-table-wrap">
            <table class="query-results-table">
              <caption class="sr-only">{t("query-query-workspace-query-results")}</caption>
              <thead><tr><th scope="col">{t("query-query-workspace-type")}</th><th scope="col">{t("query-query-workspace-page")}</th><th scope="col">{t("query-query-workspace-content")}</th></tr></thead>
              <tbody>
                <For each={blockHits()}>{(hit) => (
                  <tr data-inpage-find-surface={hitSurfaceId(hit)}>
                    <td>{hitKind(hit)}</td>
                    <td><button type="button" {...resultActions(hit, false)}>{hitPage(hit)}</button></td>
                    <td>{task(hit)}<button type="button" {...resultActions(hit)}><MarkedText text={hit.display_text} spans={hitSpans(hit)} /></button></td>
                  </tr>
                )}</For>
              </tbody>
            </table>
          </div>
        </Match>

        <Match when={blockView().view === "board"}>
          <div class="query-results-board" aria-label={t("query-query-workspace-query-results-grouped-by-page")}>
            <For each={boardGroups()}>{([page, pageHits]) => (
              <section class="query-board-column">
                <h2>{page}<span class="query-board-count">{pageHits.length}</span></h2>
                <div role="list">
                  <For each={pageHits}>{(hit) => (
                    <div role="listitem">{hit.entity === "block" && task(hit)}<button type="button" class="query-board-card" data-inpage-find-surface={hitSurfaceId(hit)} {...resultActions(hit)}>
                      <span class="sr-only">{hitKind(hit)}: </span><MarkedText text={hit.display_text} spans={hitSpans(hit)} />
                    </button></div>
                  )}</For>
                </div>
              </section>
            )}</For>
          </div>
        </Match>
      </Switch> },
        ]}
      />
      </Show>

      <Show when={advancedOpen()}>
        <AdvancedModal
          source={source}
          sourceKind={sourceKind}
          onApply={(next, kind) => { updateSource(next, kind); closeAdvanced(); }}
          onClose={closeAdvanced}
          layerId={advancedLayerId}
          trigger={() => advancedButton ?? null}
        />
      </Show>
    </section>
  );
}
