import {
  For,
  Show,
  createEffect,
  createMemo,
  createResource,
  createSignal,
  createUniqueId,
  onCleanup,
  onMount,
  untrack,
  type JSX,
} from "solid-js";
import { FloatingPortal } from "./FloatingPortal";
import { placeSheetTop } from "./popoverFit";
import { backend } from "../backend";
import {
  builderRoot,
  filterLabel,
  removeAt,
} from "../editor/queryBuilder";
import type {
  Anchor,
  Diagnostic,
  Filter,
  ParsedQuery,
  Query,
  QueryPrintDialect,
  RegistrySnapshot,
  RegistryRow,
  Span,
  ViewSettings,
} from "../editor/queryIr";
import {
  QuerySentence,
  QuerySheet,
  countConditions,
  rawLeaves,
  stop,
  type AnchorPrompt,
  type RegistryAccess,
} from "./QuerySheet";
import { sharedQueryResult } from "../queryResultCache";
import { graphOwner, ownedWhen, readOwned } from "../owned";
import { dataRev, graphEpoch, graphMeta } from "../graphSession";
import { queryBuilderAutoOpen, setQueryBuilderAutoOpen } from "../ui";
import { queryTextOpen, setQueryTextOpen as rememberTextOpen } from "../navSettings";
import { dismissOnOutsidePointer, registerTransientLayer } from "../transientLayers";
import { QueryDisplay } from "./QueryDisplay";
import { QueryLivePreview } from "./QueryLivePreview";
import { readLatestOr } from "../resourceRead";

/** One animation frame later (a timer where there is no rAF). */
const nextFrame = (callback: () => void): number =>
  typeof requestAnimationFrame === "function"
    ? requestAnimationFrame(callback)
    : (setTimeout(callback, 16) as unknown as number);
const cancelFrame = (handle: number) => {
  if (!handle) return;
  if (typeof cancelAnimationFrame === "function") cancelAnimationFrame(handle);
  else clearTimeout(handle);
};

// **The visual query builder: a resting SENTENCE that expands into a SHEET** (SPEC §7.2–§7.4).

export type { RegistryAccess };

export type QueryBuilderChange =
  | ((next: BuilderSession) => void)
  | ((next: BuilderSession) => Promise<boolean>);

/** One landed registry read, tagged with the graph scope and declaration
*  revision it answers — and carrying either the snapshot or the terminal
*  failure that replaced it. Both are scoped, so neither can be published for a
*  graph or a revision that has since been replaced. */
interface RegistryRead {
  scope: string;
  key: string;
  snapshot: RegistrySnapshot | null;
  failure: Error | null;
}

/** The pair an edit session holds (§4.3.1). */
export interface BuilderSession {
  query: Query;
  view: ViewSettings;
}

const errorMessage = (error: unknown): string =>
  error instanceof Error ? error.message : String(error);

// **`+ sort` and `+ summarize` are gone** (P5B, Q3).

// The text pane (§4.3.1, §7.1)

/** GH #619 item 4: the query text is hidden behind an "Edit as text" toggle, and the toggle remembers its state.
 *  It is a per-device preference persisted through the app-settings backend (`navSettings.ts`), so it survives a
 *  restart; localStorage does not in the Tine app. A failed write rolls the toggle back and toasts. */

/** How long the pane waits after the last keystroke before asking the engine. */
const PANE_DEBOUNCE_MS = 150;

/** The handle the sheet's `⟨advanced⟩` control and its retained rows use to reach the text. */
export interface PaneHandle {
  focus: () => void;
  /** Put the selection on a span of the CURRENT draft, when there is one that belongs to this exact text. */
  select: (span: Span, forText: string) => void;
}

/** One diagnostic, with the fields §4.3.2 gives it — not a joined string. */
interface PaneDiagnostics {
  /** The revision these belong to. A diagnostic never outlives its draft. */
  revision: number;
  /** The exact text they were computed for; a span is only offered while the textarea still holds this. */
  text: string;
  items: Diagnostic[];
}

