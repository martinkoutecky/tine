// Block-body rendering: splits a block's text lines into paragraphs, fenced
// code blocks (syntax-highlighted), and markdown tables.

import { For, Show, createMemo, createResource, createSignal, onCleanup, type JSX } from "solid-js";
import { Dynamic } from "solid-js/web";
import { InlineText, renderInlines, renderRawHtml, renderSanitizedHtml, MathView, CopyButton } from "./inline";
import type { Block as AstBlock, ListItem as AstListItem, Format } from "./ast";
import { hiccupToHtml } from "./hiccup";
import { coarseSpanAttrs, type SpanDomAttrs } from "./spans";
import { evalCalc } from "../editor/calc";
import { toggleListItemAtIndex, doc, formatForBlock } from "../store";
import { graphMeta } from "../ui";
import { isRenderHiddenProp, isPropertyLine, propertyKeyNorm } from "./block";
import { isQuarantined, parserReady } from "./parse";
import { parseBody, stripPlanningLines } from "./facets";
import { observeNear, unobserveNear, renderedBlocks } from "../lazyObserve";
import { BeginQuery, inspectBeginQuery } from "../components/BeginQuery";

function escapeHtml(code: string): string {
  return code.replace(/&/g, "&amp;").replace(/</g, "&lt;");
}

// highlight.js/common is large — load on first code block, cache the promise.
let hljsMod: Promise<typeof import("highlight.js/lib/common").default> | null = null;
function loadHljs() {
  if (!hljsMod) hljsMod = import("highlight.js/lib/common").then((m) => m.default);
  return hljsMod;
}

// A fenced code block: renders escaped (plain) immediately, then upgrades to
// syntax-highlighted once highlight.js loads. The highlight result is memoized
// (highlightAuto is expensive) so re-renders don't re-tokenize.
function CodeBlock(props: { code: string; lang: string; spanAttrs?: SpanDomAttrs }): JSX.Element {
  const [hljs] = createResource(loadHljs);
  const html = createMemo(() => {
    const h = hljs();
    if (!h) return escapeHtml(props.code);
    try {
      if (props.lang && h.getLanguage(props.lang)) {
        return h.highlight(props.code, { language: props.lang }).value;
      }
      return h.highlightAuto(props.code).value;
    } catch {
      return escapeHtml(props.code);
    }
  });
  return (
    <pre class="code-block" {...(props.spanAttrs ?? {})}>
      <CopyButton text={props.code} title="Copy code" class="code-copy" />
      <code class="hljs" innerHTML={html()} />
    </pre>
  );
}

// A ```calc block: each input line on the left, its evaluated result on the
// right (Logseq's calculator). The raw text is unchanged — render-only.
// Exported so the editor can show the SAME live results panel while you type.
export function CalcBlock(props: { src: string; spanAttrs?: SpanDomAttrs }): JSX.Element {
  const lines = createMemo(() => evalCalc(props.src));
  return (
    <div class="calc-block" {...(props.spanAttrs ?? {})}>
      <For each={lines()}>
        {(ln, i) => (
          <>
            <div class="calc-lineno">{i() + 1}</div>
            <div class="calc-in" classList={{ "calc-error": !!ln.error }}>{ln.input || " "}</div>
            <div class="calc-out" classList={{ "calc-error": !!ln.error }}>{ln.output ?? ""}</div>
          </>
        )}
      </For>
    </div>
  );
}

// ===========================================================================
// AST block renderer (lsdoc). Renders a Tine block's parsed `Block[]`. The FIRST
// node is the header (bullet/heading); Block.tsx wraps it with marker/priority/
// heading chrome, so here we just render each block's content.
// See subagent-tasks/notes/ast-render-contract.md.
// ===========================================================================

const CALLOUT_TYPES = ["note", "tip", "important", "caution", "warning", "pinned"];

function isInlineFlow(b: AstBlock): boolean {
  return b.kind === "paragraph" || b.kind === "bullet" || b.kind === "heading";
}

