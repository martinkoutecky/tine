import { optionsUpdater } from "./primitives";
import { For, Show, createEffect, createMemo, createSignal, onCleanup, onMount, type JSX } from "solid-js";
import { graphOwner, readOwned, type Owner } from "../owned";
import { reportUiFailure } from "../uiFailure";
import { exportModal, closeExportModal, typographyMode, type ExportRequest } from "../ui";
import { pushToast } from "../toasts";
import { graphMeta } from "../graphSession";
import { exportNodesFor, formatForPage } from "../document";
import { backend } from "../backend";
import { writeClipboardText } from "../clipboard";
import { resolveBlockBatched, resolvedBlockRefSync } from "../resolveBatch";
import { expandTemplate } from "../render/inline";
import { visibleBody, isRenderHiddenProp } from "../render/block";
import { parseBlock, blockRegions, propertyValueInline } from "../render/parse";
import { splitTrailingMap } from "../editor/edn";
import { formFamilyForMacroName, isQueryMacroName } from "../editor/queryMacro";
import {
  exportOutline,
  DEFAULT_EXPORT_OPTIONS,
  type ExportContent,
  type ExportNode,
  type ExportOptions,
  type IndentStyle,
  type MaxDepth,
} from "../editor/exportText";
import { exportOpml } from "../editor/exportOpml";
import { exportHtml } from "../editor/exportHtml";
import type { Block, Format, Inline, ListItem } from "../render/ast";
import type { BlockDto, PageDto, QueryExportResult, QueryExportSpec, RefGroup } from "../types";
import { registerTransientLayer } from "../transientLayers";

const STORE_KEY = "tine.exportOptions";
type ExportFormat = "text" | "opml" | "html";

// Persist the last-used options so the modal opens the way you left it (the
// indent style especially — most people pick one and keep it).
function loadOptions(): ExportOptions {
  try {
    const raw = localStorage.getItem(STORE_KEY);
    if (raw) return { ...DEFAULT_EXPORT_OPTIONS, ...JSON.parse(raw) };
  } catch {
    /* ignore malformed/missing */
  }
  return { ...DEFAULT_EXPORT_OPTIONS };
}
function saveOptions(o: ExportOptions): void {
  try {
    localStorage.setItem(STORE_KEY, JSON.stringify(o));
  } catch {
    /* storage unavailable — non-fatal */
  }
}

// The two content choices name the OUTPUT the user gets, not the internal mode
// (GH #352: "Rendered"/"Source" left the preserve-Markdown option
// undiscoverable). Values stay "rendered"/"source" so saved settings still work.
const CONTENT_STYLES: { value: ExportContent; label: string; hint: string }[] = [
  { value: "rendered", label: "Plain text", hint: "cleaned — the text as displayed, without markup markers (bold, highlighting, links)" },
  { value: "source", label: "Markdown", hint: "formatting preserved — block references resolved and embeds expanded" },
];

const FORMAT_STYLES: { value: ExportFormat; label: string }[] = [
  { value: "text", label: "Text" },
  { value: "opml", label: "OPML" },
  { value: "html", label: "HTML" },
];

const INDENT_STYLES: { value: IndentStyle; label: string; hint: string }[] = [
  { value: "dashes", label: "Dashes", hint: "Logseq outline (- bullets)" },
  { value: "spaces", label: "Spaces", hint: "indent, no bullets" },
  { value: "no-indent", label: "No indent", hint: "flat, no bullets" },
];

const MAX_DEPTHS: MaxDepth[] = ["all", 1, 2, 3, 4, 5, 6, 7, 8, 9];

// `sourceOnly` toggles are moot in rendered mode (the markers are already gone).
type ExportToggle = { key: keyof ExportOptions; label: string; sourceOnly?: boolean; renderedOnly?: boolean };
const COMMON_TOGGLES: ExportToggle[] = [
  { key: "stripLinks", label: "[[links]] → text" },
  { key: "removeEmphasis", label: "Remove emphasis", sourceOnly: true },
  { key: "removeTags", label: "Remove #tags" },
];
const TEXT_TOGGLES: ExportToggle[] = [
  { key: "removeProperties", label: "Remove properties" },
  { key: "newlineAfterBlock", label: "Newline after block" },
  { key: "resolveRefsFully", label: "Resolve refs fully", renderedOnly: true },
];

