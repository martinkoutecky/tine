/** The snapshot backend of a published query export (Stage 2).
 *
 *  An export's `app/` is this very frontend opened over `app/snapshot.json`:
 *  every answer the app needs was computed natively at export time by the one
 *  engine over the selected pages and nothing else. This module answers the
 *  `Backend` interface from that file. Its two query methods, `parseQuery` and
 *  `queryRun`, are lookups keyed exactly as `Macro.tsx` asks; a query the export
 *  was not made with is refused with a typed reason, never re-run in the
 *  browser. Everything that would write, sync, install or reach the OS is
 *  refused with `PublishedExportReadOnlyError`.
 *
 *  Spec: tine-agents/specs/notes/2026-09-14-publish-query-stage2.md §5.
 *  Classification of every method is pinned by `publishedBackend.guard.test.ts`.
 */
import type { Backend, LoadGraphResult } from "./backend";
import type { ExecutionContext, ParsedQuery, Query, QueryResult, QueryTextDialect, ViewSettings } from "./editor/queryIr";
import type { BacklinkFilterContext, BacklinkFilterTarget, BlockDto, BlockPreview, MatchEvidence, PageDto, PageEntry, PageRead, QueryExecution, QueryHit, QueryPageScope, RefGroup } from "./types";
import { previewDtoSubtree } from "./previewProjection";
import { pageIdentityKey } from "./pageIdentity";
import { blockRegions } from "./render/parse";
import { blockRefsInText } from "./render/pageRefs";
import { searchSubstringSpans } from "./editor/searchQuery";
import { searchFold } from "./editor/searchFold";

type PublishedPage = PageDto & { path: string };

/** `<meta name="tine-published" content="snapshot.json">` in the exported shell. */
export const PUBLISHED_META_NAME = "tine-published";
export const PUBLISHED_SNAPSHOT_SCHEMA = 1;
/** The reason code every refused query carries (typed-errors contract). */
export const PUBLISHED_QUERY_REASON = "published_export_static";
export const PUBLISHED_QUERY_MESSAGE = "This export answers only the queries it was made with.";
export const PUBLISHED_READ_ONLY_MESSAGE = "This is a read-only published export.";

export interface PublishedQueryExecution {
  argument: string;
  parsed: ParsedQuery;
}

/** One recorded query execution (`publish/app_export.rs::QuerySnapshot`). */
export interface PublishedQuery {
  host: string;
  argument: string;
  dialect: QueryTextDialect;
  properties: [string, string][];
  parsed: ParsedQuery;
  execution?: PublishedQueryExecution | null;
  context: ExecutionContext;
  executed_context: ExecutionContext;
  view: ViewSettings;
  result: QueryResult;
}

export interface PublishedSnapshot {
  schema: number;
  name: string;
  exported_at: string;
  home: string;
  pages: PublishedPage[];
  entries: PageEntry[];
  backlinks: Record<string, RefGroup[]>;
  block_ref_counts: Record<string, number>;
  aliases: [string, string][];
  icons: Record<string, string>;
  queries: PublishedQuery[];
}

/** The snapshot's relative URL when this document is a published export, else null. */
export function publishedSnapshotUrl(doc: Document | undefined = globalThis.document): string | null {
  const meta = doc?.querySelector(`meta[name="${PUBLISHED_META_NAME}"]`);
  const content = meta?.getAttribute("content")?.trim();
  return content ? content : null;
}

/** Whether this document is a published export (the one presentation flag). */
export function isPublishedExport(): boolean {
  return publishedSnapshotUrl() !== null;
}

let snapshotPromise: Promise<PublishedSnapshot> | null = null;

/** Largest snapshot document a viewer will read (I-22). Far above any real export;
 *  a larger body is a wrong or hostile file and refuses visibly. */
export const PUBLISHED_SNAPSHOT_MAX_BYTES = 512 * 1024 * 1024;

/** Read a response body of at most `max` bytes: a declared length over the cap refuses
 *  before any byte is read, and a body that outgrows it (no or wrong length) is cancelled
 *  as soon as the running total passes the cap. O(max) memory at worst. */
export async function readBounded(response: Response, max: number, label: string): Promise<Uint8Array> {
  const tooLarge = () => new Error(`${label}: larger than ${max} bytes`);
  const declared = Number(response.headers?.get("content-length") ?? "");
  if (Number.isFinite(declared) && declared > max) throw tooLarge();
  const reader = response.body?.getReader();
  if (!reader) {
    const whole = new Uint8Array(await response.arrayBuffer());
    if (whole.byteLength > max) throw tooLarge();
    return whole;
  }
  const chunks: Uint8Array[] = [];
  let total = 0;
  for (;;) {
    const { done, value } = await reader.read();
    if (done) break;
    total += value.byteLength;
    if (total > max) {
      // The oversize refusal is the answer; a failed cancel is attached, not dropped.
      try {
        await reader.cancel();
      } catch (cancelError) {
        throw new Error(tooLarge().message, { cause: cancelError });
      }
      throw tooLarge();
    }
    chunks.push(value);
  }
  const out = new Uint8Array(total);
  let at = 0;
  for (const chunk of chunks) {
    out.set(chunk, at);
    at += chunk.byteLength;
  }
  return out;
}