/** An inline-flow block with NO visible content — empty inlines or only whitespace /
 *  line breaks. An empty block body (`- `) parses to `[empty bullet, paragraph " "]`,
 *  which `renderBlocks` would `<br>`-join into a stray second line box (≈2 lines tall) —
 *  making an empty/spacer bullet ~25px taller than its editor and jump on click (issue
 *  #12). Dropping these renders an empty block as one empty line (min-height keeps it
 *  clickable), matching the editor's height. */
function isEmptyInlineFlow(b: AstBlock): boolean {
  if (b.kind !== "paragraph" && b.kind !== "bullet" && b.kind !== "heading") return false;
  return b.inline.every(
    (i) => (i.k === "plain" && i.text.trim() === "") || i.k === "break" || i.k === "hardbreak"
  );
}

/** Render a Tine block's parsed content (`Block[]`). Consecutive inline-flow
 *  blocks (header + continuation paragraphs) are `<br>`-joined to match the old
 *  line-stacked look; block-level constructs render standalone. */
export function renderBlocks(
  blocks: AstBlock[],
  blockId?: string,
  headingLevel?: number | null,
  macroExpansion = false,
  format: Format = "md",
): JSX.Element {
  const content = (
    <For each={blocks}>
      {(b, i) => (
        <>
          <Show when={i() > 0 && isInlineFlow(b) && isInlineFlow(blocks[i() - 1])}>
            <br />
          </Show>
          {/* A `# heading` block's size applies ONLY to the heading's own line (the
              first inline-flow node), NOT to continuation constructs in the same block
              (e.g. a `> quote` under it) — matching OG. The heading level comes from
              the facet cache (facetsOf); we wrap just block 0. */}
          <Show when={i() === 0 && headingLevel && isInlineFlow(b) && b.kind !== "heading"} fallback={renderBlock(b, blockId, macroExpansion, format)}>
            <span class={`heading-text h${headingLevel}`} {...(coarseSpanAttrs(b.span) ?? {})}>{renderBlock(b, blockId, macroExpansion, format)}</span>
          </Show>
        </>
      )}
    </For>
  );
  if (!isQuarantined(blocks)) return content;
  return (
    <span class="parse-quarantine" title="This block couldn't be parsed and is shown as raw text">
      {content}
    </span>
  );
}

function renderBlock(b: AstBlock, blockId?: string, macroExpansion = false, format: Format = "md"): JSX.Element {
  switch (b.kind) {
    case "paragraph":
    case "bullet":
      return renderInlines(b.inline, blockId, true, macroExpansion, format);
    case "heading": {
      // A heading on a CONTINUATION line of a multiline block (i.e. not blocks[0],
      // which renderBlocks styles from the facet cache) must still render at its
      // own ATX size — Heading carries the size in `.size` (`.level` is nesting).
      // Clamp to h1..h6; an out-of-range/absent size renders as plain inline text.
      const sz = b.size;
      return sz != null && sz >= 1 && sz <= 6
        ? <span class={`heading-text h${sz}`} {...(coarseSpanAttrs(b.span) ?? {})}>{renderInlines(b.inline, blockId, true, macroExpansion, format)}</span>
        : renderInlines(b.inline, blockId, true, macroExpansion, format);
    }
    case "src":
      return b.lang === "calc"
        ? <CalcBlock src={b.code} spanAttrs={coarseSpanAttrs(b.span)} />
        : <CodeBlock code={b.code} lang={b.lang} spanAttrs={coarseSpanAttrs(b.span)} />;
    case "example":
      return <CodeBlock code={b.code} lang="" spanAttrs={coarseSpanAttrs(b.span)} />;
    case "quote":
      return renderQuote(b, blockId, macroExpansion, format);
    case "custom":
      return renderCustom(b, blockId, macroExpansion, format);
    case "list":
      return <AstList items={b.items} blockId={blockId} cbItems={flattenCheckboxItems(b.items)} spanAttrs={coarseSpanAttrs(b.span)} macroExpansion={macroExpansion} format={format} />;
    case "table":
      return renderTable(b, blockId, macroExpansion, format);
    case "properties":
      return renderProps(b, blockId, macroExpansion, format);
    case "hr":
      return <hr class="md-hr" {...(coarseSpanAttrs(b.span) ?? {})} />;
    case "displayed_math":
      return <MathView tex={b.text} display={true} spanAttrs={coarseSpanAttrs(b.span)} />;
    case "latex_env":
      return <MathView tex={`\\begin{${b.name}}${b.content}\\end{${b.name}}`} display={true} spanAttrs={coarseSpanAttrs(b.span)} />;
    case "raw_html":
      return renderRawHtml(b.text, coarseSpanAttrs(b.span));
    case "footnote_def":
      return (
        <div class="footnote-def" {...(coarseSpanAttrs(b.span) ?? {})}>
          <sup class="footnote-ref">{b.name}</sup> {renderInlines(b.inline, blockId, true, macroExpansion, format)}
        </div>
      );
    case "drawer":
    case "directive":
    case "comment":
      return null; // org drawers / `#+KEY:` keywords / `# comment` — not rendered
    case "hiccup":
      if (macroExpansion) {
        const html = hiccupToHtml(b.v);
        if (html !== null) return renderSanitizedHtml(html, coarseSpanAttrs(b.span));
      }
      // Direct Clojure-Hiccup remains literal; only configured macro expansions
      // opt into the bounded transcriber + sanitizer path.
      return <span class="ast-hiccup" {...(coarseSpanAttrs(b.span) ?? {})}>{b.v}</span>;
  }
}