const EMBED_EXPORT_NODE_LIMIT = 2_000;

const BUILT_IN_MACRO_NAMES = new Set([
  "query",
  "embed",
  "video",
  "youtube",
  "youtube-timestamp",
  "vimeo",
  "bilibili",
  "tweet",
  "twitter",
  "img",
  "cloze",
  "zotero",
  "namespace",
]);

function isBuiltInMacro(name: string): boolean {
  const n = name.toLowerCase();
  return BUILT_IN_MACRO_NAMES.has(n) || isQueryMacroName(n) || n.startsWith("zotero-");
}

interface WarmTargets {
  refs: Set<string>;
  macros: Map<string, { name: string; args: string[] }>;
}

type WarmedMacro =
  | { kind: "text"; text: string }
  | { kind: "nodes"; nodes: ExportNode[]; emptyText?: string; note?: string; truncation?: string };

function resolveExportBlockRef(uuid: string) {
  const g = resolvedBlockRefSync(uuid);
  const block = g?.blocks[0];
  if (!block) return null;
  // Match what BlockRefView shows on screen by default: renderedText keeps the
  // first line unless the export's "resolve refs fully" option is enabled.
  return { raw: visibleBody(block.raw).join("\n"), format: formatForPage(g.page) };
}

function resolveExportMacro(name: string, args: string[]) {
  if (isBuiltInMacro(name)) return null;
  const macros = graphMeta()?.macros;
  if (!macros || !Object.prototype.hasOwnProperty.call(macros, name)) return null;
  return { raw: expandTemplate(macros[name], args), format: "md" as const };
}

function macroKey(name: string, args: string[]): string {
  return JSON.stringify([name.toLowerCase(), args]);
}

function macroArg(args: string[]): string {
  return args.join(", ").trim();
}

function blockRefTarget(s: string): string | null {
  return /^\(\(([^)]+)\)\)$/.exec(s.trim())?.[1] ?? null;
}

function pageRefTarget(s: string): string | null {
  return /^\[\[([^\]]+)\]\]$/.exec(s.trim())?.[1] ?? null;
}

/** Project a bounded DTO subtree into the export serializer's read-only forest.
 * Cost is O(blocks and descendants); it does not read or write the graph. */
export function blockDtosToExportNodes(blocks: BlockDto[], format: Format): ExportNode[] {
  return blocks.map((b) => ({ raw: b.raw, format, children: blockDtosToExportNodes(b.children, format) }));
}

function pageToExportNodes(page: PageDto): ExportNode[] {
  return blockDtosToExportNodes(page.blocks, page.format ?? formatForPage(page.name));
}

function refGroupToExportNodes(group: RefGroup): ExportNode[] {
  return blockDtosToExportNodes(group.blocks, formatForPage(group.page));
}

type PageReadCache = Map<string, Promise<PageDto | null>>;

function cachedPage(cache: PageReadCache, page: string, kind: "page" | "journal"): Promise<PageDto | null> {
  const key = `${kind}\0${page}`;
  let pending = cache.get(key);
  if (!pending) {
    pending = backend().getPage(page, kind);
    cache.set(key, pending);
  }
  return pending;
}

export function queryGroupsToExportNodes(result: QueryExportResult): ExportNode[] {
  return result.groups.map((group) => ({
    raw: group.page,
    format: "md",
    children: blockDtosToExportNodes(group.blocks, formatForPage(group.page)),
  }));
}

function literalBuiltInMacroText(name: string, args: string[]): string | null {
  const n = name.toLowerCase();
  const arg = macroArg(args);
  if (/^(video|youtube|vimeo|bilibili|youtube-timestamp|tweet|twitter|img)$/.test(n)) {
    return arg.replace(/^\[\[|\]\]$/g, "");
  }
  if (n === "cloze") return arg.split(/\\\\/)[0]?.trim() ?? arg;
  if (n === "namespace" || n === "zotero" || n.startsWith("zotero-")) return arg;
  return null;
}