/** The one shared, memoized fetch of the snapshot. `main.tsx` awaits it before
 *  mounting; every backend method awaits it too, so the backend can be
 *  installed before any snapshot bytes exist. */
export function loadPublishedSnapshot(url = publishedSnapshotUrl() ?? "snapshot.json"): Promise<PublishedSnapshot> {
  if (!snapshotPromise) {
    snapshotPromise = (async () => {
      const response = await fetch(url, { cache: "no-store" });
      if (!response.ok) throw new Error(`snapshot ${url}: HTTP ${response.status}`);
      // I-22: bound the served bytes before they become a string and an object graph.
      const bytes = await readBounded(response, PUBLISHED_SNAPSHOT_MAX_BYTES, `snapshot ${url}`);
      const snapshot = JSON.parse(new TextDecoder().decode(bytes)) as PublishedSnapshot;
      validateSnapshot(snapshot);
      return snapshot;
    })();
  }
  return snapshotPromise;
}

/** Test-only: forget the memoized fetch. */
export function __resetPublishedSnapshotForTest(): void {
  snapshotPromise = null;
}

/** Refuse snapshots without the expected schema and required read-only data. */
export function validateSnapshot(snapshot: PublishedSnapshot): void {
  assertSnapshotDepth(snapshot);
  if (snapshot.schema !== PUBLISHED_SNAPSHOT_SCHEMA) {
    throw new Error(`snapshot schema ${String(snapshot.schema)} is not ${PUBLISHED_SNAPSHOT_SCHEMA}`);
  }
  for (const key of ["name", "home", "pages", "entries", "backlinks", "block_ref_counts", "aliases", "icons", "queries"] as const) {
    if (!(key in snapshot)) throw new Error(`snapshot lacks ${key}`);
  }
}

/** Admission bound for served JSON and query keys. Iterative O(JSON nodes),
 * including query/result trees and backlinks, before any recursive reader or
 * clone. Excessive depth and cycles refuse with a visible snapshot error. */
function assertSnapshotDepth(value: unknown): void {
  const pending = [{ value, depth: 0 }];
  while (pending.length) {
    const { value: node, depth } = pending.pop()!;
    if (!node || typeof node !== "object") continue;
    if (depth > 512) throw new Error("snapshot depth exceeds 512");
    for (const child of Object.values(node)) pending.push({ value: child, depth: depth + 1 });
  }
}

// ---- key parity with the engine -------------------------------------------

/** JSON with object keys sorted at every depth: two IR values are the same
 *  query exactly when their stable text is. */
export function stableJson(value: unknown): string {
  assertSnapshotDepth(value);
  return JSON.stringify(sortKeys(value));
}

function sortKeys(value: unknown): unknown {
  if (Array.isArray(value)) return value.map(sortKeys);
  if (value && typeof value === "object") {
    const out: Record<string, unknown> = {};
    for (const key of Object.keys(value as Record<string, unknown>).sort()) {
      const inner = (value as Record<string, unknown>)[key];
      if (inner !== undefined) out[key] = sortKeys(inner);
    }
    return out;
  }
  return value;
}

/** The key two `ViewSettings` are compared under. The engine serializes its
 *  dense struct (`sort: []`, `columns: []`, `aggregates: []` are always
 *  written) while the frontend's `queryDisplaySettings` omits every field it
 *  has nothing to say about; both mean the same display, so an absent field
 *  and an empty list are the same key. */
export function viewKey(view: ViewSettings): string {
  const out: Record<string, unknown> = {};
  for (const [key, value] of Object.entries(view)) {
    if (value === undefined || value === null) continue;
    if (Array.isArray(value) && value.length === 0) continue;
    out[key] = value;
  }
  return stableJson(out);
}

/** The host block's `tine.*` properties as `parseQuery` receives them, compared
 *  the way the engine reads them: keys case-insensitively, values trimmed. */
function propertiesEqual(a: [string, string][], b: [string, string][]): boolean {
  if (a.length !== b.length) return false;
  const norm = (pairs: [string, string][]) =>
    pairs.map(([key, value]) => `${key.trim().toLowerCase()}\0${value.trim()}`).sort();
  const left = norm(a);
  const right = norm(b);
  return left.every((entry, index) => entry === right[index]);
}

function literalNeedle(query: string): { empty: boolean; folded: string } {
  const raw = query.trim();
  return { empty: raw === "", folded: raw === "" ? "" : searchFold(raw) };
}

function includesFolded(text: string, needle: string): boolean {
  return searchFold(text).includes(needle);
}

function firstMappedSpan(text: string, needle: string): { start: number; end: number } | null {
  if (!needle) return null;
  return searchSubstringSpans(text, needle, 1)[0] ?? null;
}

// ---- the backend --------------------------------------------------------------

/** Methods declared optional on `Backend`; a published export leaves them
 *  absent so the browser fallbacks in `backend.ts` take over. */
const OPTIONAL_METHODS = new Set([
  "loadDrafts",
  "storeDraft",
  "retireDraft",
  // GH #181: external Tine links need the native URL handler; a published guide has none.
  "tineLinks",
]);