// A `> [!NOTE]` callout (GitHub-flavoured) arrives as a `quote` whose first
// paragraph's leading plain text is `[!TYPE] …` — re-detect it. (Org `#+BEGIN_NOTE`
// is a `custom` block, handled in renderCustom.) Otherwise render a blockquote.
function renderQuote(b: Extract<AstBlock, { kind: "quote" }>, blockId?: string, macroExpansion = false, format: Format = "md"): JSX.Element {
  const first = b.children[0];
  if (first && first.kind === "paragraph") {
    const lead = first.inline[0];
    if (lead && lead.k === "plain") {
      const m = /^\[!(\w+)\]\s*(.*)$/.exec(lead.text);
      if (m) {
        const type = m[1].toLowerCase();
        // Split the lead paragraph at the FIRST soft break: the `[!TYPE]` text remainder
        // (`m[2]`) plus the inline markup BEFORE the break is the TITLE; everything after
        // begins the BODY. (lsdoc v0.2.3: `> [!NOTE] Heads **up**` keeps `**up**` in the
        // title, not the body — previously it spilled into the body.)
        const titleText = m[2];
        const afterLead = first.inline.slice(1);
        const brk = afterLead.findIndex((n) => n.k === "break");
        const titleMarkup = brk === -1 ? afterLead : afterLead.slice(0, brk);
        const bodyInlines = brk === -1 ? [] : afterLead.slice(brk + 1);
        const titleEmpty = titleText.trim() === "" && titleMarkup.length === 0;
        const bodyChildren: AstBlock[] = bodyInlines.length
          ? [{ kind: "paragraph", inline: bodyInlines }, ...b.children.slice(1)]
          : b.children.slice(1);
        return (
          <div class={`callout callout-${type}`} {...(coarseSpanAttrs(b.span) ?? {})}>
            <div class="callout-title">
              <Show when={!titleEmpty} fallback={type.toUpperCase()}>
                {titleText}
                {renderInlines(titleMarkup, blockId, true, macroExpansion, format)}
              </Show>
            </div>
            <div class="callout-body">{renderBlocks(bodyChildren, blockId, undefined, macroExpansion, format)}</div>
          </div>
        );
      }
    }
  }
  return <blockquote class="md-quote" {...(coarseSpanAttrs(b.span) ?? {})}>{renderBlocks(b.children, blockId, undefined, macroExpansion, format)}</blockquote>;
}

