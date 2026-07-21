// Configurable text export for a block subtree / multi-block selection — Tine's
// take on OG Logseq's "Copy / Export" modal (handler/export/text.cljs). The core
// is pure (operates on an ExportNode tree) so it's unit-testable; the store
// builds the tree from the live doc. The inline "remove" transforms are
// pragmatic regexes (not a full inline parse) — enough for the common cases the
// modal offers, matching OG's option set within reason.

import { isPropertyLine } from "../render/block";
import { renderedBlockText, type RenderedTextOptions } from "../render/renderedText";
import type { Format } from "../render/ast";

// dashes  = Logseq outline: `\t`×level + "- " + text (paste back into Logseq).
// spaces  = indentation preserved with spaces, NO bullet (portable indented text).
// no-indent = flat: no indentation, no bullet (plain lines).
export type IndentStyle = "dashes" | "spaces" | "no-indent";
export type MaxDepth = "all" | number;

// rendered = the text as displayed (glyphs, no markup markers) — lsdoc-AST
//            flattening via render/renderedText.ts, never a regex re-scan.
// source   = the raw markdown/org, with the regex "remove" transforms below.
export type ExportContent = "rendered" | "source";

export interface ExportOptions {
  content: ExportContent;
  indent: IndentStyle;
  maxDepth?: MaxDepth;
  stripLinks: boolean; // [[Foo]] -> Foo
  removeEmphasis: boolean; // **/__/*/_/~~/== markers dropped (source mode only — rendered has none)
  removeTags: boolean; // #tag and #[[tag]] removed
  removeProperties: boolean; // drop `key:: value` lines
  newlineAfterBlock: boolean; // blank line after each block
  /** Apply `->`→`→` glyphs in rendered mode; the modal sets this from the app's
   *  typography mode each time (not persisted — it must match what you see). */
  typographicGlyphs?: boolean;
  /** Render trivial inline math scripts as unicode (`E=mc^2` → `E=mc²`); lossy and off by default. */
  mathAsUnicode?: boolean;
  /** Render all lines of resolved block refs instead of the inline first-line view. */
  resolveRefsFully?: boolean;
  resolveBlockRef?: RenderedTextOptions["resolveBlockRef"];
  resolveMacro?: RenderedTextOptions["resolveMacro"];
}

export const DEFAULT_EXPORT_OPTIONS: ExportOptions = {
  content: "rendered",
  indent: "dashes",
  // OG 1.0.0 defaults its `:keep-only-level<=N` export option to `:all`
  // (handler/export/common.cljs:45-54).
  maxDepth: "all",
  stripLinks: false,
  removeEmphasis: false,
  removeTags: false,
  removeProperties: false,
  newlineAfterBlock: false,
  mathAsUnicode: false,
  resolveRefsFully: false,
};

export interface ExportNode {
  raw: string;
  format?: Format; // md when absent
  children: ExportNode[];
}

/** Apply the inline "remove" transforms to one content line. */
function stripInline(text: string, opts: ExportOptions): string {
  let s = text;
  if (opts.removeTags) {
    s = s.replace(/#\[\[[^\]]*\]\]/g, ""); // #[[Foo Bar]]
    s = s.replace(/(^|\s)#[\w/-]+/g, "$1"); // #tag (keep the boundary char)
    s = s.replace(/[ \t]{2,}/g, " "); // tidy gaps left by removed tags
  }
  if (opts.stripLinks) {
    s = s.replace(/\[\[([^\]]*)\]\]/g, "$1"); // [[Foo]] -> Foo
  }
  if (opts.removeEmphasis) {
    s = s.replace(/(\*\*|__)(.*?)\1/g, "$2"); // bold
    s = s.replace(/(\*|_)(.*?)\1/g, "$2"); // italic
    s = s.replace(/~~(.*?)~~/g, "$1"); // strikethrough
    s = s.replace(/==(.*?)==/g, "$1"); // highlight
  }
  return s;
}

/** One block's export lines: rendered (AST flattening) or source (raw + regex
 *  transforms). Both honor removeProperties/stripLinks/removeTags; emphasis
 *  markers only exist in source. */
function blockExportLines(n: ExportNode, opts: ExportOptions): string[] {
  if (opts.content === "rendered") {
    return renderedBlockText(n.raw, n.format ?? "md", {
      typographicGlyphs: opts.typographicGlyphs ?? false,
      stripLinks: opts.stripLinks,
      removeTags: opts.removeTags,
      removeProperties: opts.removeProperties,
      resolveBlockRefsFully: opts.resolveRefsFully ?? false,
      mathAsUnicode: opts.mathAsUnicode ?? false,
      resolveBlockRef: opts.resolveBlockRef,
      resolveMacro: opts.resolveMacro,
    }).split("\n");
  }
  let lines = n.raw.split("\n");
  if (opts.removeProperties) lines = lines.filter((l) => !isPropertyLine(l));
  return lines.map((l) => stripInline(l, opts));
}

/** Serialize an export-node forest to text per `opts`. */
export function exportOutline(nodes: ExportNode[], opts: ExportOptions): string {
  const out: string[] = [];
  const walk = (n: ExportNode, level: number) => {
    let lines = blockExportLines(n, opts);
    // Trailing blank lines left by stripping add nothing; keep at least one line.
    while (lines.length > 1 && lines[lines.length - 1].trim() === "") lines.pop();
    const first = lines[0] ?? "";

    if (opts.indent === "dashes") {
      const tabs = "\t".repeat(level);
      out.push(`${tabs}- ${first}`.replace(/\s+$/, ""));
      for (const l of lines.slice(1)) out.push(l === "" ? "" : `${tabs}  ${l}`);
    } else if (opts.indent === "spaces") {
      const pad = "  ".repeat(level);
      out.push(`${pad}${first}`.replace(/\s+$/, ""));
      for (const l of lines.slice(1)) out.push(l === "" ? "" : `${pad}${l}`);
    } else {
      out.push(first.replace(/\s+$/, ""));
      for (const l of lines.slice(1)) out.push(l);
    }

    if (opts.newlineAfterBlock) out.push("");
    // Export roots count as depth 1, while `level` remains the zero-based
    // indentation level. OG 1.0.0 applies its shared depth filter before
    // serialization, retaining accepted headings and suppressing their deeper
    // descendants (handler/export/common.cljs:547-565).
    const maxDepth = opts.maxDepth ?? "all";
    if (maxDepth === "all" || level + 1 < maxDepth) {
      for (const c of n.children) walk(c, level + 1);
    }
  };
  for (const n of nodes) walk(n, 0);
  while (out.length && out[out.length - 1] === "") out.pop();
  return out.join("\n");
}