function collectInlineTargets(inlines: Inline[], targets: WarmTargets): void {
  for (const s of inlines) {
    switch (s.k) {
      case "emphasis":
      case "subscript":
      case "superscript":
      case "tag":
        collectInlineTargets(s.children, targets);
        break;
      case "link":
        if (s.url.type === "block_ref") targets.refs.add(s.url.v);
        if (s.label) collectInlineTargets(s.label, targets);
        break;
      case "macro": {
        targets.macros.set(macroKey(s.name, s.args), { name: s.name, args: s.args });
        if (s.name.toLowerCase() === "embed") {
          const uuid = blockRefTarget(macroArg(s.args));
          if (uuid) targets.refs.add(uuid);
        }
        break;
      }
    }
  }
}

function collectListItemTargets(item: ListItem, targets: WarmTargets): void {
  if (item.name) collectInlineTargets(item.name, targets);
  item.content.forEach((b) => collectBlockTargets(b, targets));
  item.items.forEach((child) => collectListItemTargets(child, targets));
}

function collectBlockTargets(block: Block, targets: WarmTargets): void {
  switch (block.kind) {
    case "paragraph":
    case "heading":
    case "bullet":
      collectInlineTargets(block.inline, targets);
      break;
    case "quote":
    case "custom":
      block.children.forEach((b) => collectBlockTargets(b, targets));
      break;
    case "list":
      block.items.forEach((item) => collectListItemTargets(item, targets));
      break;
    case "table":
      if (block.header) block.header.forEach((c) => collectInlineTargets(c, targets));
      block.rows.forEach((r) => r.forEach((c) => collectInlineTargets(c, targets)));
      break;
    case "footnote_def":
      collectInlineTargets(block.inline, targets);
      break;
  }
}

function collectRawTargets(raw: string, format: Format, targets: WarmTargets): void {
  try {
    parseBlock(raw, format === "org").forEach((b) => collectBlockTargets(b, targets));
    for (const property of blockRegions(raw, format).properties) {
      if (!isRenderHiddenProp(property.key)) collectInlineTargets(propertyValueInline(property, format), targets);
    }
  } catch (error) {
    // Keep the export usable if a malformed block misses pre-warm, and say so (I-9).
    reportUiFailure("export-preview", error);
  }
}

function collectNodeTargets(nodes: ExportNode[], targets: WarmTargets): void {
  for (const n of nodes) {
    collectRawTargets(n.raw, n.format ?? "md", targets);
    collectNodeTargets(n.children, targets);
  }
}

async function warmMacro(
  macro: { name: string; args: string[] },
  warmed: Map<string, WarmedMacro>,
  pages: PageReadCache,
  owner: Owner,
): Promise<void> {
  const key = macroKey(macro.name, macro.args);
  const name = macro.name.toLowerCase();
  const arg = macroArg(macro.args);
  try {
    if (name === "embed") {
      const uuid = blockRefTarget(arg);
      if (uuid) {
        const result = await readOwned(owner, backend().previewBlock(uuid, EMBED_EXPORT_NODE_LIMIT));
        if (result.kind === "stale") return;
        const preview = result.value;
        if (preview) warmed.set(key, {
          kind: "nodes",
          nodes: refGroupToExportNodes(preview.group),
          truncation: preview.truncated > 0
            ? `[embed truncated: ${preview.truncated} descendant blocks omitted]`
            : undefined,
        });
        return;
      }
      const page = pageRefTarget(arg);
      if (page) {
        const result = await readOwned(owner, cachedPage(pages, page, "page"));
        if (result.kind === "stale") return;
        const dto = result.value;
        if (dto) warmed.set(key, { kind: "nodes", nodes: pageToExportNodes(dto) });
        return;
      }
    }
    const text = literalBuiltInMacroText(name, macro.args);
    if (text != null) warmed.set(key, { kind: "text", text });
  } catch (error) {
    // Fall back to the literal macro text, and say so (I-9).
    if (owner()) reportUiFailure("export-preview", error);
  }
}