function renderCustom(b: Extract<AstBlock, { kind: "custom" }>, blockId?: string, macroExpansion = false, format: Format = "md"): JSX.Element {
  const type = b.name.toLowerCase();
  if (CALLOUT_TYPES.includes(type)) {
    return (
      <div class={`callout callout-${type}`} {...(coarseSpanAttrs(b.span) ?? {})}>
        <div class="callout-title">{type.toUpperCase()}</div>
        <Show when={b.children.length > 0}>
          <div class="callout-body">{renderBlocks(b.children, blockId, undefined, macroExpansion, format)}</div>
        </Show>
      </div>
    );
  }
  if (type === "quote") return <blockquote class="md-quote" {...(coarseSpanAttrs(b.span) ?? {})}>{renderBlocks(b.children, blockId, undefined, macroExpansion, format)}</blockquote>;
  return <>{renderBlocks(b.children, blockId, undefined, macroExpansion, format)}</>;
}

function renderTable(b: Extract<AstBlock, { kind: "table" }>, blockId?: string, macroExpansion = false, format: Format = "md"): JSX.Element {
  const al = (i: number) => {
    const align = b.aligns[i] ?? null;
    return align ? { "text-align": align } : undefined;
  };
  // Wrap in a horizontal-scroll container (mirrors OG's `div.table-wrapper`):
  // a wide table scrolls instead of cram-wrapping its cells down to nothing.
  return (
    <div class="md-table-wrap">
      <table class="md-table" {...(coarseSpanAttrs(b.span) ?? {})}>
        <Show when={b.header}>
          <thead>
            <tr>
              <For each={b.header!}>{(cell, i) => <th style={al(i())}>{renderInlines(cell, blockId, true, macroExpansion, format)}</th>}</For>
            </tr>
          </thead>
        </Show>
        <tbody>
          <For each={b.rows}>
            {(row) => (
              <tr>
                <For each={row}>{(cell, i) => <td style={al(i())}>{renderInlines(cell, blockId, true, macroExpansion, format)}</td>}</For>
              </tr>
            )}
          </For>
        </tbody>
      </table>
    </div>
  );
}

function renderProps(b: Extract<AstBlock, { kind: "properties" }>, blockId?: string, macroExpansion = false, format: Format = "md"): JSX.Element {
  const visible = b.props.filter(([k]) => !isRenderHiddenProp(k, graphMeta()?.block_hidden_properties ?? []));
  const fmt = formatForBlock(blockId) ?? format; // parse org property values as org
  return (
    <Show when={visible.length > 0}>
      <span class="block-properties">
        <For each={visible}>
          {([k, v]) => (
            <span class="block-property">
              <span class="block-property-key">{propertyKeyNorm(k)}</span>{" "}
              <span class="block-property-val"><InlineText text={v} format={fmt} macroExpansion={macroExpansion} /></span>
            </span>
          )}
        </For>
      </span>
    </Show>
  );
}

// The AST carries no source line (contract R12), so to toggle a checkbox we map the
// clicked item to its `[ ]`/`[x]` line in `raw` BY DOCUMENT POSITION, not by text:
// `flattenCheckboxItems` lists every checkbox item depth-first (the same order the
// `[ ]` lines appear in `raw`), and the click flips the Nth such raw line. Positional
// targeting is what makes two items with the same label toggle independently.
function flattenCheckboxItems(items: AstListItem[]): AstListItem[] {
  const out: AstListItem[] = [];
  const walk = (xs: AstListItem[]) => {
    for (const it of xs) {
      if (it.checkbox !== undefined) out.push(it);
      if (it.items.length) walk(it.items);
    }
  };
  walk(items);
  return out;
}