/** The query text pane. */
function QueryTextPane(props: {
  session: () => BuilderSession | undefined;
  dialect: Extract<QueryPrintDialect, "og" | "tql">;
  /** Whether the pane is on screen at all. */
  visible: () => boolean;
  /** A successful parse of the current revision: the new filter/anchor, ready to be shown. */
  onParsed: (query: Query) => void;
  /** Commit the last-good parse. Enabled only when the current revision parsed. */
  onCommit: (query: Query) => void;
  onStale: (stale: boolean) => void;
  /** Published while mounted so the sheet above can bring the user here. */
  handle?: (handle: PaneHandle | null) => void;
  /** The §7.5 crossing notice, hosted here while the sheet is open (N3). */
  notice?: () => JSX.Element;
  /** The registry's keys, for the honest "what vocabulary exists" hint an unknown identifier gets. */
  vocabulary?: () => string[];
}): JSX.Element {
  const [draft, setDraft] = createSignal<string | null>(null);
  const [error, setError] = createSignal<string | null>(null);
  const [pending, setPending] = createSignal(false);
  const [good, setGood] = createSignal<Query | null>(null);
  const [diagnostics, setDiagnostics] = createSignal<PaneDiagnostics | null>(null);
  // The edit revision, and the revision whose parse currently holds `good`.
  const [revision, setRevision] = createSignal(0);
  const [goodRevision, setGoodRevision] = createSignal<number | null>(null);
  let timer: ReturnType<typeof setTimeout> | undefined;
  let disposed = false;
  let textarea: HTMLTextAreaElement | undefined;
  onCleanup(() => {
    clearTimeout(timer);
    // Disposal invalidates every outstanding response.
    disposed = true;
    setRevision((n) => n + 1);
    props.handle?.(null);
  });

  /** **The gate BOTH landing paths go through (I-20).**
  *
  *  One number answers all three questions the dossier separates, because all
  *  three bump it: an input bumps it, a session replacement bumps it (the reset
  *  effect below), and disposal bumps it. So "is this the answer to what is on
  *  screen right now, in a pane that still exists" is `mine === revision()`
  *  with `disposed` as the belt.
  *
  *  Note the difference from what this replaced. The old test was `mine >
  *  settled` — newer than the last response we ACCEPTED — which lets revision 1
  *  land while revision 2 is still in flight. */
  const accepts = (mine: number) => !disposed && mine === revision();

  // The session's own text is PRINTED BY RUST.
  const [printedResource] = createResource(
    () => (props.visible() ? props.session() : undefined),
    async (session) => {
      const key = { query: session.query, view: session.view };
      try {
        const printed = await readOwned(graphOwner(), backend().printQuery(session.query, session.view, props.dialect));
        if (printed.kind === "stale") return undefined;
        return { ...key, text: printed.value as string | null, refusal: null as string | null };
      } catch (error) {
        return { ...key, text: null as string | null, refusal: errorMessage(error) };
      }
    },
  );
  /** The printed text is shown only for the pair it was printed FROM. */
  const printedNow = () => {
    const landed = readLatestOr(printedResource, undefined, "query text");
    const current = props.session();
    if (!landed || !current) return undefined;
    return landed.query === current.query && landed.view === current.view ? landed : undefined;
  };

  // A session arriving from outside the pane (a chip edit, a save landing, a different block) invalidates …
  createEffect(() => {
    props.session();
    setRevision((n) => n + 1);
    setGoodRevision(null);
    clearTimeout(timer);
    setDraft(null);
    setError(null);
    setDiagnostics(null);
    setPending(false);
    setGood(null);
    props.onStale(false);
  });

  const text = () => draft() ?? printedNow()?.text ?? "";
  const refusal = () => (draft() === null ? printedNow()?.refusal ?? null : null);

  const run = async (source: string, mine: number) => {
    let parsed: ParsedQuery;
    try {
      const landed = await readOwned(graphOwner(() => accepts(mine)), backend().parseQuery(source, props.dialect));
      if (landed.kind === "stale") return;
      parsed = landed.value;
    } catch (error) {
      // The rejection path takes the SAME gate as the success path.
      if (!accepts(mine)) return;
      setPending(false);
      setError(errorMessage(error));
      setDiagnostics(null);
      props.onStale(true);
      return;
    }
    if (!accepts(mine)) return;
    setPending(false);
    // A diagnostic inside an `off` subtree carries `disabled` and does not invalidate (§3.5) — a parse with only …
    const all = parsed.query.diagnostics ?? [];
    setDiagnostics(all.length ? { revision: mine, text: source, items: all } : null);
    const blocking = all.filter((d) => !d.disabled);
    if (blocking.length) {
      setError(blocking.map((d) => d.message).join(" · "));
      props.onStale(true);
      return;
    }
    setError(null);
    setGood(parsed.query);
    setGoodRevision(mine);
    props.onStale(false);
    props.onParsed(parsed.query);
  };

  const onInput = (next: string) => {
    setDraft(next);
    const mine = revision() + 1;
    setRevision(mine);
    setGoodRevision(null);
    setDiagnostics(null);
    clearTimeout(timer);
    // The pane is not an options editor (§4.3.1).
    if (next.trim().endsWith("}")) {
      setPending(false);
      setError("The options map (title, collapsed) is edited with the title and Display controls, not here.");
      props.onStale(true);
      return;
    }
    setPending(true);
    timer = setTimeout(() => void run(next, mine), PANE_DEBOUNCE_MS);
  };

  /** Only a good parse OF THE CURRENT REVISION is saveable. */
  const savable = () =>
    draft() !== null && !pending() && !error() && good() !== null && goodRevision() === revision();

  // **Spans are already UTF-16 code units** (`queryIr.ts`: converted once, at the Rust boundary, precisely …
  const spanFor = (diagnostic: Diagnostic): Span | null => {
    const current = diagnostics();
    const span = diagnostic.span;
    if (!current || !span) return null;
    if (current.revision !== revision() || current.text !== text()) return null;
    if (span.start < 0 || span.end < span.start || span.end > current.text.length) return null;
    return span;
  };
  const select = (span: Span, forText: string) => {
    if (!textarea || textarea.value !== forText) return;
    textarea.focus();
    textarea.setSelectionRange(span.start, span.end);
  };
  onMount(() => props.handle?.({ focus: () => textarea?.focus(), select }));

  /** The registry keys that CONTAIN the token an unknown identifier names. */
  const vocabularyNear = (diagnostic: Diagnostic): string[] => {
    if (diagnostic.kind !== "unknown_ident") return [];
    const span = spanFor(diagnostic);
    const token = span ? text().slice(span.start, span.end).replace(/["']/g, "").trim() : "";
    if (token.length < 2) return [];
    const needle = token.toLowerCase();
    return (props.vocabulary?.() ?? [])
      .filter((key) => key.toLowerCase().includes(needle) && key.toLowerCase() !== needle)
      .slice(0, 4);
  };

  return (
    <div class="query-text-pane">
      <label class="query-text-pane-label" for={undefined}>
        {props.dialect === "tql" ? "Query text" : "Raw query DSL"}
      </label>
      <textarea
        ref={textarea}
        class="qb-input query-text-pane-input"
        classList={{ "query-text-pane-invalid": !!error() }}
        rows={3}
        spellcheck={false}
        aria-label={props.dialect === "tql" ? "Query text (TQL)" : "Query expression"}
        aria-invalid={error() ? "true" : undefined}
        value={text()}
        disabled={!!refusal()}
        onInput={(event) => onInput(event.currentTarget.value)}
      />
      <div class="query-text-pane-status">
        <Show when={refusal()}>
          {(message) => <span class="query-text-pane-error" role="alert">{message()}</span>}
        </Show>
        {/* The parser's OWN message, never a catch-all (I-9). */}
        <Show when={error() && !diagnostics()?.items.length}>
          <span class="query-text-pane-error" role="alert">{error()}</span>
        </Show>
        <Show when={pending() && !error()}>
          <span class="query-text-pane-pending">Checking…</span>
        </Show>
        <button
          type="button"
          class="qb-commit query-text-pane-save"
          disabled={!savable()}
          onClick={() => { const q = good(); if (q) props.onCommit(q); }}
        >
          Save query text
        </button>
      </div>
      {/* The structured half of §4.3.2: the kind, the span, the parser's own
          alternatives, and whether the diagnostic is inside an `off` subtree. */}
      <Show when={diagnostics()?.items.length}>
        <ul class="query-text-pane-diagnostics">
          <For each={diagnostics()!.items}>
            {(diagnostic) => (
              <li
                class="query-text-pane-diagnostic"
                classList={{ "is-disabled": diagnostic.disabled === true }}
                data-kind={diagnostic.kind}
              >
                <span class="query-text-pane-diagnostic-message">{diagnostic.message}</span>
                <Show when={diagnostic.disabled}>
                  <span class="query-text-pane-diagnostic-off"> (in a disabled condition — the query still runs)</span>
                </Show>
                <Show when={spanFor(diagnostic)}>
                  {(span) => (
                    <button
                      type="button"
                      class="query-text-pane-locate"
                      onClick={() => select(span(), diagnostics()!.text)}
                    >
                      Show me
                    </button>
                  )}
                </Show>
                <Show when={diagnostic.suggestions?.length}>
                  <span class="query-text-pane-diagnostic-alts">
                    <For each={diagnostic.suggestions!.slice(0, 4)}>
                      {(alternative, index) => (
                        <>
                          <Show when={index() > 0}>, </Show>
                          <code>{alternative}</code>
                        </>
                      )}
                    </For>
                  </span>
                </Show>
                <Show when={vocabularyNear(diagnostic).length}>
                  <span class="query-text-pane-diagnostic-alts">
                    Properties in this graph: <For each={vocabularyNear(diagnostic)}>
                      {(key, index) => (
                        <>
                          <Show when={index() > 0}>, </Show>
                          <code>{key}</code>
                        </>
                      )}
                    </For>
                  </span>
                </Show>
                {/* A syntax error the parser had no alternative for still gets a
                    route: the vocabulary above and the Guide's TQL reference. */}
                <Show when={diagnostic.kind === "syntax" && !diagnostic.suggestions?.length}>
                  <span class="query-text-pane-diagnostic-alts">
                    The fields and properties this graph has are in the picker above; the
                    query language is in the Guide under <em>Find and revisit → Query text (TQL)</em>.
                  </span>
                </Show>
              </li>
            )}
          </For>
        </ul>
      </Show>
      {/* §7.5: one notice, hosted here while the sheet is open. */}
      <Show when={props.notice}>{(render) => render()()}</Show>
    </div>
  );
}

// The host

// The one registry read, shared by every open builder (§6.4, I-13, N4)

/** The graph-scoped declaration revision, bumped when a `tine.type` is written. */
const [declarationBump, setDeclarationBump] = createSignal({ epoch: 0, revision: 0 });

/** Re-read the registry for THIS graph, in every open builder. */
export function requestQueryRegistryRefresh(): void {
  const epoch = graphEpoch();
  setDeclarationBump((prior) =>
    prior.epoch === epoch ? { epoch, revision: prior.revision + 1 } : { epoch, revision: 1 },
  );
}

/** The current graph's declaration revision. */
function declarationRevision(): number {
  const bump = declarationBump();
  return bump.epoch === graphEpoch() ? bump.revision : 0;
}

/** Tests mount several graphs in one process; the counter is module state. */
export function resetQueryRegistryRevisionForTests(): void {
  setDeclarationBump({ epoch: -1, revision: 0 });
}

/** **One registry read, however many surfaces ask for it** (D-14, I-12): every open builder keys the same
 * `sharedQueryResult` entry by graph scope, data revision and declaration revision. */
export function createQueryRegistryAccess(active: () => boolean): RegistryAccess {
  const registryScope = () => `${graphMeta()?.root ?? ""}\0${graphEpoch()}`;
  const registryKey = () =>
    active() ? `${dataRev()}\0${declarationRevision()}` : undefined;
  const [registrySnapshot] = createResource(
    () => {
      const key = registryKey();
      return key === undefined ? undefined : { scope: registryScope(), key };
    },
    async (request): Promise<RegistryRead | undefined> => {
      try {
        const landed = await readOwned(
          graphOwner(),
          sharedQueryResult(request.scope, `query-registry\0${request.key}`, () => backend().queryRegistry()),
        );
        if (landed.kind === "stale") return undefined;
        return { scope: request.scope, key: request.key, snapshot: landed.value, failure: null };
      } catch (error) {
        // A failed read is a terminal answer for this key, shown as such (I-9), never as an empty registry.
        return {
          scope: request.scope,
          key: request.key,
          snapshot: null,
          failure: error instanceof Error ? error : new Error(errorMessage(error)),
        };
      }
    },
  );
  // A type declaration changes operator semantics: an answer is usable only for the current graph AND revision.
  const registryRead = (): RegistryRead | undefined => {
    const landed = readLatestOr(registrySnapshot, undefined, "query vocabulary");
    return landed && landed.scope === registryScope() && landed.key === registryKey() ? landed : undefined;
  };
  let lastRows: { scope: string; rows: RegistryRow[] } | undefined;
  const registryRows = () => {
    const scope = registryScope();
    const current = registryRead()?.snapshot?.rows;
    if (current) lastRows = { scope, rows: current };
    return current ?? (lastRows?.scope === scope ? lastRows.rows : undefined);
  };
  const registryFailure = () => registryRead()?.failure ?? null;
  const currentRows = () => registryRead()?.snapshot?.rows;
  return {
    rows: registryRows,
    pending: () => registryKey() !== undefined && currentRows() === undefined && registryFailure() === null,
    failure: registryFailure,
    unavailable: () => registryKey() !== undefined && currentRows() === undefined,
    request: requestQueryRegistryRefresh,
    retry: requestQueryRegistryRefresh,
  };
}

/** Deepest-and-rightmost first, so removing several leaves in one pass never
*  invalidates a `loc` that has not been used yet. */
function compareLocsDescending(a: number[], b: number[]): number {
  for (let i = 0; i < Math.max(a.length, b.length); i++) {
    const left = a[i] ?? -1;
    const right = b[i] ?? -1;
    if (left !== right) return right - left;
  }
  return 0;
}

/** The four "Try:" suggestions of the empty state (§7.3). */
export function suggestedKeys(rows: RegistryRow[] | undefined): string[] {
  if (!rows?.length) return [];
  return [...rows]
    .sort((a, b) => {
      const byCount =
        b.count_blocks + b.count_pages - (a.count_blocks + a.count_pages);
      return byCount !== 0 ? byCount : a.normalized_name.localeCompare(b.normalized_name);
    })
    .slice(0, 4)
    .map((row) => row.normalized_name);
}

export function QueryBuilder(props: {
  /** The persisted reading of the query. */
  session: () => BuilderSession | undefined;
  /** Persist an edit. */
  onChange: QueryBuilderChange;
  /** The text pane's language: `tql` for a query block, `og` for the workspace, which materializes OG text. */
  paneDialect?: Extract<QueryPrintDialect, "og" | "tql">;
  /** The §7.5 crossing notice, drawn inside the pane while the sheet is open. */
  notice?: () => JSX.Element;
  /** Told whether the sheet is open, so the host can take the notice back when it closes. */
  onOpenChange?: (open: boolean) => void;
  /** The workspace's permanently expanded sheet (§7.2). */
  sheetAlwaysOpen?: boolean;
  /** The live result count, rendered beside the sentence (§7.2). */
  total?: JSX.Element;
  /** The pane's text no longer parses, so the rows on screen are the LAST reading that ran. */
  onStale?: (stale: boolean) => void;
  blockId?: string;
  /** The bindings a live preview runs under (the block's own run uses the same). */
  previewContext?: () => import("../editor/queryIr").ExecutionContext | undefined;
  /** The macro's "pages and blocks" choice (GH #619 item 9); absent on the workspace. */
  both?: import("./querySheetParts").BothKindsControl;
  parentTransientId?: string;
  /** Display writes use the host's guarded query save. Workspace callers may
   * leave it absent until their route owns a display draft. */
  display?: { view: () => ViewSettings; apply: (view: ViewSettings) => void | Promise<boolean>;
    formulas?: () => readonly string[] };
}): JSX.Element {
  // The pane's last-good parse, not yet saved.
  const [paneQuery, setPaneQuery] = createSignal<Query | null>(null);
  const [stale, setStale] = createSignal(false);
  const [open, setOpen] = createSignal(false);
  // The text pane is mounted only while its toggle is on (default off, remembered).
  // The remembered state is a durable device preference (navSettings), not localStorage.
  const textOpen = queryTextOpen;
  const setTextOpen = (next: boolean) => {
    rememberTextOpen(next);
    // A hidden pane cannot hold an unparsed draft, so nothing is left "stale" behind it.
    if (!next) {
      setStale(false);
      props.onStale?.(false);
    }
  };
  const [openMenu, setOpenMenu] = createSignal<string | null>(null);
  const [anchorPrompt, setAnchorPrompt] = createSignal<AnchorPrompt | null>(null);
  const [previewError, setPreviewError] = createSignal<string | null>(null);

  // I-20: anchor previews obey the same current-revision rule as text parses.
  let anchorRevision = 0;
  const invalidateAnchorPreview = () => {
    anchorRevision += 1;
    setAnchorPrompt(null);
  };
  onCleanup(invalidateAnchorPreview);

  createEffect(() => {
    props.session();
    setPaneQuery(null);
    invalidateAnchorPreview();
    setPreviewError(null);
  });

  const session = createMemo<BuilderSession | undefined>(() => {
    const persisted = props.session();
    if (!persisted) return undefined;
    const pane = paneQuery();
    return pane ? { query: pane, view: persisted.view } : persisted;
  });
  // The sheet always edits an `and`/`or` root, so "+ add condition" has somewhere to add.
  const root = createMemo(() => builderRoot(session()?.query.filter ?? { kind: "and", items: [] }));

  const sheetOpen = () => !!props.sheetAlwaysOpen || open();
  createEffect(() => { if (!sheetOpen()) invalidateAnchorPreview(); });
  // The host needs to know, because the §7.5 notice lives inline under the block while the sheet is shut and …
  createEffect(() => props.onOpenChange?.(sheetOpen()));

  // **The ONE graph-level read the builder makes (§6.4, K20, I-13, N4).** It used to make two: …
  const registry = createQueryRegistryAccess(sheetOpen);
  const suggestions = createMemo(() => suggestedKeys(registry.rows()));
  const vocabulary = () => (registry.rows() ?? []).map((row) => row.normalized_name);

  // Open the sheet (on its empty condition list, chooser CLOSED — Martin 2026-10-03, GH #619 comment 2) when this block was just created via `/query`.
  // **Consumed once this builder is CONNECTED, never at construction (GH #619).** The block swaps its edit
  // surface for its view surface right after `/query`, and a builder can be constructed into a subtree that is
  // thrown away or attached a frame later. The instance that took the one-shot flag at construction could be
  // the detached one: it opened a sheet no sentence anchored, which painted at the viewport's top-left. Waiting
  // for a connected sentence means the instance the user actually sees is the one that opens.
  if (props.blockId) {
    createEffect(() => {
      if (queryBuilderAutoOpen() !== props.blockId) return;
      let frame = 0;
      let alive = true;
      const take = () => {
        if (!alive || queryBuilderAutoOpen() !== props.blockId) return;
        if (!props.sheetAlwaysOpen && !sentenceEl?.isConnected) {
          frame = nextFrame(take);
          return;
        }
        setQueryBuilderAutoOpen(null);
        setOpen(true);
      };
      take();
      onCleanup(() => {
        alive = false;
        cancelFrame(frame);
      });
    });
  }

  /** A row edit: a new filter over the CURRENT reading, saved immediately. */
  const apply = (next: Filter) => {
    const current = session();
    if (!current) return;
    invalidateAnchorPreview();
    const outcome = props.onChange({ query: { ...current.query, filter: next }, view: current.view });
    setOpenMenu(null);
    return outcome;
  };
  /** §4.3.1 carry-forward: a parse replaces only the filter, the anchor and the diagnostics. */
  const carryForward = (parsed: Query): Query => {
    const current = props.session();
    const options = current && current.query.source.kind !== "builder"
      ? current.query.source.og_options ?? ""
      : "";
    return {
      anchor: parsed.anchor,
      filter: parsed.filter,
      diagnostics: parsed.diagnostics,
      source: parsed.source.kind === "builder"
        ? parsed.source
        : { ...parsed.source, og_options: options },
    };
  };
  const commitQuery = (query: Query) => {
    const current = session();
    if (!current) return;
    props.onChange({ query, view: current.view });
  };

  /**
  * **Switching the anchor re-validates through the ENGINE (§7.4, §3.5, D-14).**
  *
  * The frontend has no "does this leaf apply to this row" oracle and must not
  * grow one — that is the twin this campaign removed. So the preview is the
  * engine answering: print the query under the new anchor, parse it back, and
  * read the `not_applicable` diagnostics the lowering raised. The leaves that
  * do not apply come back RETAINED (`Raw(NotApplicable)`, P3's Rust half), so
  * "keep anyway" keeps the author's condition rather than a memory of it.
  */
  const switchAnchor = async (anchor: Anchor) => {
    const current = session();
    invalidateAnchorPreview();
    setPreviewError(null);
    if (!current || current.query.anchor === anchor) return;
    const mine = anchorRevision;
    const next: Query = { ...current.query, anchor };
    let parsed: ParsedQuery;
    try {
      const printed = await readOwned(graphOwner(ownedWhen(() => mine === anchorRevision)), backend().printQuery(next, current.view, "tql"));
      if (printed.kind === "stale") return;
      const landed = await readOwned(graphOwner(ownedWhen(() => mine === anchorRevision)), backend().parseQuery(printed.value, "tql"));
      if (landed.kind === "stale") return;
      parsed = landed.value;
    } catch (error) {
      if (mine !== anchorRevision) return;
      setPreviewError(errorMessage(error));
      return;
    }
    if (mine !== anchorRevision) return;
    const carried = carryForward(parsed.query);
    const notApplicable = (carried.diagnostics ?? []).filter((d) => d.kind === "not_applicable");
    const live = notApplicable.filter((d) => d.disabled !== true);
    if (live.length === 0) {
      // Nothing objected: commit the switch itself, not the round trip, so an untouched query keeps the tree it …
      if (notApplicable.length === 0) return commitQuery(next);
      // Only DISABLED objections: the grey Off rows were already off, and a prompt about conditions that are not …
      return commitQuery(carried);
    }
    const rootFilter = builderRoot(carried.filter);
    const sites = rawLeaves(rootFilter).filter(
      (site) => site.leaf.diagnostic_kind === "not_applicable" && !site.disabled,
    );
    setAnchorPrompt({
      anchor,
      count: live.length,
      total: countConditions(rootFilter),
      names: sites.map((site) => filterLabel(site.leaf)),
      onRemove: () => {
        let filter = rootFilter;
        for (const loc of sites.map((site) => site.loc).sort(compareLocsDescending)) {
          filter = loc.length === 0 ? { kind: "and", items: [] } : removeAt(filter, loc);
        }
        setAnchorPrompt(null);
        commitQuery({ ...carried, filter });
      },
      // The leaves stay as red rows with their message; the query is invalid and returns nothing until they are …
      onKeep: () => {
        setAnchorPrompt(null);
        commitQuery(carried);
      },
      onCancel: () => setAnchorPrompt(null),
    });
  };

  // -- the sheet's layer and its position ------------------------------------

  // **A ROOT transient layer whose id is unique per MOUNT, not per block.** `transientLayers` keys by id and a …
  const sheetLayerId = `query-sheet:${props.blockId ?? "workspace"}:${createUniqueId()}`;
  let sheetEl: HTMLDivElement | undefined;
  let sentenceEl: HTMLSpanElement | undefined;

  const dismissable = () => open() && !props.sheetAlwaysOpen;
  createEffect(() => {
    if (!dismissable()) return;
    const unregister = registerTransientLayer({
      id: sheetLayerId,
      root: () => sheetEl ?? null,
      trigger: () => sentenceEl ?? null,
      dismiss: () => {
        setOpen(false);
        return true;
      },
    });
    onCleanup(unregister);
  });
  dismissOnOutsidePointer({
    open: dismissable,
    // A press while one of the sheet's own popovers is open belongs to that popover's rung: closing the sheet …
    inside: () =>
      openMenu() !== null
      || sheetEl?.querySelector(".qs-menu, .qb-picker, .qb-menu")
      || document.querySelector(`[data-transient-parent="${sheetLayerId}"]`)
        ? [document.body]
        : [sheetEl ?? null, sentenceEl ?? null],
    dismiss: () => setOpen(false),
  });

  // The sheet is portalled and positioned from the sentence's rect on wide screens.
  // **Measured only from a CONNECTED sentence, and the sheet is not drawn until it has been (GH #619).** A
  // detached element's rect is all zeros, and the fixed anchor used to be drawn from that: the sheet opened at
  // the viewport's top-left. A builder whose sentence is not in the document yet retries each frame; once it
  // is, it measures, then re-measures after layout settles, on scroll/resize and when the sentence resizes.
  const [rect, setRect] = createSignal<{ top: number; left: number; width: number; placed: boolean } | null>(null);
  let observeSheet: (() => void) | undefined;
  const measure = (): boolean => {
    const element = sentenceEl;
    if (!element || !element.isConnected) return false;
    const box = element.getBoundingClientRect();
    // The sheet's own height (known once it is mounted) decides whether it hangs below, flips above, or is
    // pushed up so it never runs off the window. Hidden until that has been measured, so it never jumps.
    const view = window.visualViewport?.height ?? window.innerHeight;
    const next = {
      top: sheetEl ? placeSheetTop(box.top, box.bottom, sheetEl.offsetHeight, view) : box.bottom,
      left: box.left,
      width: Math.max(box.width, 320),
      placed: !!sheetEl,
    };
    const prev = untrack(rect);
    if (!prev || prev.top !== next.top || prev.left !== next.left || prev.width !== next.width || prev.placed !== next.placed) {
      setRect(next);
    }
    observeSheet?.();
    return true;
  };
  createEffect(() => {
    if (!open() || props.sheetAlwaysOpen) {
      setRect(null);
      return;
    }
    let alive = true;
    let frame = 0;
    const settle = () => {
      if (!alive) return;
      if (!measure()) {
        frame = nextFrame(settle);
        return;
      }
      // Layout that lands just after mount (the answers above hydrating) moves the sentence: look once more.
      frame = nextFrame(() => {
        measure();
        frame = nextFrame(measure);
      });
    };
    settle();
    const remeasure = () => void measure();
    let observer: ResizeObserver | undefined;
    if (typeof window !== "undefined") {
      window.addEventListener("scroll", remeasure, true);
      window.addEventListener("resize", remeasure);
      if (typeof ResizeObserver === "function") {
        const watching = new ResizeObserver(remeasure);
        observer = watching;
        let watchedSheet: Element | undefined;
        // The sheet mounts only after the first measurement; watch it from the first measurement after that.
        observeSheet = () => {
          if (sheetEl && sheetEl !== watchedSheet) {
            watchedSheet = sheetEl;
            watching.observe(sheetEl);
          }
        };
        if (sentenceEl) observer.observe(sentenceEl);
        const block = sentenceEl?.closest(".query-block");
        if (block) observer.observe(block);
      }
    }
    onCleanup(() => {
      alive = false;
      cancelFrame(frame);
      observer?.disconnect();
      observeSheet = undefined;
      if (typeof window === "undefined") return;
      window.removeEventListener("scroll", remeasure, true);
      window.removeEventListener("resize", remeasure);
    });
  });

  // The pane publishes a handle while it is mounted, so the sheet's `⟨advanced⟩` control and its retained rows …
  const [paneHandle, setPaneHandle] = createSignal<PaneHandle | null>(null);

  const footer = () => (
    <>
      {/* GH #619 item 7: the sheet covers the block's results, so it carries its own live ones. The
          workspace shows its results beside the sheet already. */}
      <Show when={!props.sheetAlwaysOpen}>
        <QueryLivePreview
          query={() => session()?.query}
          view={() => session()?.view ?? {}}
          context={props.previewContext}
          hostBlockId={props.blockId}
          both={props.both?.on}
        />
      </Show>
      <Show when={props.display}>{(display) => <QueryDisplay
        view={display().view} apply={display().apply} registry={registry} formulas={display().formulas}
        rowKind={() => session()?.query.anchor ?? "block"}
        parentTransientId={props.sheetAlwaysOpen ? props.parentTransientId : sheetLayerId} />}</Show>
      {/* GH #619 item 4: the text is one toggle away, not in the way. The sheet's rows are the
          primary editor; "Edit as text" opens the pane (remembered across sheets), and a
          retained leaf that rows cannot edit opens it for the user. A resting sentence and a
          closed toggle mount no pane at all. */}
      <button
        type="button"
        class="qs-text-toggle"
        aria-expanded={textOpen()}
        onClick={() => setTextOpen(!textOpen())}
      >
        Edit as text
      </button>
      <Show when={textOpen()} fallback={<Show when={props.notice}>{(render) => render()()}</Show>}>
        <QueryTextPane
          session={props.session}
          dialect={props.paneDialect ?? "tql"}
          visible={sheetOpen}
          vocabulary={vocabulary}
          notice={props.notice}
          handle={setPaneHandle}
          onParsed={(parsed) => setPaneQuery(carryForward(parsed))}
          onCommit={(parsed) => {
            const current = props.session();
            if (!current) return;
            props.onChange({ query: carryForward(parsed), view: current.view });
          }}
          onStale={(value) => {
            setStale(value);
            props.onStale?.(value);
          }}
        />
      </Show>
    </>
  );

  const sheet = (extraRef?: (element: HTMLDivElement) => void) => (
    <QuerySheet
      anchor={() => session()?.query.anchor ?? "block"}
      onAnchor={(anchor) => void switchAnchor(anchor)}
      both={props.both}
      anchorPrompt={anchorPrompt}
      root={root}
      query={() => session()?.query}
      apply={apply}
      registry={registry}
      onEditText={() => {
        if (textOpen()) return paneHandle()?.focus();
        setTextOpen(true);
        // The pane mounts on the next flush; focus its textarea once it has published its handle.
        queueMicrotask(() => paneHandle()?.focus());
      }}
      suggestions={suggestions}
      openMenu={openMenu}
      setOpenMenu={setOpenMenu}
      // In the workspace the sheet is not a layer of its own: its menus parent to the Advanced modal exactly as …
      layerId={props.sheetAlwaysOpen ? props.parentTransientId : sheetLayerId}
      footer={footer()}
      stale={stale()}
      sheetRef={(element) => {
        sheetEl = element;
        extraRef?.(element);
      }}
    />
  );

  return (
    <Show when={session()}>
      {(current) => (
        <div class="qs-builder">
          <Show when={!props.sheetAlwaysOpen}>
            <QuerySentence
              query={current().query}
              both={props.both?.on}
              total={props.total}
              open={open()}
              onOpen={() => setOpen(!open())}
              sentenceRef={(element) => {
                sentenceEl = element;
              }}
            />
          </Show>
          <Show when={previewError()}>
            {(message) => (
              <div class="qs-preview-error" role="alert">
                The anchor wasn't changed: {message()}
              </div>
            )}
          </Show>
          <Show when={props.sheetAlwaysOpen}>{sheet()}</Show>
          <Show when={open() && !props.sheetAlwaysOpen && rect() !== null}>
            <FloatingPortal>
              <div
                class="qs-overlay"
                onClick={(e) => {
                  stop(e);
                  setOpen(false);
                }}
              />
              <div
                class="qs-sheet-anchor"
                style={{
                  top: `${rect()?.top ?? 0}px`,
                  left: `${rect()?.left ?? 0}px`,
                  width: `${rect()?.width ?? 320}px`,
                  visibility: rect()?.placed ? undefined : "hidden",
                }}
              >
                {sheet()}
              </div>
            </FloatingPortal>
          </Show>
        </div>
      )}
    </Show>
  );
}
