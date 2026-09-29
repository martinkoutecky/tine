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
  type JSX,
} from "solid-js";
import { backend, type SavePageEntry, type SavePagesResult } from "../backend";
import { captureBinding } from "../binding";
import { graphOwner, readOwned, writeOwned } from "../owned";
import { pushToast } from "../toasts";
import { errorFamily } from "../errorFamily";
import {
  friendlySearchToDsl,
  friendlySearchToSavedDsl,
  parseSearchQuery,
} from "../editor/searchQuery";
import type { PaneRouter, QueryPresentation, QueryRoute } from "../router";
import type {
  MatchSpan,
  PageDto,
  ResolvedPage,
  QueryDiagnostic,
  QueryExecution,
  QueryExplainNode,
  QueryHit,
} from "../types";
import type { ParsedQuery, Query, QueryResult, ViewSettings, ExplainEmptyResult } from "../editor/queryIr";
import { QueryBuilder, type BuilderSession } from "./QueryBuilder";
import { QUERY_MACRO_NAMES } from "../editor/queryMacroName";
import { SearchResultRow } from "./SearchResultRow";
import { QueryPageResults, type QueryPageHit } from "./QueryPageResults";
import { QueryResultSections } from "./QueryResultSections";
import { registerTransientLayer } from "../transientLayers";
import { bumpPageInventoryRev } from "../graphSession";
import { blockDtoExternalId } from "../blockIdentity";
import { createPage, CreatePageRefusal, queryWorkspacePage } from "../document";

const PAGE_LIMIT = 40;
const BLOCK_LIMIT = 100;

export interface MaterializeQueryInput {
  title: string;
  sourceKind: QueryRoute["sourceKind"];
  source: string;
  presentation: QueryPresentation;
  /** Stable workspace identity: also bounds the native validation cancellation lane. */
  routeId: string;
}

export interface MaterializeQueryDependencies {
  /** The one name answerer: an existing file, an alias, or where a new page goes. */
  resolvePage(name: string, kind: "page"): Promise<ResolvedPage>;
  savePages(entries: SavePageEntry[], bindingGeneration?: number): Promise<SavePagesResult>;
  /** Rust-authoritative friendly-search validation; required before every nonblank friendly save. */
  runGraphSearch(source: string, pageLimit: number, blockLimit: number, lane: string, explain: boolean): Promise<QueryExecution>;
}

export type MaterializeQueryResult =
  | { ok: true; name: string; page: PageDto; rev: string }
  | {
      ok: false;
      kind: "invalid-name" | "empty-query" | "invalid-query" | "exists" | "conflict" | "error";
      message: string;
    };