// An in-block list from the AST (`ListItem[]`). `cbItems` is the block-wide
// depth-first list of checkbox items, shared across nested AstLists so each
// checkbox knows its global index.
function AstList(props: { items: AstListItem[]; blockId?: string; cbItems: AstListItem[]; spanAttrs?: SpanDomAttrs; macroExpansion?: boolean; format?: Format }): JSX.Element {
  const ordered = props.items[0]?.ordered ?? false;
  return (
    <Dynamic component={ordered ? "ol" : "ul"} class="md-list" {...(props.spanAttrs ?? {})}>
      <For each={props.items}>
        {(item) => (
          <li
            class="md-list-item"
            classList={{
              "has-checkbox": item.checkbox !== undefined,
              "has-term": !!item.name && item.name.length > 0,
            }}
          >
            <Show when={item.checkbox !== undefined}>
              <span
                class="block-checkbox"
                classList={{ checked: item.checkbox === true }}
                role="checkbox"
                aria-checked={item.checkbox === true}
                onClick={(e) => {
                  e.stopPropagation();
                  if (props.blockId) toggleAstCheckbox(props.blockId, props.cbItems.indexOf(item));
                }}
              />{" "}
            </Show>
            {/* Markdown definition-list term (`term\n: def`): the item's label,
                rendered inline before its definition body (lsdoc render contract). */}
            <Show when={item.name && item.name.length > 0}>
              <span class="md-list-term">{renderInlines(item.name!, props.blockId, true, props.macroExpansion ?? false, props.format)}</span>{" "}
            </Show>
            {renderBlocks(item.content, props.blockId, undefined, props.macroExpansion ?? false, props.format)}
            <Show when={item.items.length > 0}>
              <AstList items={item.items} blockId={props.blockId} cbItems={props.cbItems} macroExpansion={props.macroExpansion} format={props.format} />
            </Show>
          </li>
        )}
      </For>
    </Dynamic>
  );
}

/** The body's render blocks: the WHOLE re-bulleted raw parsed by lsdoc (one parse,
 *  shared with the facet cache), MINUS the nodes whose chrome Block.tsx draws —
 *  `properties` (chips) and standalone SCHEDULED/DEADLINE lines (date badges). The
 *  marker/priority/heading are facet FIELDS on `blocks[0]` (not inline), so the body
 *  renders markerless automatically. A planning `Timestamp` only ever appears on a
 *  standalone planning line (lsdoc never makes one mid-text), so dropping any block
 *  that contains one drops exactly the badge lines. */
function bodyBlocks(raw: string, isOrg: boolean): AstBlock[] {
  const parsed = parseBody(raw, isOrg ? "org" : "md");
  if (isQuarantined(parsed)) return parsed;
  // Drop `properties` (chips in chrome), then remove only the parser-confirmed
  // whole-line planning timestamp from its inline flow. A trailing body line can
  // share that Paragraph (#75), so filtering the entire block would eat content.
  return stripPlanningLines(parsed, raw).filter(
    (b) => b.kind !== "properties" && !isEmptyInlineFlow(b)
  );
}

function renderBody(raw: string, format: Format, blockId?: string, headingLevel?: number | null, macroExpansion = false): JSX.Element {
  const blocks = bodyBlocks(raw, format === "org");
  const beginQuery = inspectBeginQuery(raw, format, blocks);
  return beginQuery
    ? <BeginQuery match={beginQuery} currentPage={blockId ? doc.byId[blockId]?.page : undefined} />
    : renderBlocks(blocks, blockId, headingLevel, macroExpansion, format);
}

/** Render a block's body. Parses the WHOLE block's `raw` (re-bulleted like OG, via
 *  `parseBody`) SYNCHRONOUSLY into lsdoc's AST and renders it (`bodyBlocks` →
 *  `renderBlocks`), skipping the property/planning nodes that Block.tsx renders as
 *  chrome. So lsdoc is the single source for the body AND (via the facet cache) the
 *  header chrome — no `blockView` re-derivation.
 *
 *  The parser is initialized once before first paint (main.tsx / capture.tsx). The
 *  `<Show>` fallback renders the raw text literally and only triggers if the wasm
 *  parser failed to load (degraded mode), so content is never silently blank. */