async function warmQueryMacros(
  macros: { name: string; args: string[] }[],
  warmed: Map<string, WarmedMacro>,
  owner: Owner,
): Promise<void> {
  if (!macros.length) return;
  const specs: QueryExportSpec[] = macros.map((macro) => {
    const { form } = splitTrailingMap(macroArg(macro.args));
    return {
      key: macroKey(macro.name, macro.args),
      query: form,
      // `{{tine-query}}` carries TQL; the macro name, not the text, chooses.
      ...(formFamilyForMacroName(macro.name) === "tql" ? { dialect: "tql" as const } : {}),
    };
  });
  try {
    const result = await readOwned(owner, backend().exportQuerySubtrees(specs));
    if (result.kind === "stale") return;
    const batch = result.value;
    const byKey = new Map(batch.results.map((result) => [result.key, result]));
    for (const spec of specs) {
      const result = byKey.get(spec.key);
      if (!result) {
        warmed.set(spec.key, {
          kind: "nodes",
          nodes: [],
          emptyText: "Query expansion omitted",
          truncation: `[query truncated: shared export budget supports the first ${batch.results.length} query macros; ${batch.omitted_queries} omitted]`,
        });
        continue;
      }
      warmed.set(spec.key, {
        kind: "nodes",
        nodes: queryGroupsToExportNodes(result),
        emptyText: "No results",
        truncation:
          result.total > result.shown || result.omitted_nodes > 0
            ? `[query truncated: showing first ${result.shown} of ${result.total} results${
              result.omitted_nodes > 0 ? `; ${result.omitted_nodes} descendant blocks omitted` : ""
            }]`
            : undefined,
      });
    }
  } catch (error) {
    // Leave the literal macro visible when native resolution rejects the bounded
    // request; never fall back to whole-page hydration in the WebView. The
    // failure is shown (I-9).
    if (owner()) reportUiFailure("export-preview", error);
  }
}

/** Resolve block refs and supported macros in export nodes into warmed.
 * Ref lookups run in parallel, query macros use one bounded native export
 * batch, and page embeds can read whole pages. Cost grows with refs, queries
 * and embedded page content. Failed resolutions generally leave the literal
 * macro for export; stale graph ownership stops later phases. */
export function warmExportResolutions(nodes: ExportNode[], warmed: Map<string, WarmedMacro>): Promise<void> {
  return warmExportResolutionsOwned(nodes, warmed, graphOwner());
}

async function warmExportResolutionsOwned(nodes: ExportNode[], warmed: Map<string, WarmedMacro>, owner: Owner): Promise<void> {
  const targets: WarmTargets = { refs: new Set(), macros: new Map() };
  const pages: PageReadCache = new Map();
  const seenRefs = new Set<string>(), seenMacros = new Set<string>();
  collectNodeTargets(nodes, targets);
  // Follow resolved targets too: refs inside refs and embeds share the same
  // graph-owned warming path. Bound work before each native read.
  for (let depth = 0; depth < 6; depth++) {
    for (let refDepth = 0; refDepth < 64; refDepth++) {
      const refs = [...targets.refs].filter(uuid => !seenRefs.has(uuid)).slice(0, 2000 - seenRefs.size);
      if (!refs.length) break;
      refs.forEach(uuid => seenRefs.add(uuid));
      await Promise.all(refs.map(uuid => readOwned(owner, resolveBlockBatched(uuid).catch((error) => {
        if (owner()) reportUiFailure("export-preview", error);
        return null;
      }))));
      if (!owner()) return;
      for (const uuid of refs) {
        const resolved = resolveExportBlockRef(uuid);
        if (resolved) collectRawTargets(resolved.raw, resolved.format, targets);
      }
    }
    const macros = [...targets.macros.entries()].filter(([key]) => !seenMacros.has(key)).slice(0, 2000 - seenMacros.size);
    macros.forEach(([key]) => seenMacros.add(key));
    if (!macros.length) break;
    await warmQueryMacros(macros.map(([,macro]) => macro).filter(macro => isQueryMacroName(macro.name)), warmed, owner);
    await Promise.all(macros.map(([,macro]) => macro).filter(macro => !isQueryMacroName(macro.name))
      .map(macro => warmMacro(macro, warmed, pages, owner)));
    if (!owner()) return;
    for (const [key] of macros) {
      const result = warmed.get(key);
      if (result?.kind === "nodes") collectNodeTargets(result.nodes, targets);
    }
  }
}

// "Copy / Export" modal — live-preview Text/OPML/HTML export of a block forest,
// with per-format controls mirroring OG Logseq's dialog. Read-only preview;
// Copy writes the currently selected serializer payload to the clipboard.
/** Render the shared read-only export preview for document ids or a caller's
 * already materialized forest. Copy uses the currently selected serializer;
 * only clipboard failure is reported to the user. */
export function ExportModal(): JSX.Element {
  return (
    <Show when={exportModal()}>
      {(m) => <Modal request={m()} />}
    </Show>
  );
}