export interface QueryWorkspaceDependencies extends MaterializeQueryDependencies {
  parseQuery(source: string, dialect: "macro_query"): Promise<ParsedQuery>;
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

function savedQueryRaw(input: Pick<MaterializeQueryInput, "source" | "sourceKind" | "presentation">): string {
  const source = input.source.trim();
  const dsl = input.sourceKind === "search" ? friendlySearchToSavedDsl(source) : source;
  const query = `{{${QUERY_MACRO_NAMES[0]} ${dsl}}}`;
  return input.presentation === "list" ? query : `${query}\ntine.view:: ${input.presentation}`;
}

/**
 * Materialize a virtual workspace as exactly one ordinary query block.
 *
 * The preflight existence check provides a friendly error. The authoritative
 * race guard is the audited no-baseline save (`null`, never force): if another
 * writer creates the page between the two calls, the backend rejects it as a
 * conflict and this workspace remains virtual.
 */
export async function materializeQueryWorkspace(
  input: MaterializeQueryInput,
  deps: MaterializeQueryDependencies
): Promise<MaterializeQueryResult> {
  const binding = captureBinding();
  const owner = graphOwner();
  const name = input.title.trim();
  if (!name) {
    return { ok: false, kind: "invalid-name", message: "Enter a page title before saving." };
  }
  if (!input.source.trim()) {
    return { ok: false, kind: "empty-query", message: "Enter a search or query before saving." };
  }
  if (input.sourceKind === "search") {
    try {
      const search = await readOwned(owner, deps.runGraphSearch(input.source.trim(), 0, 0, `query-workspace:${input.routeId}:materialize`, true));
      if (search.kind === "stale") return { ok: false, kind: "error", message: "The graph changed before this workspace could be saved." };
      const execution = search.value;
      if (execution.cancelled) return { ok: false, kind: "invalid-query", message: "Search validation was superseded. Try saving again." };
      if (execution.diagnostics.length) return { ok: false, kind: "invalid-query", message: execution.diagnostics.map((item) => item.message).join(" · ") };
      if (!execution.explanation.branches.length) return { ok: false, kind: "empty-query", message: "Enter a search with at least one included term before saving." };
    } catch (error) {
      const detail = error instanceof Error ? error.message : String(error);
      return { ok: false, kind: "invalid-query", message: detail ? `Could not validate this search: ${detail}` : "Could not validate this search." };
    }
  }

  try {
    const resolution = await readOwned(owner, deps.resolvePage(name, "page"));
    if (resolution.kind === "stale") return { ok: false, kind: "error", message: "The graph changed before this workspace could be saved." };
    const resolved = resolution.value;
    if (resolved.kind === "existing") {
      return {
        ok: false,
        kind: "exists",
        message: `A page named “${name}” already exists. Choose another title.`,
      };
    }
    // An alias names another page: refuse, write nothing, keep the query here.
    if (resolved.kind === "alias") {
      return {
        ok: false,
        kind: "exists",
        message: `“${name}” is an alias of an existing page. Choose another title.`,
      };
    }

    const page = queryWorkspacePage(name, savedQueryRaw(input));
    const saved = await writeOwned(owner, deps.savePages([{ id: resolved.id, page, baseRev: null, force: false, kinds: ["create-page"] }], binding.backendGeneration)
      .then((result) => {
        if ("failed" in result) {
          const refusal = result.failed as { family: string };
          throw Object.assign(new Error(refusal.family), { family: refusal.family });
        }
        return result as { ok: string[] };
      }));
    if (saved.kind === "stale") return { ok: false, kind: "error", message: "The graph changed before this workspace could be saved." };
    const result = saved.value;
    const rev = result.ok[0];
    if (!owner()) return { ok: false, kind: "error", message: "The graph changed before this workspace could be saved." };
    bumpPageInventoryRev();
    return { ok: true, name, page, rev };
  } catch (error) {
    const detail = error instanceof Error ? error.message : String(error);
    if (!owner()) {
      pushToast(`Could not save “${name}”: ${detail}`, "error");
      return { ok: false, kind: "error", message: "The graph changed before this workspace could be saved." };
    }
    if (error instanceof CreatePageRefusal) {
      if (error.reason === "graph-changed" || error.reason === "stale-binding")
        return { ok: false, kind: "error", message: "The graph changed before this workspace could be saved." };
      if (error.reason === "alias")
        return { ok: false, kind: "exists", message: `“${name}” is an alias of an existing page. Choose another title.` };
      return { ok: false, kind: "error", message: `Could not save “${name}” because its local page state changed. Try again.` };
    }
    if (errorFamily(error) === "conflict") {
      return {
        ok: false,
        kind: "conflict",
        message: `“${name}” was created or changed before this workspace could be saved. It has not been overwritten.`,
      };
    }
    return {
      ok: false,
      kind: "error",
      message: detail ? `Could not save “${name}”: ${detail}` : `Could not save “${name}”.`,
    };
  }
}

function defaultDependencies(): QueryWorkspaceDependencies {
  const api = backend();
  return {
    resolvePage: (name, kind) => api.resolvePage(name, kind),
    savePages: async (entries, bindingGeneration) => {
      const entry = entries[0];
      return { ok: [await createPage(entry.page.name, entry.page, { id: entry.id, baseRev: entry.baseRev, bindingGeneration })] };
    },
    runGraphSearch: (source, pageLimit, blockLimit, lane, explain) =>
      api.runGraphSearch(source, pageLimit, blockLimit, lane, explain),
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
    code: "unsupported_clause", message: `This query clause is not supported yet: ${clause}`,
  });
  return {
    hits,
    diagnostics,
    cancelled: false,
    explanation: {
      branches: explanation?.rows.map((row) => ({
        description: `${row.conjunct}: ${row.alone} alone${row.without === null || row.without === undefined ? "" : `, ${row.without} without it`}`,
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

function hitKind(hit: QueryHit): "Page" | "Block" {
  return hit.entity === "page" ? "Page" : "Block";
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
  if (parsed.kind === "empty") return "Type to search page names and block text.";
  if (parsed.kind === "invalid") return `The regular expression is invalid: ${parsed.error}`;
  if (parsed.kind === "regex") return `Matches page names or block text using the case-sensitive regular expression /${parsed.re.source}/.`;
  const describeGroup = (group: typeof parsed.groups[number]) => group.map((term) => {
    const value = term.quoted ? `the exact phrase “${term.text}”` : `“${term.text}”`;
    return term.negated ? `excluding ${value}` : `containing ${value}`;
  }).join(" and ");
  const groups = parsed.groups.map(describeGroup);
  return groups.length === 1
    ? `Matches page names or block text ${groups[0]}.`
    : `Matches page names or block text when it is ${groups.join("; or ")}.`;
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
      return { source: "", error: "A regular expression cannot be combined with the other friendly fields yet." };
    }
    try {
      new RegExp(regex);
      return { source: `/${regex}/`, error: null };
    } catch (error) {
      return { source: "", error: error instanceof Error ? error.message : "Invalid regular expression." };
    }
  }

  if ([fields.all, fields.any, fields.exact, fields.exclude].some((value) => value.includes('"'))) {
    return { source: "", error: "Quotation marks are not supported inside these fields." };
  }
  const common = [
    ...filterWords(fields.all),
    ...(fields.exact.trim() ? [`"${fields.exact.trim()}"`] : []),
    ...filterWords(fields.exclude).map((term) => `-${term}`),
  ];
  const alternatives = filterWords(fields.any);
  if (!common.some((term) => !term.startsWith("-")) && !alternatives.length) {
    return { source: "", error: "Add at least one word or exact phrase to include." };
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
  if (parsed.kind === "regex") return { ...empty, regex: parsed.re.source };
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
  // The builder edits the ENGINE's reading of the draft and writes back the OG
  // text the engine printed (I-12); a workspace materializes an OG `{{query}}`.
  const [builderSession] = createResource(dsl, async (text): Promise<BuilderSession | undefined> => {
    const parsed = await readOwned(graphOwner(() => dsl() === text), backend().parseQuery(text, "og"));
    return parsed.kind === "current" ? { query: parsed.value.query, view: parsed.value.view } : undefined;
  });
  const applyBuilderEdit = async (next: BuilderSession) => {
    try {
      const printed = await readOwned(graphOwner(), backend().printQuery(next.query, next.view, "og"));
      if (printed.kind === "stale") return;
      setDsl(printed.value);
      setError(null);
    } catch (failure) {
      // An edit the OG syntax cannot say has nowhere to go here; the printer's
      // own message says which part (I-9), and the draft is left as it was.
      setError(failure instanceof Error ? failure.message : String(failure));
    }
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
      if (!dsl().trim()) {
        setError("The query DSL cannot be empty.");
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
            if (event.shiftKey && document.activeElement === first) {
              event.preventDefault();
              last.focus();
            } else if (!event.shiftKey && document.activeElement === last) {
              event.preventDefault();
              first.focus();
            }
          }
        }}
      >
        <header class="query-advanced-header">
          <div>
            <h2 id="query-advanced-title">Filters and advanced query</h2>
            <p>Use the friendly fields, or switch losslessly to the visual query builder.</p>
          </div>
          <button type="button" aria-label="Close filters" onClick={props.onClose}>×</button>
        </header>

        <Show when={draftKind() === "search"} fallback={
          <div class="query-dsl-editor">
            <QueryBuilder
              session={() => (builderSession.error === undefined ? builderSession.latest : undefined)}
              onChange={(next) => void applyBuilderEdit(next)}
              paneDialect="og"
              sheetAlwaysOpen
              parentTransientId={props.layerId}
            />
            <p class="query-advanced-note">Switching back to friendly fields is offered only when it can be lossless.</p>
          </div>
        }>
          <Show when={structuredFriendly()} fallback={
            <div class="query-friendly-raw">
              <p>This search uses a combination that cannot be split into fields without changing it.</p>
              <label>
                Friendly search syntax
                <textarea
                  ref={(element) => { firstField = element; }}
                  rows={4}
                  value={rawFriendly()}
                  onInput={(event) => { setRawFriendly(event.currentTarget.value); setFriendlyDirty(true); setError(null); }}
                  spellcheck={false}
                />
              </label>
              <button type="button" class="query-switch-to-dsl" onClick={switchToDsl}>
                Edit as visual query
              </button>
            </div>
          }>
          <div class="query-friendly-fields">
            <label>
              All of these words
              <input ref={(element) => { firstField = element; }} value={all()} onInput={(event) => { setAll(event.currentTarget.value); setFriendlyDirty(true); }} />
            </label>
            <label>
              Any of these words
              <input value={any()} onInput={(event) => { setAny(event.currentTarget.value); setFriendlyDirty(true); }} />
            </label>
            <label>
              This exact phrase
              <input value={exact()} onInput={(event) => { setExact(event.currentTarget.value); setFriendlyDirty(true); }} />
            </label>
            <label>
              Exclude these words
              <input value={exclude()} onInput={(event) => { setExclude(event.currentTarget.value); setFriendlyDirty(true); }} />
            </label>
            <label>
              Case-sensitive regular expression
              <input value={regex()} onInput={(event) => { setRegex(event.currentTarget.value); setFriendlyDirty(true); }} placeholder="pattern without / /" />
            </label>
            <button type="button" class="query-switch-to-dsl" onClick={switchToDsl}>
              Edit as visual query
            </button>
          </div>
          </Show>
        </Show>

        <Show when={error()}>
          <p class="query-advanced-error" role="alert">{error()}</p>
        </Show>
        <footer class="query-advanced-actions">
          <button type="button" onClick={props.onClose}>Cancel</button>
          <button type="button" class="primary" onClick={apply}>Apply</button>
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
  const [explain, setExplain] = createSignal(false);
  const [advancedOpen, setAdvancedOpen] = createSignal(false);
  const [title, setTitle] = createSignal("");
  const [saveError, setSaveError] = createSignal<string | null>(null);
  const [saving, setSaving] = createSignal(false);
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

  const [execution] = createResource(
    () => ({
      id: props.route.id,
      source: source().trim(),
      sourceKind: sourceKind(),
      explain: explain(),
    }),
    async (request): Promise<QueryExecution> => {
      if (!request.source) {
        return { hits: [], diagnostics: [], explanation: { branches: [] }, cancelled: false };
      }
      if (request.sourceKind === "search") {
        return deps().runGraphSearch(
          request.source,
          PAGE_LIMIT,
          BLOCK_LIMIT,
          `query-workspace:${request.id}`,
          request.explain
        );
      }
      const parsed = await deps().parseQuery(request.source, "macro_query");
      const result = await deps().queryRun(parsed.query, parsed.view);
      const explanation = request.explain && result.total === 0 && !(result.diagnostics ?? []).some((item) => !item.disabled)
        ? await deps().queryExplainEmpty(parsed.query, parsed.view)
        : undefined;
      return irToExecution(result, explanation);
    }
  );

  const hits = () => execution()?.hits ?? [];
  const pageHits = () => hits().filter((hit): hit is QueryPageHit => hit.entity === "page");
  const blockHits = () => hits().filter((hit): hit is Extract<QueryHit, { entity: "block" }> => hit.entity === "block");
  const boardGroups = createMemo(() => {
    const grouped = new Map<string, QueryHit[]>();
    for (const hit of blockHits()) {
      const page = hitPage(hit);
      const group = grouped.get(page);
      if (group) group.push(hit);
      else grouped.set(page, [hit]);
    }
    return [...grouped.entries()];
  });

  const updateSource = (next: string, kind = sourceKind()) => {
    setSource(next);
    setSourceKind(kind);
    props.router.updateActiveQuery({ source: next, sourceKind: kind });
  };
  const updatePresentation = (next: QueryPresentation) => {
    setPresentation(next);
    props.router.updateActiveQuery({ presentation: next });
  };
  const closeAdvanced = () => {
    setAdvancedOpen(false);
    queueMicrotask(() => advancedButton?.focus());
  };
  const openHit = (hit: QueryHit) => {
    if (hit.entity === "page") {
      props.router.openPageTarget({
        name: hit.page.name,
        pageKind: hit.page.kind,
        ...(hit.page.path ? { path: hit.page.path } : {}),
      });
    } else {
      props.router.openPageAtBlock({
        name: hit.page,
        pageKind: hit.kind,
        block: blockDtoExternalId(hit.block),
        ...(hit.path ? { path: hit.path } : {}),
      });
    }
  };
  const hitSurfaceId = (hit: QueryHit) =>
    `query:${props.route.id}:${hit.entity}:${hit.entity === "page" ? hit.page.name : hit.block.id}`;
  const save = async (event: SubmitEvent) => {
    event.preventDefault();
    if (saving()) return;
    setSaving(true);
    setSaveError(null);
    try {
      const result = await materializeQueryWorkspace({
        title: title(),
        sourceKind: sourceKind(),
        source: source(),
        presentation: presentation(),
        routeId: props.route.id,
      }, deps());
      if (!result.ok) {
        setSaveError(result.message);
        return;
      }
      props.router.replaceActiveRoute({ kind: "page", name: result.name, pageKind: "page" });
    } finally {
      setSaving(false);
    }
  };

  const resultButton = (hit: QueryHit, body: JSX.Element) => (
    <button
      type="button"
      class="query-result-row switcher-row"
      data-inpage-find-surface={hitSurfaceId(hit)}
      onClick={() => openHit(hit)}
    >
      {body}
    </button>
  );

  return (
    <section class="query-workspace" data-query-route-id={props.route.id} aria-label="Search and query workspace">
      <header class="query-workspace-header">
        <div class="query-workspace-search-row">
          <label class="query-workspace-source-label">
            <span class="sr-only">{sourceKind() === "search" ? "Search" : "Query DSL"}</span>
            <input
              ref={sourceInput}
              class="query-workspace-source"
              type="search"
              value={source()}
              onInput={(event) => updateSource(event.currentTarget.value)}
              placeholder={sourceKind() === "search" ? "Search pages and blocks" : "Query DSL"}
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
            Filters / Advanced
          </button>
        </div>
        <p id="query-workspace-summary" class="query-workspace-summary">
          {sourceKind() === "search"
            ? friendlySummary(source())
            : "Runs the saved query expression against blocks. Its presentation is controlled separately."}
        </p>

        <div class="query-workspace-controls">
          <div class="query-presentations" role="group" aria-label="Result presentation">
            <For each={["search", "list", "table", "board"] as QueryPresentation[]}>
              {(view) => (
                <button
                  type="button"
                  classList={{ active: presentation() === view }}
                  aria-pressed={presentation() === view}
                  onClick={() => updatePresentation(view)}
                >
                  {view[0].toUpperCase() + view.slice(1)}
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
            {explain() ? "Hide explanation" : "Explain query"}
          </button>
        </div>

        <form class="query-workspace-save" onSubmit={save}>
          <label>
            <span class="sr-only">Page title</span>
            <input
              value={title()}
              onInput={(event) => { setTitle(event.currentTarget.value); setSaveError(null); }}
              placeholder="Name this search to save it as a page"
              aria-invalid={!!saveError()}
            />
          </label>
          <button type="submit" disabled={saving()}>{saving() ? "Saving…" : "Save page"}</button>
        </form>
        <Show when={saveError()}>
          <p class="query-workspace-save-error" role="alert">{saveError()}</p>
        </Show>
      </header>

      <section class="query-workspace-status" aria-live="polite" aria-atomic="true">
        <Show when={!source().trim()}>{sourceKind() === "search" ? "Enter a search to begin." : "Enter a query to begin."}</Show>
        <Show when={!!source().trim() && execution.loading}>Searching…</Show>
        <Show when={!!source().trim() && !execution.loading && execution.error}>
          Search failed: {execution.error instanceof Error ? execution.error.message : String(execution.error)}
        </Show>
        <Show when={!!source().trim() && !execution.loading && !execution.error && execution()?.cancelled}>
          Search superseded by a newer request.
        </Show>
        <Show when={!!source().trim() && !execution.loading && !execution.error && execution() && !execution()?.cancelled}>
          {hits().length} result{hits().length === 1 ? "" : "s"}
        </Show>
      </section>

      <Show when={(execution()?.diagnostics.length ?? 0) > 0}>
        <ul class="query-workspace-diagnostics" aria-label="Query diagnostics">
          <For each={execution()?.diagnostics ?? []}>{(diagnostic) => (
            <li role="alert" data-code={diagnostic.code}>{diagnostic.message}</li>
          )}</For>
        </ul>
      </Show>

      <Show when={explain() && (execution()?.explanation.branches.length ?? 0) > 0}>
        <section class="query-workspace-explanation" aria-label="Query explanation">
          <h2>How this query works</h2>
          <ExplainTree nodes={execution()?.explanation.branches ?? []} />
        </section>
      </Show>

      <Show when={source().trim()}>
      <QueryResultSections
        pending={execution.loading}
        failure={execution.error ? `Search failed: ${execution.error instanceof Error ? execution.error.message : String(execution.error)}` : null}
        families={[
          { kind: "page", hits: pageHits().length, hasMore: !!execution()?.has_more?.pages,
            body: <QueryPageResults hits={pageHits()} presentation={presentation()} surfaceId={hitSurfaceId} onOpen={openHit} /> },
          { kind: "block", hits: blockHits().length, hasMore: !!execution()?.has_more?.blocks,
            body: <Switch>
        <Match when={presentation() === "search"}>
          <div class="query-results-search" role="list" aria-label="Block results">
            <For each={blockHits()}>{(hit) => (
              <div role="listitem">
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

        <Match when={presentation() === "list"}>
          <ul class="query-results-list" aria-label="Query results">
            <For each={blockHits()}>{(hit) => (
              <li>
                <button type="button" data-inpage-find-surface={hitSurfaceId(hit)} onClick={() => openHit(hit)}>
                  <span class="query-list-context">{hitPage(hit)}</span>
                  <span class="query-list-text"><MarkedText text={hit.display_text} spans={hitSpans(hit)} /></span>
                </button>
              </li>
            )}</For>
          </ul>
        </Match>

        <Match when={presentation() === "table"}>
          <div class="query-results-table-wrap">
            <table class="query-results-table">
              <caption class="sr-only">Query results</caption>
              <thead><tr><th scope="col">Type</th><th scope="col">Page</th><th scope="col">Content</th></tr></thead>
              <tbody>
                <For each={blockHits()}>{(hit) => (
                  <tr data-inpage-find-surface={hitSurfaceId(hit)}>
                    <td>{hitKind(hit)}</td>
                    <td><button type="button" onClick={() => openHit(hit)}>{hitPage(hit)}</button></td>
                    <td><MarkedText text={hit.display_text} spans={hitSpans(hit)} /></td>
                  </tr>
                )}</For>
              </tbody>
            </table>
          </div>
        </Match>

        <Match when={presentation() === "board"}>
          <div class="query-results-board" aria-label="Query results grouped by page">
            <For each={boardGroups()}>{([page, pageHits]) => (
              <section class="query-board-column">
                <h2>{page}<span class="query-board-count">{pageHits.length}</span></h2>
                <div role="list">
                  <For each={pageHits}>{(hit) => (
                    <button type="button" role="listitem" class="query-board-card" data-inpage-find-surface={hitSurfaceId(hit)} onClick={() => openHit(hit)}>
                      <span class="sr-only">{hitKind(hit)}: </span><MarkedText text={hit.display_text} spans={hitSpans(hit)} />
                    </button>
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