export function AstBody(props: { raw: string; blockId?: string; format?: Format; headingLevel?: number | null; macroExpansion?: boolean }): JSX.Element {
  // Deferred (off-screen) placeholder text: raw minus property lines (cheap, no
  // parse) — a good height proxy, replaced by the real render once near.
  const placeholder = createMemo(() => props.raw.split("\n").filter((l) => !isPropertyLine(l)).join("\n"));
  // P1 block-render virtualization (see docs/adr): defer the synchronous parse +
  // AST→DOM build until the block is near the viewport. Render-once-keep: once a
  // block has rendered (latched by id in `renderedBlocks`) it renders eagerly
  // forever — no second placeholder↔real transition, so zero scroll-height churn.
  const id = props.blockId;
  const [near, setNear] = createSignal(id == null || renderedBlocks.has(id));
  let deferredEl: Element | undefined;
  const observe = (el: Element) => {
    deferredEl = el;
    observeNear(el, () => {
      if (id != null) renderedBlocks.add(id);
      setNear(true);
    });
  };
  onCleanup(() => {
    if (deferredEl) unobserveNear(deferredEl);
  });
  return (
    <Show
      when={near()}
      fallback={
        <span
          class="ast-fallback ast-deferred"
          style={estimateBodyReserve(placeholder().split("\n"), props.headingLevel ?? null)}
          ref={observe}
        >
          {placeholder()}
        </span>
      }
    >
      <Show when={parserReady()} fallback={<span class="ast-fallback">{placeholder()}</span>}>
        {renderBody(props.raw, props.format ?? "md", props.blockId, props.headingLevel, props.macroExpansion ?? false)}
      </Show>
    </Show>
  );
}

/** A cheap `min-height` for the deferred (raw-text) placeholder, so the one-time
 *  first render-in doesn't visibly jump for constructs whose raw text is a poor
 *  height proxy. Prose / lists / fenced code / tables: raw line-count ≈ rendered,
 *  so no reserve. Headings render larger than their body-size raw line; display
 *  math (`$$…$$`) and media embeds render much taller than their single raw line.
 *  Pure — NO DOM measurement (a re-measure loop is the content-visibility trap
 *  that was reverted in e2cdfc7). Values are approximate; the goal is to shrink,
 *  not eliminate, the first-view delta. */
export function estimateBodyReserve(lines: string[], headingLevel: number | null): JSX.CSSProperties | undefined {
  if (headingLevel != null) {
    // h1 ≈ 2.1em … h6 ≈ 1.3em, tracking the heading scale in app.css.
    const em = Math.max(1.3, 2.1 - (headingLevel - 1) * 0.15);
    return { "min-height": `${em.toFixed(2)}em` };
  }
  for (const l of lines) {
    const t = l.trim();
    // Display math renders as a centered block ~2.4em tall from one raw `$$` line.
    if (t.startsWith("$$")) return { "min-height": "2.4em" };
    // Image/video/audio embed renders a media box far taller than `![](…)`. The
    // true height is decode-driven (and already reflows on load today), so this is
    // a rough typical to narrow the gap, not an exact reservation.
    if (/!\[[^\]]*\]\([^)]+\)/.test(t)) return { "min-height": "6em" };
  }
  return undefined;
}

// Flip the `cbIndex`-th checkbox of the block: find the cbIndex-th `[ ]`/`[x]`
// list line in `raw` (document order) and toggle exactly that line. No text match,
// so duplicate labels and `**markup**` in the item never mis-target.
function toggleAstCheckbox(blockId: string, cbIndex: number) {
  if (cbIndex < 0) return;
  const node = doc.byId[blockId];
  if (!node) return;
  const lines = node.raw.split("\n");
  const re = /^\s*(?:[-+*]|\d+[.)])\s+\[[ xX]\]\s+/;
  let seen = -1;
  for (let i = 0; i < lines.length; i++) {
    if (re.test(lines[i])) {
      if (++seen === cbIndex) {
        toggleListItemAtIndex(blockId, i);
        return;
      }
    }
  }
}