function Modal(props: { request: ExportRequest }): JSX.Element {
  let root: HTMLDivElement | undefined;
  createEffect(() => {
    const unregister = registerTransientLayer({ id: "copy-export", root: () => root ?? null, dismiss: () => { closeExportModal(); return true; } });
    onCleanup(unregister);
  });
  const [opts, setOpts] = createSignal<ExportOptions>(loadOptions());
  const [format, setFormat] = createSignal<ExportFormat>("text");
  const [warmRev, setWarmRev] = createSignal(0);
  const [warming, setWarming] = createSignal(false);
  const warmedMacros = new Map<string, WarmedMacro>();
  const update = optionsUpdater(opts, setOpts, saveOptions);

  // Build the node forest once (the selection is fixed while the modal is open);
  // the preview recomputes from it as options change. Rendered mode applies the
  // typographic glyphs exactly when the app displays them (not persisted).
  const nodes = "nodes" in props.request ? props.request.nodes : exportNodesFor(props.request.ids);
  // Name the preserved syntax after the selection's actual format (GH #352).
  const sourceLabel = () => {
    const formats = new Set(nodes.map((n) => n.format ?? "md"));
    if (formats.size === 1 && formats.has("org")) return "Org";
    if (formats.size === 1) return "Markdown";
    return "Markdown/Org";
  };
  const expanding = new Set<string>();
  let expansions = 0;
  const resolveMacro = (name: string, args: string[]) => {
    const key = macroKey(name, args);
    const warmed = warmedMacros.get(key);
    if (warmed?.kind === "text") return { raw: "", format: "md" as const, text: warmed.text };
    if (warmed?.kind === "nodes") {
      // This callback reenters exportOutline, so its per-call rendering budget
      // cannot protect it. One preview owns the cycle/depth/fan-out budget.
      if (expanding.has(key) || expanding.size >= 64 || expansions >= EMBED_EXPORT_NODE_LIMIT) {
        return { raw: "", format: "md" as const, text: "[embed expansion omitted]" };
      }
      expansions++;
      expanding.add(key);
      try {
        const body = exportOutline(warmed.nodes, {
          ...opts(),
          content: "rendered",
          indent: "spaces",
          typographicGlyphs: typographyMode() === "render",
          resolveBlockRef: resolveExportBlockRef,
          resolveMacro,
        });
        const lines = [body || warmed.emptyText, warmed.note, warmed.truncation].filter((s): s is string => !!s);
        return { raw: "", format: "md" as const, text: lines.join("\n") };
      } finally { expanding.delete(key); }
    }
    return resolveExportMacro(name, args);
  };
  const payload = createMemo(() => {
    warmRev();
    expansions = 0;
    const markupOptions = {
      ...opts(), resolveBlockRef: resolveExportBlockRef,
      resolveEmbed: (name: string, args: string[]) => {
        const result = warmedMacros.get(macroKey(name, args));
        return result?.kind === "nodes" ? [
          ...result.nodes,
          ...[result.note, result.truncation].filter((text): text is string => !!text).map(raw => ({raw, children:[]})),
        ] : null;
      },
    };
    if (format() === "opml") return exportOpml(nodes, markupOptions);
    if (format() === "html") return exportHtml(nodes, markupOptions);
    return exportOutline(nodes, {
      ...markupOptions,
      typographicGlyphs: typographyMode() === "render",
      resolveBlockRef: resolveExportBlockRef,
      resolveMacro,
    });
  });

  const copy = () => {
    if (warming()) return;
    const request = exportModal();
    void writeClipboardText(payload())
      .then(() => { pushToast("Copied to clipboard", "success"); if (exportModal() === request) closeExportModal(); })
      .catch(() => pushToast("Couldn't copy: clipboard write failed.", "error"));
  };

  let disposed = false;
  onCleanup(() => {
    disposed = true;
  });

  onMount(() => {
    setWarming(true);
    void warmExportResolutionsOwned(nodes, warmedMacros, graphOwner(() => !disposed)).finally(() => {
      if (disposed) return;
      setWarmRev(warmRev() + 1);
      setWarming(false);
    });

    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Enter" && (e.metaKey || e.ctrlKey)) {
        e.preventDefault();
        copy();
      }
    };
    window.addEventListener("keydown", onKey, true);
    onCleanup(() => window.removeEventListener("keydown", onKey, true));
  });

  const visibleToggles = () => format() === "text" ? [...COMMON_TOGGLES, ...TEXT_TOGGLES] : COMMON_TOGGLES;
  const toggleDisabled = (t: ExportToggle) =>
    format() === "text"
      && ((t.sourceOnly && opts().content === "rendered") || (t.renderedOnly && opts().content !== "rendered"));
  const toggleTitle = (t: ExportToggle) => {
    if (format() === "text" && t.sourceOnly && opts().content === "rendered") return "Plain text output has no markup markers to remove";
    if (format() === "text" && t.renderedOnly && opts().content !== "rendered") return "Only applies to plain text output";
    return undefined;
  };

  const blockCount = "nodes" in props.request ? props.request.count : props.request.ids.length;
  return (
    <div class="modal-overlay" onClick={closeExportModal}>
      <div ref={root} class="export-modal" onClick={(e) => e.stopPropagation()}>
        <div class="export-head">
          Copy / export <span class="export-count">{blockCount} block{blockCount === 1 ? "" : "s"}</span>
        </div>

        <textarea class="export-preview" readonly spellcheck={false} value={payload()} />

        <div class="export-opts">
          {/* OG 1.0.0 exposes Text/OPML/HTML together at
              src/main/frontend/components/export.cljs:148-162. */}
          <div class="export-opt-row export-indent">
            <span class="export-opt-label">Format</span>
            <For each={FORMAT_STYLES}>
              {(s) => (
                <button
                  class="export-indent-btn"
                  classList={{ active: format() === s.value }}
                  onClick={() => setFormat(s.value)}
                >
                  {s.label}
                </button>
              )}
            </For>
          </div>
          <Show when={format() === "text"}>
            {/* OG keeps content indentation, property, and newline controls
                Text-only (components/export.cljs:190-206,240-260). */}
            <div class="export-opt-row export-indent">
              <span class="export-opt-label">Content</span>
              <For each={CONTENT_STYLES}>
                {(s) => (
                  <button
                    class="export-indent-btn"
                    classList={{ active: opts().content === s.value }}
                    title={s.hint}
                    onClick={() => update({ content: s.value })}
                  >
                    {s.value === "source" ? sourceLabel() : s.label}
                  </button>
                )}
              </For>
            </div>
            <div class="export-opt-row export-indent">
              <span class="export-opt-label">Indent</span>
              <For each={INDENT_STYLES}>
                {(s) => (
                  <button
                    class="export-indent-btn"
                    classList={{ active: opts().indent === s.value }}
                    title={s.hint}
                    onClick={() => update({ indent: s.value })}
                  >
                    {s.label}
                  </button>
                )}
              </For>
            </div>
          </Show>
          <label class="export-opt-row export-indent">
            <span class="export-opt-label">Level ≤</span>
            <select
              value={opts().maxDepth}
              onChange={(e) => {
                const value = e.currentTarget.value;
                update({ maxDepth: value === "all" ? "all" : Number(value) });
              }}
            >
              <For each={MAX_DEPTHS}>
                {(depth) => <option value={depth}>{depth}</option>}
              </For>
            </select>
          </label>
          <div class="export-opt-row export-toggles">
            {/* Cleanup + depth are shared by Text/OPML/HTML in OG
                (components/export.cljs:207-238,262-275). */}
            <For each={visibleToggles()}>
              {(t) => (
                <label
                  class="export-toggle"
                  classList={{ "export-toggle-moot": toggleDisabled(t) }}
                  title={toggleTitle(t)}
                >
                  <input
                    type="checkbox"
                    disabled={toggleDisabled(t)}
                    checked={opts()[t.key] as boolean}
                    onChange={(e) => update({ [t.key]: e.currentTarget.checked } as Partial<ExportOptions>)}
                  />
                  <span>{t.label}</span>
                </label>
              )}
            </For>
          </div>
        </div>

        <div class="export-foot">
          <button class="export-btn-secondary" onClick={closeExportModal}>Close</button>
          <button
            class="export-btn-primary"
            disabled={warming()}
            onClick={copy}
          >
            {warming() ? "Resolving..." : "Copy"}
          </button>
        </div>
      </div>
    </div>
  );
}
