import { For, Show, createMemo, createSignal, onCleanup, onMount, type JSX } from "solid-js";
import { exportModal, closeExportModal, pushToast, typographyMode, graphMeta } from "../ui";
import { exportNodesFor, formatForPage } from "../store";
import { backend } from "../backend";
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
} from "../editor/exportText";
import type { Block, Format, Inline, ListItem } from "../render/ast";
import type { BlockDto, PageDto, RefGroup } from "../types";

const STORE_KEY = "tine.exportOptions";

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

const INDENT_STYLES: { value: IndentStyle; label: string; hint: string }[] = [
  { value: "dashes", label: "Dashes", hint: "Logseq outline (- bullets)" },
  { value: "spaces", label: "Spaces", hint: "indent, no bullets" },
  { value: "no-indent", label: "No indent", hint: "flat, no bullets" },
];

// `sourceOnly` toggles are moot in rendered mode (the markers are already gone).
const TOGGLES: { key: keyof ExportOptions; label: string; sourceOnly?: boolean; renderedOnly?: boolean }[] = [
  { key: "stripLinks", label: "[[links]] → text" },
  { key: "removeEmphasis", label: "Remove emphasis", sourceOnly: true },
  { key: "removeTags", label: "Remove #tags" },
  { key: "removeProperties", label: "Remove properties" },
  { key: "newlineAfterBlock", label: "Newline after block" },
  { key: "resolveRefsFully", label: "Resolve refs fully", renderedOnly: true },
];

const QUERY_EXPORT_BLOCK_LIMIT = 50;
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

function queryGroupsToExportNodes(groups: RefGroup[]): { nodes: ExportNode[]; shown: number; total: number } {
  let shown = 0;
  let total = 0;
  const nodes: ExportNode[] = [];
  for (const g of groups) {
    const kept: BlockDto[] = [];
    for (const block of g.blocks) {
      total++;
      if (shown < QUERY_EXPORT_BLOCK_LIMIT) {
        kept.push(block);
        shown++;
      }
    }
    if (kept.length) {
      nodes.push({
        raw: g.page,
        format: "md",
        children: blockDtosToExportNodes(kept, formatForPage(g.page)),
      });
    }
  }
  return { nodes, shown, total };
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

async function warmMacro(macro: { name: string; args: string[] }, warmed: Map<string, WarmedMacro>): Promise<void> {
  const key = macroKey(macro.name, macro.args);
  const name = macro.name.toLowerCase();
  const arg = macroArg(macro.args);
  try {
    if (name === "embed") {
      const uuid = blockRefTarget(arg);
      if (uuid) {
        const group = await resolveBlockBatched(uuid);
        if (group) warmed.set(key, { kind: "nodes", nodes: refGroupToExportNodes(group) });
        return;
      }
      const page = pageRefTarget(arg);
      if (page) {
        const dto = await backend().getPage(page, "page");
        if (dto) warmed.set(key, { kind: "nodes", nodes: pageToExportNodes(dto) });
        return;
      }
    }
    if (name === "query") {
      const { form } = splitTrailingMap(arg);
      const groups = ADVANCED_RE.test(form) ? (await backend().runAdvancedQuery(form)).groups : await backend().runQuery(form);
      const result = queryGroupsToExportNodes(groups);
      warmed.set(key, {
        kind: "nodes",
        nodes: result.nodes,
        emptyText: "No results",
        truncation:
          result.total > result.shown
            ? `[query truncated: showing first ${result.shown} of ${result.total} results]`
            : undefined,
      });
      return;
    }
    const text = literalBuiltInMacroText(name, macro.args);
    if (text != null) warmed.set(key, { kind: "text", text });
  } catch {
    /* fall back to the literal macro text */
  }
}

async function warmExportResolutions(nodes: ExportNode[], warmed: Map<string, WarmedMacro>): Promise<void> {
  const targets: WarmTargets = { refs: new Set(), macros: new Map() };
  collectNodeTargets(nodes, targets);
  await Promise.all([...targets.refs].map((uuid) => resolveBlockBatched(uuid).catch(() => null)));
  await Promise.all([...targets.macros.values()].map((macro) => warmMacro(macro, warmed)));
}

// "Copy / Export" modal — live-preview text export of a block subtree or a
// multi-block selection, with indent-style + remove options (mirrors OG Logseq's
// export dialog). Read-only preview; Copy writes to the clipboard.
export function ExportModal(): JSX.Element {
  return (
    <Show when={exportModal()}>
      {(m) => <Modal ids={m().ids} />}
    </Show>
  );
}

function Modal(props: { ids: string[] }): JSX.Element {
  const [opts, setOpts] = createSignal<ExportOptions>(loadOptions());
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
  const text = createMemo(() => {
    warmRev();
    return exportOutline(nodes, {
      ...opts(),
      typographicGlyphs: typographyMode() === "render",
      resolveBlockRef: resolveExportBlockRef,
      resolveMacro,
    });
  });

  const copy = () => {
    if (opts().content === "rendered" && warming()) return;
    void backend().writeText(text());
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
      if (e.key === "Escape") {
        e.preventDefault();
        closeExportModal();
      } else if (e.key === "Enter" && (e.metaKey || e.ctrlKey)) {
        e.preventDefault();
        copy();
      }
    };
    window.addEventListener("keydown", onKey, true);
    onCleanup(() => window.removeEventListener("keydown", onKey, true));
  });

  const toggleDisabled = (t: (typeof TOGGLES)[number]) =>
    (t.sourceOnly && opts().content === "rendered") || (t.renderedOnly && opts().content !== "rendered");
  const toggleTitle = (t: (typeof TOGGLES)[number]) => {
    if (t.sourceOnly && opts().content === "rendered") return "Rendered text has no markup markers";
    if (t.renderedOnly && opts().content !== "rendered") return "Only applies to rendered text";
    return undefined;
  };

  const blockCount = props.ids.length;
  return (
    <div class="modal-overlay" onClick={closeExportModal}>
      <div class="export-modal" onClick={(e) => e.stopPropagation()}>
        <div class="export-head">
          Copy / export <span class="export-count">{blockCount} block{blockCount === 1 ? "" : "s"}</span>
        </div>

        <textarea class="export-preview" readonly spellcheck={false} value={text()} />

        <div class="export-opts">
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
          <div class="export-opt-row export-toggles">
            <For each={TOGGLES}>
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
          <button class="export-btn-primary" disabled={opts().content === "rendered" && warming()} onClick={copy}>
            {opts().content === "rendered" && warming() ? "Resolving..." : "Copy"}
          </button>
        </div>
      </div>
    </div>
  );
}
