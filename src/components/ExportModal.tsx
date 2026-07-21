import { For, Show, createEffect, createMemo, createSignal, onCleanup, onMount, type JSX } from "solid-js";
import { exportModal, closeExportModal, pushToast, typographyMode, graphMeta } from "../ui";
import { exportNodesFor, formatForPage } from "../store";
import { backend } from "../backend";
import { writeClipboardText } from "../clipboard";
import { resolveBlockBatched, resolvedBlockRefSync } from "../resolveBatch";
import { expandTemplate } from "../render/inline";
import { visibleBody } from "../render/block";
import { parseBlock } from "../render/parse";
import { splitTrailingMap } from "../editor/edn";
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

const CONTENT_STYLES: { value: ExportContent; label: string; hint: string }[] = [
  { value: "rendered", label: "Rendered", hint: "the text as displayed — glyphs (→ –), no markup markers" },
  { value: "source", label: "Source", hint: "the raw Markdown/Org text" },
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
const ADVANCED_RE = /\[\s*:find|:where|:find/;

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
  return BUILT_IN_MACRO_NAMES.has(n) || n.startsWith("zotero-");
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

function blockDtosToExportNodes(blocks: BlockDto[], format: Format): ExportNode[] {
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
  } catch {
    /* keep export usable if a malformed block misses pre-warm */
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
): Promise<void> {
  const key = macroKey(macro.name, macro.args);
  const name = macro.name.toLowerCase();
  const arg = macroArg(macro.args);
  try {
    if (name === "embed") {
      const uuid = blockRefTarget(arg);
      if (uuid) {
        const preview = await backend().previewBlock(uuid, EMBED_EXPORT_NODE_LIMIT);
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
        const dto = await cachedPage(pages, page, "page");
        if (dto) warmed.set(key, { kind: "nodes", nodes: pageToExportNodes(dto) });
        return;
      }
    }
    const text = literalBuiltInMacroText(name, macro.args);
    if (text != null) warmed.set(key, { kind: "text", text });
  } catch {
    /* fall back to the literal macro text */
  }
}

async function warmQueryMacros(
  macros: { name: string; args: string[] }[],
  warmed: Map<string, WarmedMacro>,
): Promise<void> {
  if (!macros.length) return;
  const specs: QueryExportSpec[] = macros.map((macro) => {
    const { form } = splitTrailingMap(macroArg(macro.args));
    return {
      key: macroKey(macro.name, macro.args),
      query: form,
      advanced: ADVANCED_RE.test(form),
    };
  });
  try {
    const batch = await backend().exportQuerySubtrees(specs);
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
  } catch {
    // Leave the literal macro visible when native resolution rejects the bounded
    // request; never fall back to whole-page hydration in the WebView.
  }
}

export async function warmExportResolutions(nodes: ExportNode[], warmed: Map<string, WarmedMacro>): Promise<void> {
  const targets: WarmTargets = { refs: new Set(), macros: new Map() };
  const pages: PageReadCache = new Map();
  collectNodeTargets(nodes, targets);
  await Promise.all([...targets.refs].map((uuid) => resolveBlockBatched(uuid).catch(() => null)));
  const macros = [...targets.macros.values()];
  await warmQueryMacros(
    macros.filter((macro) => macro.name.toLowerCase() === "query"),
    warmed,
  );
  // Page embeds are intentionally whole-page exports, but run them after the
  // globally bounded query batch so their PageDto cache cannot overlap query
  // source-page hydration (which no longer uses getPage at all).
  await Promise.all(
    macros
      .filter((macro) => macro.name.toLowerCase() !== "query")
      .map((macro) => warmMacro(macro, warmed, pages)),
  );
}

// "Copy / Export" modal — live-preview Text/OPML/HTML export of a block forest,
// with per-format controls mirroring OG Logseq's dialog. Read-only preview;
// Copy writes the currently selected serializer payload to the clipboard.
export function ExportModal(): JSX.Element {
  return (
    <Show when={exportModal()}>
      {(m) => <Modal ids={m().ids} />}
    </Show>
  );
}

function Modal(props: { ids: string[] }): JSX.Element {
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
  const update = (patch: Partial<ExportOptions>) => {
    const next = { ...opts(), ...patch };
    setOpts(next);
    saveOptions(next);
  };

  // Build the node forest once (the selection is fixed while the modal is open);
  // the preview recomputes from it as options change. Rendered mode applies the
  // typographic glyphs exactly when the app displays them (not persisted).
  const nodes = exportNodesFor(props.ids);
  const resolveMacro = (name: string, args: string[]) => {
    const warmed = warmedMacros.get(macroKey(name, args));
    if (warmed?.kind === "text") return { raw: "", format: "md" as const, text: warmed.text };
    if (warmed?.kind === "nodes") {
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
    }
    return resolveExportMacro(name, args);
  };
  const payload = createMemo(() => {
    warmRev();
    if (format() === "opml") return exportOpml(nodes, opts());
    if (format() === "html") return exportHtml(nodes, opts());
    return exportOutline(nodes, {
      ...opts(),
      typographicGlyphs: typographyMode() === "render",
      resolveBlockRef: resolveExportBlockRef,
      resolveMacro,
    });
  });

  const copy = () => {
    if (format() === "text" && opts().content === "rendered" && warming()) return;
    void writeClipboardText(payload());
    pushToast("Copied to clipboard", "success");
    closeExportModal();
  };

  let disposed = false;
  onCleanup(() => {
    disposed = true;
  });

  onMount(() => {
    setWarming(true);
    void warmExportResolutions(nodes, warmedMacros).finally(() => {
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
    if (format() === "text" && t.sourceOnly && opts().content === "rendered") return "Rendered text has no markup markers";
    if (format() === "text" && t.renderedOnly && opts().content !== "rendered") return "Only applies to rendered text";
    return undefined;
  };

  const blockCount = props.ids.length;
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
                    {s.label}
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
            disabled={format() === "text" && opts().content === "rendered" && warming()}
            onClick={copy}
          >
            {format() === "text" && opts().content === "rendered" && warming() ? "Resolving..." : "Copy"}
          </button>
        </div>
      </div>
    </div>
  );
}