/** Build the snapshot backend. `load` is awaited lazily by every method. */
export function publishedBackend(load: () => Promise<PublishedSnapshot> = loadPublishedSnapshot): Backend {
  const appBools = new Map<string, boolean>();
  const appStrings = new Map<string, string>();
  let session: string | null = null;
  let workspaces = JSON.stringify({ version: 1, activeId: "published", workspaces: [{
    id: "published", name: "Published", blob: {
      tabs: [{ history: [{ kind: "journals" }], pos: 0, pinned: false }], activeIndex: 0,
      leftSidebar: true, rightSidebar: false, rightSidebarItems: [],
      favoritesSectionExpanded: true, recentSectionExpanded: true,
      layout: { kind: "pane", paneId: "main", tabs: [{ history: [{ kind: "journals" }], pos: 0, pinned: false }], activeIndex: 0 },
      focusedPaneId: "main", recentPages: [],
    },
  }] });
  let notices = '{"dismissed":[]}';
  let smoothScroll = false;
  let activationSeq = 0;
  const unsubscribed = async () => () => {};

  const pageByName = (snapshot: PublishedSnapshot, name: string): PublishedPage | null => {
    const wanted = pageIdentityKey(name);
    const direct = snapshot.pages.find((page) => pageIdentityKey(page.name) === wanted);
    if (direct) return direct;
    const alias = snapshot.aliases.find(([from]) => pageIdentityKey(from) === wanted);
    return alias ? snapshot.pages.find((page) => pageIdentityKey(page.name) === pageIdentityKey(alias[1])) ?? null : null;
  };
  const walk = (blocks: BlockDto[], visit: (block: BlockDto, ancestors: string[]) => void, ancestors: string[] = []) => {
    for (const block of blocks) {
      visit(block, ancestors);
      walk(block.children, visit, [...ancestors, block.raw.split("\n")[0] ?? ""]);
    }
  };
  const collect = (snapshot: PublishedSnapshot, keep: (block: BlockDto, page: PublishedPage) => boolean, limit = Infinity): RefGroup[] => {
    const groups: RefGroup[] = [];
    let budget = limit;
    for (const page of snapshot.pages) {
      if (budget <= 0) break;
      const matched: BlockDto[] = [];
      walk(page.blocks, (block, ancestors) => {
        if (budget > 0 && keep(block, page)) {
          matched.push({ ...structuredClone(block), breadcrumb: ancestors });
          budget--;
        }
      });
      if (matched.length) groups.push({ page: page.name, kind: page.kind, path: page.path, blocks: matched });
    }
    return groups;
  };
  const findBlock = (snapshot: PublishedSnapshot, uuid: string): { page: PublishedPage; block: BlockDto } | null => {
    for (const page of snapshot.pages) {
      let found: BlockDto | null = null;
      walk(page.blocks, (block) => {
        if (!found && ((blockRegions(block.raw, page.format ?? "md").id?.value.trim() ?? block.id) === uuid)) found = block;
      });
      if (found) return { page, block: found };
    }
    return null;
  };
  const staticQueryRefusal = async () => Object.assign(new Error(PUBLISHED_QUERY_MESSAGE), { reasonCode: PUBLISHED_QUERY_REASON });
  const readOnlyRefusal = async () => new Error(PUBLISHED_READ_ONLY_MESSAGE);
  const resolveBlock = async (uuid: string): Promise<RefGroup | null> => {
    const snapshot = await load();
    const found = findBlock(snapshot, uuid);
    if (!found) return null;
    return {
      page: found.page.name,
      kind: found.page.kind,
      path: found.page.path,
      blocks: [{ ...structuredClone(found.block), children: [] }],
    };
  };
  const quickSwitch = async (query: string, limit: number): Promise<PageEntry[]> => {
    const snapshot = await load();
    const wanted = literalNeedle(query);
    if (!wanted.empty && !wanted.folded) return [];
    return snapshot.entries
      .filter((entry) => wanted.empty || includesFolded(entry.name.trim(), wanted.folded))
      .slice(0, limit)
      .map((entry) => structuredClone(entry));
  };
  const backlinkGroups = (snapshot: PublishedSnapshot, name: string): RefGroup[] => {
    const exact = snapshot.backlinks[name];
    if (exact) return exact;
    const wanted = pageIdentityKey(name);
    const key = Object.keys(snapshot.backlinks).find((candidate) => pageIdentityKey(candidate) === wanted);
    return key ? snapshot.backlinks[key] : [];
  };
  const backlinkRoot = (groups: RefGroup[], target: BacklinkFilterTarget): { group: RefGroup; block: BlockDto } | null => {
    for (const group of groups) {
      if (group.kind !== target.kind || pageIdentityKey(group.page) !== pageIdentityKey(target.page)) continue;
      let found: BlockDto | null = null;
      walk(group.blocks, (block) => {
        if (!found && block.id === target.block_id) found = block;
      });
      if (found) return { group, block: found };
    }
    return null;
  };
  const backlinkText = (root: BlockDto): string => {
    const lines: string[] = [];
    walk([root], (block) => lines.push(block.raw));
    return lines.join("\n");
  };
  /** `../assets/<name>` — or null when the authored name would leave the
   *  export's own `assets/` folder. The rule is the static exporter's
   *  (`AssetSink::asset_relative` in `publish.rs`), applied to the name after
   *  its `assets/` prefix: a `?` query or `#` fragment is dropped, a remote
   *  reference, a backslash, an absolute path, a leading `.` step or a `..`
   *  step is refused, and empty and interior `.` steps collapse the way
   *  `Path::components` collapses them — so every name the export copied is
   *  answered under the path it was copied to, and nothing else is asked for.
   *  A colon inside a file name (`x:y.png`, `data:plot.png`) is a name. */
  const assetUrl = (name: string): string | null => {
    const bare = name.split(/[?#]/)[0] ?? "";
    if (bare.includes("://") || bare.includes("\\") || bare.startsWith("/")) return null;
    const steps = bare.split("/").filter((segment) => segment !== "");
    if (steps.length === 0 || steps[0] === "." || steps.includes("..")) return null;
    const segments = steps.filter((segment) => segment !== ".");
    if (segments.length === 0) return null;
    return `../assets/${segments.map(encodeURIComponent).join("/")}`;
  };

  const answered = {
    // ---- graph identity ----
    async loadGraph(): Promise<LoadGraphResult> {
      const snapshot = await load();
      return {
        kind: "loaded",
        binding_generation: 1,
        meta: {
          root: snapshot.name,
          journals_dir: "journals",
          pages_dir: "pages",
          preferred_workflow: "todo",
          shortcuts: {},
          start_of_week: 6,
          block_hidden_properties: [], linked_references_collapsed_threshold: 100,
          default_journal_template: null,
          default_home: snapshot.home,
          favorites: [],
          journal_page_title_format: "MMM do, yyyy",
          journal_file_name_format: "yyyy_MM_dd",
          preferred_format: "md",
          macros: {},
          enable_timetracking: true,
          show_brackets: true,
          doc_mode_enter_for_new_block: false,
          logical_outdenting: false,
          logbook_with_second_support: true,
          logbook_enabled_in_timestamped_blocks: true,
          logbook_enabled_in_all_blocks: false,
          guide_announced: true, mobile_gestures_disabled_in_block_with_tags: [],
        },
      };
    },
    async inspectGraphAccess(path: string) {
      const snapshot = await load();
      return { graph_root: path || snapshot.name, external_assets_path: null, approved: true };
    },
    async startupGraphPath() {
      return (await load()).name;
    },
    // ---- pages ----
    async pageInventory() {
      const entries = (await load()).entries.map((entry) => ({
        key: pageIdentityKey(entry.name), name: entry.name,
        is_journal: entry.kind === "journal", day: entry.date_key,
        target: { kind: "existing" as const, id: entry.path, others: [] },
      }));
      return { rev: "0", entries, unreadable: [] };
    },
    async resolvePage(name: string, kind: "journal" | "page") {
      const snapshot = await load();
      const page = pageByName(snapshot, name);
      return page && page.kind === kind
        ? { kind: "existing" as const, id: page.path, others: [] }
        : { kind: "absent" as const, id: "" };
    },
    async getPage(name: string, kind: "journal" | "page") {
      const snapshot = await load();
      const page = pageByName(snapshot, name);
      if (!page || page.kind !== kind) return null;
      return { ...structuredClone(page), id: page.path } satisfies PageRead;
    },
    async getPageByPath(path: string) {
      const snapshot = await load();
      const page = snapshot.pages.find((candidate) => candidate.path === path);
      return page ? { ...structuredClone(page), id: page.path } satisfies PageRead : null;
    },
    async journalFeedPage(limit: number, beforeDay: number | null) {
      const snapshot = await load();
      const now = new Date();
      const as_of_day = now.getFullYear() * 10_000 + (now.getMonth() + 1) * 100 + now.getDate();
      const candidates = snapshot.entries
        .filter((entry) => entry.kind === "journal" && entry.date_key !== null)
        .filter((entry) => beforeDay === null || (entry.date_key as number) < beforeDay)
        .sort((a, b) => (b.date_key as number) - (a.date_key as number));
      const rows = candidates.slice(0, limit);
      const pages = rows
        .map((entry) => pageByName(snapshot, entry.name))
        .filter((page): page is PublishedPage => page !== null)
        .map((page) => ({ ...structuredClone(page), id: page.path } satisfies PageRead));
      const done = rows.length === candidates.length;
      return {
        pages,
        next_before_day: done || !rows.length ? null : (rows[rows.length - 1].date_key as number),
        done,
        as_of_day,
      };
    },
    async journalContentDays() {
      const snapshot = await load();
      return snapshot.entries
        .filter((entry) => entry.kind === "journal" && entry.date_key !== null)
        .map((entry) => entry.date_key as number);
    },
    async pageIcons(names: string[]) {
      const snapshot = await load();
      const out: Record<string, string> = {};
      for (const name of names) {
        const icon = snapshot.icons[name];
        if (icon !== undefined) out[name] = icon;
      }
      return out;
    },
    // ---- references ----
    async getBacklinks(name: string) {
      const snapshot = await load();
      return structuredClone(backlinkGroups(snapshot, name));
    },
    async getBacklinkFilterContext(name: string, targets: BacklinkFilterTarget[]): Promise<BacklinkFilterContext> {
      const snapshot = await load();
      const groups = backlinkGroups(snapshot, name);
      const roots: { group: RefGroup; block: BlockDto; text: string }[] = [];
      const requested = new Set<string>();
      for (const target of targets) {
        const key = `${target.kind}\0${pageIdentityKey(target.page)}\0${target.block_id}`;
        if (requested.has(key)) continue;
        requested.add(key);
        const root = backlinkRoot(groups, target);
        if (root) roots.push({ ...root, text: backlinkText(root.block) });
      }
      return {
        entries: roots.map((root) => ({
          page: root.group.page,
          kind: root.group.kind,
          block_id: root.block.id,
          text: root.text,
          facets: [...(root.block.tags ?? []), ...(root.block.marker ? [root.block.marker] : [])],
        })),
        truncated: roots.length < requested.size,
      };
    },
    async getUnlinkedRefs() {
      await load();
      return [];
    },
    async getBlockRefCounts() {
      return { ...(await load()).block_ref_counts };
    },
    async getBlockReferrers(uuid: string) {
      const snapshot = await load();
      // Admission by substring, decision by the parser: a `((uuid))` inside code is not a reference.
      return collect(snapshot, (block, page) =>
        block.raw.includes(uuid) && blockRefsInText(block.raw, page.format ?? "md").some((v) => v.trim() === uuid));
    },
    resolveBlock,
    async resolveBlocks(uuids: string[]): Promise<(RefGroup | null)[]> {
      return Promise.all(uuids.map((uuid) => resolveBlock(uuid)));
    },
    async previewBlock(uuid: string, maxNodes: number): Promise<BlockPreview | null> {
      const snapshot = await load();
      const found = findBlock(snapshot, uuid);
      if (!found) return null;
      const { blocks, truncated } = previewDtoSubtree(found.block, maxNodes, "owned");
      return {
        group: { page: found.page.name, kind: found.page.kind, path: found.page.path, blocks },
        truncated,
      };
    },
    // ---- search ----
    async search(query: string, limit: number) {
      const snapshot = await load();
      const wanted = literalNeedle(query);
      if (wanted.empty || !wanted.folded) return [];
      return collect(snapshot, (block) => includesFolded(block.raw, wanted.folded), limit);
    },
    quickSwitch,
    async captureQuickSwitch(query: string, limit: number): Promise<PageEntry[]> {
      return quickSwitch(query, limit);
    },
    async queryFacets() {
      await load();
      return [];
    },
    // ---- queries: the two seams (§3.2) ----
    async parseQuery(text: string, dialect: QueryTextDialect, blockProperties: [string, string][] = []): Promise<ParsedQuery> {
      const snapshot = await load();
      for (const record of snapshot.queries) {
        if (record.dialect !== dialect || !propertiesEqual(record.properties, blockProperties)) continue;
        if (record.argument === text) return structuredClone(record.parsed);
        if (record.execution && record.execution.argument === text) return structuredClone(record.execution.parsed);
      }
      throw await staticQueryRefusal();
    },
    /** The record whose query IR, page context and view all match wins; two
     *  identical queries on one page that differ only in `tine.sample::` are
     *  two records with two views. Then the same query in the same context
     *  under another view (a later view change is refused anyway, so the baked
     *  answer is the one the export shows). A run asked with no page at all
     *  (a query rendered inside a sheet cell, which carries no host page) falls
     *  back to the one record of that query. */
    async queryRun(query: Query, view: ViewSettings, context?: ExecutionContext): Promise<QueryResult> {
      const snapshot = await load();
      const wanted = stableJson(query);
      const wantedView = viewKey(view);
      const page = context?.current_page == null ? undefined : pageIdentityKey(context.current_page);
      const candidates = snapshot.queries.filter((record) =>
        stableJson(record.parsed.query) === wanted || (record.execution && stableJson(record.execution.parsed.query) === wanted));
      const inContext = candidates.filter((record) =>
        (record.context.current_page == null ? undefined : pageIdentityKey(record.context.current_page)) === page);
      const hit = inContext.find((record) => viewKey(record.view) === wantedView)
        ?? inContext[0]
        ?? (page === undefined ? candidates[0] : undefined);
      if (hit) return structuredClone(hit.result);
      throw await staticQueryRefusal();
    },
    /** The explicitly routed Ctrl-K consumer gets a plain substring match over page names,
     *  aliases and block text from the snapshot — navigation, not a query. A
     *  query-language search (any other consumer) is refused: the export holds
     *  answers, not an index. */
    async runGraphSearch(source: string, pageLimit: number, blockLimit: number, lane?: string, _explain?: boolean, scope?: QueryPageScope, _pageMatchScope?: import("./editor/queryIr").FriendlyPageMatchScope, _views?: { page: ViewSettings; block: ViewSettings }): Promise<QueryExecution> {
      const snapshot = await load();
      if (lane !== "quick-switch" && lane !== "quick-switch:current-page") throw await staticQueryRefusal();
      const wanted = literalNeedle(source);
      const hits: QueryHit[] = [];
      const empty = { hits, diagnostics: [], explanation: { branches: [] }, has_more: { pages: false, blocks: false }, cancelled: false };
      if (wanted.empty || !wanted.folded) return empty;
      const evidence = (field: "page_name" | "visible_content", text: string): MatchEvidence[] => {
        const span = firstMappedSpan(text, wanted.folded);
        return span === null ? [] : [{ clause_id: 0, field, mode: "contains", spans: [span] }];
      };
      const pageHits: QueryHit[] = [];
      if (!scope) {
        for (const entry of snapshot.entries) {
          const alias = snapshot.aliases.find(([from, to]) =>
            pageIdentityKey(to) === pageIdentityKey(entry.name) && includesFolded(from.trim(), wanted.folded)
          )?.[0];
          const name = searchFold(entry.name.trim());
          if (!name.includes(wanted.folded) && !alias) continue;
          const match_class = name === wanted.folded ? "exact" : name.startsWith(wanted.folded) ? "prefix" : "substring";
          const score = match_class === "exact" ? 3 : match_class === "prefix" ? 2 : 1;
          pageHits.push({ entity: "page", page: structuredClone(entry), display_text: entry.name, evidence: evidence("page_name", entry.name), score, match_class, ...(alias ? { matched_alias: alias } : {}) });
        }
        pageHits.sort((a, b) => (b.score ?? 0) - (a.score ?? 0));
      }
      const blockHits: QueryHit[] = [];
      for (const page of snapshot.pages) {
        if (scope && (scope.path ? page.path !== scope.path : pageIdentityKey(page.name) !== pageIdentityKey(scope.name))) continue;
        walk(page.blocks, (block) => {
          const text = block.raw.split("\n")[0] ?? "";
          if (!includesFolded(block.raw, wanted.folded)) return;
          blockHits.push({ entity: "block", page: page.name, kind: page.kind, path: page.path, block: { ...structuredClone(block), children: [] }, display_text: text, evidence: evidence("visible_content", text), score: 1, match_class: "substring" });
        });
      }
      hits.push(...pageHits.slice(0, pageLimit), ...blockHits.slice(0, blockLimit));
      return { ...empty, has_more: { pages: pageHits.length > pageLimit, blocks: blockHits.length > blockLimit } };
    },
    // ---- assets and the browser ----
    // A refused or missing asset rejects (I-9): an empty byte array reads as a valid empty
    // file and blanks a PDF or image with no cause. The live backend rejects the same way.
    async readAsset(name: string, maxBytes: number) {
      await load();
      const url = assetUrl(name);
      if (!url) throw new Error(`asset ${name}: not a file inside the published assets`);
      const response = await fetch(url);
      if (!response.ok) throw new Error(`asset ${name}: HTTP ${response.status}`);
      return readBounded(response, maxBytes, `asset ${name}`);
    },
    async streamAsset(name: string) {
      await load();
      const url = assetUrl(name);
      if (!url) throw new Error(`asset ${name}: not a file inside the published assets`);
      return url;
    },
    async openExternal(url: string) {
      window.open(url, "_blank", "noopener");
    },
    async openAsset(name: string) {
      const url = assetUrl(name);
      if (url) window.open(url, "_blank", "noopener");
    },
    async writeText(text: string) {
      const { writeClipboardTextStrict } = await import("./clipboard");
      await writeClipboardTextStrict(text);
    },
    async writeRich(text: string) {
      const { writeClipboardTextStrict } = await import("./clipboard");
      await writeClipboardTextStrict(text);
    },
    async confirm(message: string) {
      return window.confirm(message);
    },
  };

  const constant = {
    graphBindingGeneration: () => 1,
    async appPlatform(): Promise<"android" | "ios" | "desktop"> {
      return "desktop";
    },
    async listKnownGraphs() {
      return [];
    },
    async gpuEnv() {
      return { software_forced: false, appimage: false, flatpak: false };
    },
    // A published export has no backend clock; the browser's is the authority.
    async localClock() {
      const now = Date.now();
      return { offset_minutes: -new Date(now).getTimezoneOffset(), unix_ms: now };
    },
    async appArchitecture() {
      return "unknown";
    },
    async watcherLatencyRecent() {
      return [];
    },
    async debugInfo() {
      return { enabled: false, path: "", recorderActive: false, previousExitUnclean: false };
    },
    async debugLog() {},
    async diagnosticFrontendEvent() {},
    async diagnosticTimingEvent() {},
    async diagnosticSessionActive() {},
    async getAppBool(key: string, fallback: boolean) {
      return appBools.get(key) ?? fallback;
    },
    async setAppBool(key: string, value: boolean) {
      appBools.set(key, value);
    },
    async getAppString(key: string, fallback: string) {
      return appStrings.get(key) ?? fallback;
    },
    async setAppString(key: string, value: string) {
      appStrings.set(key, value);
    },
    async loadSession() {
      return session;
    },
    async saveSession(data: string) {
      session = data;
    },
    async loadWorkspaces() {
      return workspaces;
    },
    async saveWorkspaces(data: string) {
      workspaces = data;
      return "durable" as const;
    },
    async loadNotices() {
      return notices;
    },
    async saveNotices(data: string) {
      notices = data;
    },
    async readCustomCss() {
      return "";
    },
    async getSmoothScroll() {
      return smoothScroll;
    },
    async setSmoothScroll(value: boolean) {
      smoothScroll = value;
    },
    async warmDone() {
      return true;
    },
    async indexingProgress() {
      return null;
    },
    async listInstalledPlugins() {
      return [];
    },
    async loadPluginRegistryCache() {
      return { kind: "absent" as const };
    },
    async readLocalImage() {
      return new Uint8Array();
    },
    async listTemplates() {
      return [];
    },
    async guidePages() {
      return [];
    },
    async activateEditor(path: string) {
      return { activation: ++activationSeq, target: path, prospective: false };
    },
    async activateAbsentEditor() {
      return { activation: ++activationSeq, target: "", prospective: true };
    },
    async retireEditorActivation() {
      return true;
    },
    async presentConflictOverride(): Promise<"authorised" | "superseded" | "withdrawn"> {
      return "withdrawn";
    },
    async openPdf() {
      return { highlights: [], page: null, scale: null };
    },
    async readHighlights() {
      return [];
    },
    async queryRegistry() {
      return { rows: [], generation: 0 };
    },
    async queryOgExpressible() {
      return false;
    },
    async printQuery(): Promise<never> {
      const { QueryPrintRefusedError } = await import("./backend");
      throw new QueryPrintRefusedError("not_applicable", null);
    },
    async takeIdentifierMigrationNotice() {
      return false;
    },
    async takeDataHomeFallbackNotice() {
      return null;
    },
    async getBackupKeep() {
      return 0;
    },
    async listBackups() {
      return [];
    },
    async getCaptureEnterFiles() {
      return true;
    },
    async getLinkFirstMatch() {
      return false;
    },
    async getWatchMode() {
      return "inotify";
    },
    async listSpellcheckDictionaries() {
      return [];
    },
    async applySpellcheck() {},
    async setSystemBarAppearance() {},
    async closeGraphWindow() {},
    async trayApply() {
      return { supported: false, active: false, problem: null };
    },
    async openDevtools() {},
    async captureTarget() {
      return "main";
    },
    async bindCaptureGraph() {},
    async defaultGraphParent() {
      return "";
    },
    async graphSourceFiles() {
      return { files: [], skipped: [] };
    },
    async conflictInventory() {
      return { sync_conflicts: [], vcs_markers: [], queue: [] };
    },
    async listJournalConflicts() {
      return [];
    },
    async listSyncConflicts() {
      return [];
    },
    async listJournalFilenameMigrations() {
      return [];
    },
    async listOrphanAssets() {
      return [];
    },
    async assetTrashStats() {
      return { count: 0, bytes: 0, pages: 0, journals: 0, conflicts: 0, other: 0 };
    },
    async clipboardFiles() {
      return { files: [], skipped: 0, truncated: false };
    },
    async detectMediaEditor() {
      return "";
    },
    async readClipboardImage() {
      return null;
    },
    async diagnosticReport() {
      return { text: "{}", suggestedFileName: "tine-diagnostics-export.json" };
    },
    async saveDiagnosticReport() {
      return false;
    },
    async clearDiagnostics() {},
    // Windows Defender is the native app's concern; a published export never shows the hint.
    async defenderHint() {
      return { show: false };
    },
    async dismissDefenderHint() {},
    onStorageTransition: unsubscribed,
    onGraphRescanComplete: unsubscribed,
    onConflictsChanged: unsubscribed,
    onGraphChanged: unsubscribed,
    onGraphChangedBulk: unsubscribed,
    onAssetChanged: unsubscribed,
    onGraphConfigChanged: unsubscribed,
    onCustomCssChanged: unsubscribed,
    onGraphReopened: unsubscribed,
    onQueryProjectionChanged: unsubscribed,
    onGraphWatchStatus: unsubscribed,
    onGraphUnreadablePages: unsubscribed,
    onGraphVerificationProgress: unsubscribed,
  };

  const explicit: Record<string, unknown> = { ...answered, ...constant };
  return new Proxy(explicit, {
    get(target, property) {
      if (typeof property !== "string") return undefined;
      if (property in target) return target[property];
      if (OPTIONAL_METHODS.has(property)) return undefined;
      if (property === "then") return undefined;
      return async () => {
        throw await readOnlyRefusal();
      };
    },
    has(target, property) {
      return typeof property === "string" && !OPTIONAL_METHODS.has(property) ? true : property in target;
    },
  }) as unknown as Backend;
}

/** The method classes §5 pins. `publishedBackend.guard.test.ts` requires every
 *  `Backend` method to be in exactly one. */
export const PUBLISHED_ANSWERED_METHODS = [
  "pageInventory",
  "resolvePage",
  "loadGraph",
  "inspectGraphAccess",
  "startupGraphPath",
  "listPages",
  "getPage",
  "getPageByPath",
  "journalFeedPage",
  "journalContentDays",
  "existingPageNames",
  "referencedPageNames",
  "pageAliases",
  "pageIcons",
  "getBacklinks",
  "getBacklinkFilterContext",
  "getUnlinkedRefs",
  "getBlockRefCounts",
  "getBlockReferrers",
  "resolveBlock",
  "resolveBlocks",
  "previewBlock",
  "search",
  "quickSwitch",
  "captureQuickSwitch",
  "queryFacets",
  "parseQuery",
  "queryRun",
  "runGraphSearch",
  "readAsset",
  "streamAsset",
  "openExternal",
  "openAsset",
  "writeText",
  "writeRich",
  "confirm",
] as const;

export const PUBLISHED_CONSTANT_METHODS = [
  "graphBindingGeneration",
  "appPlatform",
  "localClock",
  "appArchitecture",
  "watcherLatencyRecent",
  "listKnownGraphs",
  "gpuEnv",
  "debugInfo",
  "debugLog",
  "diagnosticFrontendEvent",
  "diagnosticTimingEvent",
  "diagnosticSessionActive",
  "getAppBool",
  "setAppBool",
  "getAppString",
  "setAppString",
  "loadSession",
  "saveSession",
  "loadWorkspaces",
  "saveWorkspaces",
  "loadNotices",
  "saveNotices",
  "readCustomCss",
  "getSmoothScroll",
  "setSmoothScroll",
  "warmDone",
  "indexingProgress",
  "listInstalledPlugins",
  "loadPluginRegistryCache",
  "readLocalImage",
  "listTemplates",
  "guidePages",
  "activateEditor",
  "activateAbsentEditor",
  "retireEditorActivation",
  "presentConflictOverride",
  "openPdf",
  "readHighlights",
  "queryRegistry",
  "queryOgExpressible",
  "printQuery",
  "takeIdentifierMigrationNotice",
  "takeDataHomeFallbackNotice",
  "getBackupKeep",
  "listBackups",
  "getCaptureEnterFiles",
  "getLinkFirstMatch",
  "getWatchMode",
  "listSpellcheckDictionaries",
  "applySpellcheck",
  "setSystemBarAppearance",
  "closeGraphWindow",
  "trayApply",
  "openDevtools",
  "captureTarget",
  "bindCaptureGraph",
  "defaultGraphParent",
  "graphSourceFiles",
  "conflictInventory",
  "listJournalConflicts",
  "listSyncConflicts",
  "listJournalFilenameMigrations",
  "listOrphanAssets",
  "assetTrashStats",
  "clipboardFiles",
  "detectMediaEditor",
  "readClipboardImage",
  "diagnosticReport",
  "saveDiagnosticReport",
  "clearDiagnostics",
  "defenderHint",
  "dismissDefenderHint",
  "onStorageTransition",
  "onGraphRescanComplete",
  "onConflictsChanged",
  "onGraphChanged",
  "onGraphChangedBulk",
  "onAssetChanged",
  "onGraphConfigChanged",
  "onCustomCssChanged",
  "onGraphReopened",
  "onQueryProjectionChanged",
  "onGraphWatchStatus",
  "onGraphUnreadablePages",
  "onGraphVerificationProgress",
] as const;

/** Refused with `PublishedExportReadOnlyError`: writes, sync, install,
 *  capture, native UI, OS. Pinned here so a new `Backend` method must be
 *  classified deliberately. */
export const PUBLISHED_REFUSED_METHODS = [
  "addDefenderExclusion",
  "savePages",
  "publishLive",
  "sheetExportInputs",
  "approveExternalAssets",
  "openGraphWindow",
  "forgetKnownGraph",
  "revealKnownGraph",
  "installPlugin",
  "uninstallPlugin",
  "readPluginEntry",
  "setPluginEnabled",
  "verifyPluginRegistry",
  "storePluginRegistryCache",
  "createGraph",
  "suggestGraphName",
  "savePage",
  "beginDirectCrossPageMove",
  "finishDirectCrossPageMove",
  "copyGuideIntoGraph",
  "setGuideAnnounced",
  "deletePage",
  "renamePage",
  "publishHtml",
  "publishQueryPlan",
  "publishQuery",
  "pagePrintHtml",
  "queryExplainEmpty",
  "runQuery",
  "exportQuerySubtrees",
  "runAdvancedQuery",
  "setFavorites",
  "setFavoritesPage",
  "setDefaultHome",
  "setPreferredWorkflow",
  "setTimetrackingEnabled",
  "setShowBrackets",
  "setDocModeEnterForNewBlock",
  "setLogicalOutdenting",
  "setPreferredFormat",
  "setJournalTitleFormat",
  "setDefaultJournalTemplate",
  "setStartOfWeek",
  "openPageFile",
  "editCustomCss",
  "editAssetExternal",
  "trashAsset",
  "emptyAssetTrash",
  "duplicateJournalDiff",
  "resolveDuplicateJournalDay",
  "rescanGraphNow",
  "applyJournalFilenameMigrations",
  "trashJournalFile",
  "readJournalFile",
  "mergePages",
  "renameFileToPage",
  "syncConflictDiff",
  "textBlockDiff",
  "textBlockDiff3",
  "liveConflictDiff",
  "resolveLiveConflict",
  "vcsMarkerConflictDiff",
  "resolveVcsMarkerConflict",
  "resolveSyncConflict",
  "trashSyncConflict",
  "saveAsset",
  "importAsset",
  "importNativeCapture",
  "readTextFile",
  "pickFolder",
  "pickGraphFolder",
  "prepareGraphFolder",
  "pickFile",
  "capturePhoto",
  "startRecording",
  "stopRecording",
  "cancelRecording",
  "copyImageToClipboard",
  "writeHighlights",
  "savePdfAreaImage",
  "rollbackPdfAreaImage",
  "setBackupKeep",
  "setCaptureEnterFiles",
  "setWatchMode",
  "restoreBackup",
  "retryIndex",
  "createGraphVerification",
  "cancelGraphVerification",
  "saveGraphVerificationReport",
] as const;

/** Optional `Backend` members a published export leaves absent. */
export const PUBLISHED_ABSENT_METHODS = [...OPTIONAL_METHODS] as readonly string[];
