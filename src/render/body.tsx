// Block-body rendering: splits a block's text lines into paragraphs, fenced
// code blocks (syntax-highlighted), and markdown tables.

import { For, Show, createMemo, createResource, type JSX } from "solid-js";
import { Dynamic } from "solid-js/web";
import { InlineText, renderInlines, renderRawHtml, MathView, astText } from "./inline";
import type { Format } from "./parseInline";
import type { Block as AstBlock, ListItem as AstListItem } from "./ast";
import { backend } from "../backend";
import { evalCalc } from "../editor/calc";
import { toggleListItem, doc } from "../store";
import { parseBlockBatched } from "./astParse";

type Align = "left" | "center" | "right" | null;

type BodySeg =
  | { kind: "lines"; lines: string[] }
  | { kind: "code"; lang: string; code: string }
  | { kind: "calc"; code: string }
  | { kind: "table"; rows: string[][]; aligns: Align[] }
  | { kind: "hr" }
  | { kind: "quote"; lines: string[] }
  | { kind: "callout"; kind2: string; title: string; lines: string[] }
  | { kind: "list"; items: string[] };

// An in-block markdown list line: `+`/`*`/ordered marker (NOT `-`, which is the
// outliner's own block bullet and would be parsed as a child block — matching OG,
// where only `-`/`*`-in-org are block bullets). This is how a tickable checklist
// round-trips with OG/mobile: a `+ [ ]` list inside ONE bullet's content.
// In-block plain-list bullets are format-specific, because the unusable marker
// differs: in Markdown a leading `-` is the OUTLINE bullet (its own block), so
// in-block lists use `+`/`*`; in Org a leading `*` is a HEADLINE, so org plain
// lists use `-`/`+` (org-mode + Logseq). Numbered (`1.`/`1)`) works in both.
const LIST_RE_MD = /^(\s*)([+*]|\d+[.)])\s+(.*)$/;
const LIST_RE_ORG = /^(\s*)([-+]|\d+[.)])\s+(.*)$/;
function listRe(format?: Format): RegExp {
  return format === "org" ? LIST_RE_ORG : LIST_RE_MD;
}

function isTableRow(line: string): boolean {
  return /^\s*\|.*\|\s*$/.test(line);
}
function isTableSep(line: string): boolean {
  // Accept org's column-junction `+` (`|---+---|`) as well as markdown's `|---|`.
  return /^\s*\|?[\s:|+-]+\|?\s*$/.test(line) && line.includes("-");
}
function isHr(line: string): boolean {
  return /^\s*([-*_])\1{2,}\s*$/.test(line);
}
function isQuote(line: string): boolean {
  return /^\s*>\s?/.test(line);
}
function stripQuote(line: string): string {
  return line.replace(/^\s*>\s?/, "");
}
// `> [!NOTE] optional title` opens a callout; type is lowercased.
const CALLOUT_RE = /^\[!(\w+)\]\s*(.*)$/;

/** Per-column alignment from a table separator row (`:--`, `--:`, `:-:`). */
function parseAligns(sep: string): Align[] {
  return splitRow(sep).map((c) => {
    const l = c.startsWith(":");
    const r = c.endsWith(":");
    return l && r ? "center" : r ? "right" : l ? "left" : null;
  });
}

export function segmentBody(lines: string[], format?: Format): BodySeg[] {
  const LIST_RE = listRe(format);
  const segs: BodySeg[] = [];
  let buf: string[] = [];
  const flush = () => {
    if (buf.length) segs.push({ kind: "lines", lines: buf });
    buf = [];
  };

  for (let i = 0; i < lines.length; i++) {
    const line = lines[i];
    const fence = /^```(\S*)\s*$/.exec(line.trim());
    if (fence) {
      flush();
      const lang = fence[1] || "";
      const code: string[] = [];
      i++;
      while (i < lines.length && !/^```\s*$/.test(lines[i].trim())) {
        code.push(lines[i]);
        i++;
      }
      const joined = code.join("\n");
      segs.push(lang === "calc" ? { kind: "calc", code: joined } : { kind: "code", lang, code: joined });
      continue;
    }
    // table: a run of pipe rows
    if (isTableRow(line) && i + 1 < lines.length && isTableSep(lines[i + 1])) {
      flush();
      const rows: string[][] = [];
      const header = splitRow(line);
      const aligns = parseAligns(lines[i + 1]);
      i++; // skip separator
      const body: string[][] = [];
      while (i + 1 < lines.length && isTableRow(lines[i + 1])) {
        body.push(splitRow(lines[i + 1]));
        i++;
      }
      rows.push(header, ...body);
      segs.push({ kind: "table", rows, aligns });
      continue;
    }
    // horizontal rule
    if (isHr(line)) {
      flush();
      segs.push({ kind: "hr" });
      continue;
    }
    // blockquote / callout: a run of `>`-prefixed lines
    if (isQuote(line)) {
      flush();
      const qlines: string[] = [];
      while (i < lines.length && isQuote(lines[i])) {
        qlines.push(stripQuote(lines[i]));
        i++;
      }
      i--; // for-loop will ++
      const co = CALLOUT_RE.exec(qlines[0] ?? "");
      if (co) {
        segs.push({ kind: "callout", kind2: co[1].toLowerCase(), title: co[2].trim(), lines: qlines.slice(1) });
      } else {
        segs.push({ kind: "quote", lines: qlines });
      }
      continue;
    }
    // org-mode admonition block: `#+BEGIN_X` … `#+END_X` (one multi-line block,
    // line-leading, type case-insensitive). Logseq renders note/tip/important/
    // caution/warning/pinned as colored callouts, QUOTE as a plain blockquote, and
    // anything else (center, …) as plain content. The raw `#+BEGIN/#+END` stay in
    // the block's `raw`, so this is render-only and round-trips byte-for-byte.
    const beg = /^\s*#\+begin_(\w+)\s*(.*)$/i.exec(line);
    if (beg) {
      const type = beg[1].toLowerCase();
      const endRe = new RegExp(`^\\s*#\\+end_${type}\\s*$`, "i");
      const inner: string[] = [];
      let j = i + 1;
      while (j < lines.length && !endRe.test(lines[j])) {
        inner.push(lines[j]);
        j++;
      }
      if (j < lines.length) {
        // matched `#+END_<type>` — consume the whole block
        flush();
        i = j; // for-loop ++ steps past the END line
        if (["note", "tip", "important", "caution", "warning", "pinned"].includes(type)) {
          segs.push({ kind: "callout", kind2: type, title: beg[2].trim(), lines: inner });
        } else if (type === "quote") {
          segs.push({ kind: "quote", lines: inner });
        } else if (type === "src" || type === "example") {
          // org source/example block → fenced code (lang from `#+BEGIN_SRC lang`).
          segs.push({ kind: "code", lang: type === "src" ? beg[2].trim() : "", code: inner.join("\n") });
        } else {
          segs.push({ kind: "lines", lines: inner }); // center / unknown → plain
        }
        continue;
      }
      // no matching END — fall through and treat as an ordinary line
    }
    // in-block markdown list (`+`/`*`/ordered) — a run of list lines
    if (LIST_RE.test(line)) {
      flush();
      const items: string[] = [];
      while (i < lines.length && LIST_RE.test(lines[i])) {
        items.push(lines[i]);
        i++;
      }
      i--; // for-loop will ++
      segs.push({ kind: "list", items });
      continue;
    }
    buf.push(line);
  }
  flush();
  return segs;
}

interface ListNode {
  ordered: boolean;
  items: ListItemNode[];
}
interface ListItemNode {
  raw: string; // the exact source line (for round-trip-safe checkbox toggle)
  checkbox: "unchecked" | "checked" | null;
  text: string; // inline text after the marker (+ checkbox)
  children: ListNode | null;
}

/** Parse a run of `+`/`*`/ordered list lines into a nested tree by indentation. */
export function parseList(lines: string[], format?: Format): ListNode {
  const LIST_RE = listRe(format);
  const parsed = lines.map((raw) => {
    const m = LIST_RE.exec(raw)!;
    let rest = m[3];
    let checkbox: ListItemNode["checkbox"] = null;
    const cb = /^\[([ xX])\]\s+(.*)$/.exec(rest);
    if (cb) {
      checkbox = cb[1] === " " ? "unchecked" : "checked";
      rest = cb[2];
    }
    return { indent: m[1].length, ordered: /\d/.test(m[2]), raw, checkbox, text: rest };
  });
  const root: ListNode = { ordered: parsed[0].ordered, items: [] };
  const stack: { indent: number; list: ListNode }[] = [{ indent: parsed[0].indent, list: root }];
  for (const p of parsed) {
    while (stack.length > 1 && p.indent < stack[stack.length - 1].indent) stack.pop();
    let top = stack[stack.length - 1];
    if (p.indent > top.indent) {
      // deeper than the current level → a child list under the last item
      const parent = top.list.items[top.list.items.length - 1];
      const child: ListNode = { ordered: p.ordered, items: [] };
      if (parent) parent.children = child;
      stack.push({ indent: p.indent, list: child });
      top = stack[stack.length - 1];
    }
    top.list.items.push({ raw: p.raw, checkbox: p.checkbox, text: p.text, children: null });
  }
  return root;
}

function splitRow(line: string): string[] {
  return line
    .trim()
    .replace(/^\|/, "")
    .replace(/\|$/, "")
    .split("|")
    .map((c) => c.trim());
}

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
function CodeBlock(props: { code: string; lang: string }): JSX.Element {
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
    <pre class="code-block">
      <button
        class="code-copy"
        title="Copy code"
        onClick={(e) => {
          e.stopPropagation();
          void backend().writeText(props.code);
        }}
      >
        Copy
      </button>
      <code class="hljs" innerHTML={html()} />
    </pre>
  );
}

// A ```calc block: each input line on the left, its evaluated result on the
// right (Logseq's calculator). The raw text is unchanged — render-only.
// Exported so the editor can show the SAME live results panel while you type.
export function CalcBlock(props: { src: string }): JSX.Element {
  const lines = createMemo(() => evalCalc(props.src));
  return (
    <div class="calc-block">
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

/** An in-block markdown list (styled distinctly from outline bullets), with
 *  tickable `[ ]`/`[x]` checkboxes that toggle the matching line in the block. */
function MdList(props: { node: ListNode; blockId?: string; format?: Format }): JSX.Element {
  return (
    <Dynamic component={props.node.ordered ? "ol" : "ul"} class="md-list">
      <For each={props.node.items}>
        {(item) => (
          <li class="md-list-item" classList={{ "has-checkbox": item.checkbox !== null }}>
            <Show when={item.checkbox !== null}>
              <span
                class="block-checkbox"
                classList={{ checked: item.checkbox === "checked" }}
                role="checkbox"
                aria-checked={item.checkbox === "checked"}
                onClick={(e) => {
                  e.stopPropagation();
                  if (props.blockId) toggleListItem(props.blockId, item.raw);
                }}
              />{" "}
            </Show>
            <InlineText text={item.text} format={props.format} />
            <Show when={item.children}>
              <MdList node={item.children!} blockId={props.blockId} format={props.format} />
            </Show>
          </li>
        )}
      </For>
    </Dynamic>
  );
}

export function BodyContent(props: { lines: string[]; blockId?: string; format?: Format }): JSX.Element {
  return (
    <For each={segmentBody(props.lines, props.format)}>
      {(seg) => {
        if (seg.kind === "code") {
          return <CodeBlock code={seg.code} lang={seg.lang} />;
        }
        if (seg.kind === "calc") {
          return <CalcBlock src={seg.code} />;
        }
        if (seg.kind === "table") {
          const [head, ...body] = seg.rows;
          const al = (i: number) => (seg.aligns[i] ? { "text-align": seg.aligns[i]! } : undefined);
          return (
            <table class="md-table">
              <thead>
                <tr>
                  <For each={head}>{(c, i) => <th style={al(i())}><InlineText text={c} format={props.format} /></th>}</For>
                </tr>
              </thead>
              <tbody>
                <For each={body}>
                  {(row) => (
                    <tr>
                      <For each={row}>{(c, i) => <td style={al(i())}><InlineText text={c} format={props.format} /></td>}</For>
                    </tr>
                  )}
                </For>
              </tbody>
            </table>
          );
        }
        if (seg.kind === "list") {
          return <MdList node={parseList(seg.items, props.format)} blockId={props.blockId} format={props.format} />;
        }
        if (seg.kind === "hr") {
          return <hr class="md-hr" />;
        }
        if (seg.kind === "quote") {
          return (
            <blockquote class="md-quote">
              <For each={seg.lines}>
                {(line, i) => (
                  <>
                    <Show when={i() > 0}>
                      <br />
                    </Show>
                    <InlineText text={line} format={props.format} />
                  </>
                )}
              </For>
            </blockquote>
          );
        }
        if (seg.kind === "callout") {
          return (
            <div class={`callout callout-${seg.kind2}`}>
              <div class="callout-title">{seg.title || seg.kind2.toUpperCase()}</div>
              <Show when={seg.lines.length > 0}>
                <div class="callout-body">
                  <For each={seg.lines}>
                    {(line, i) => (
                      <>
                        <Show when={i() > 0}>
                          <br />
                        </Show>
                        <InlineText text={line} format={props.format} />
                      </>
                    )}
                  </For>
                </div>
              </Show>
            </div>
          );
        }
        return (
          <For each={seg.lines}>
            {(line, i) => (
              <>
                <Show when={i() > 0}>
                  <br />
                </Show>
                <InlineText text={line} blockId={props.blockId} format={props.format} />
              </>
            )}
          </For>
        );
      }}
    </For>
  );
}

// ===========================================================================
// AST block renderer (lsdoc). Renders a Tine block's parsed `Block[]` — the
// replacement for segmentBody. The FIRST node is the header (bullet/heading);
// Block.tsx wraps it with marker/priority/heading chrome, so here we just render
// each block's content. See subagent-tasks/notes/ast-render-contract.md.
// ===========================================================================

const CALLOUT_TYPES = ["note", "tip", "important", "caution", "warning", "pinned"];
// Block properties never shown as chips (id/uuid/collapsed + Logseq internals).
const HIDDEN_PROPS = new Set(["id", "collapsed", "heading", "logseq.order-list-type"]);

function isInlineFlow(b: AstBlock): boolean {
  return b.kind === "paragraph" || b.kind === "bullet" || b.kind === "heading";
}

/** Render a Tine block's parsed content (`Block[]`). Consecutive inline-flow
 *  blocks (header + continuation paragraphs) are `<br>`-joined to match the old
 *  line-stacked look; block-level constructs render standalone. */
export function renderBlocks(blocks: AstBlock[], blockId?: string): JSX.Element {
  return (
    <For each={blocks}>
      {(b, i) => (
        <>
          <Show when={i() > 0 && isInlineFlow(b) && isInlineFlow(blocks[i() - 1])}>
            <br />
          </Show>
          {renderBlock(b, blockId)}
        </>
      )}
    </For>
  );
}

function renderBlock(b: AstBlock, blockId?: string): JSX.Element {
  switch (b.kind) {
    case "paragraph":
    case "bullet":
    case "heading":
      return renderInlines(b.inline, blockId);
    case "src":
      return b.lang === "calc" ? <CalcBlock src={b.code} /> : <CodeBlock code={b.code} lang={b.lang} />;
    case "example":
      return <CodeBlock code={b.code} lang="" />;
    case "quote":
      return renderQuote(b, blockId);
    case "custom":
      return renderCustom(b, blockId);
    case "list":
      return <AstList items={b.items} blockId={blockId} />;
    case "table":
      return renderTable(b, blockId);
    case "properties":
      return renderProps(b);
    case "hr":
      return <hr class="md-hr" />;
    case "displayed_math":
      return <MathView tex={b.text} display={true} />;
    case "latex_env":
      return <MathView tex={`\\begin{${b.name}}${b.content}\\end{${b.name}}`} display={true} />;
    case "raw_html":
      return renderRawHtml(b.text);
    case "footnote_def":
      return (
        <div class="footnote-def">
          <sup class="footnote-ref">{b.name}</sup> {renderInlines(b.inline, blockId)}
        </div>
      );
    case "drawer":
    case "directive":
    case "comment":
      return null; // org drawers / `#+KEY:` keywords / `# comment` — not rendered
  }
}

// A `> [!NOTE]` callout (GitHub-flavoured) arrives as a `quote` whose first
// paragraph's leading plain text is `[!TYPE] …` — re-detect it. (Org `#+BEGIN_NOTE`
// is a `custom` block, handled in renderCustom.) Otherwise render a blockquote.
function renderQuote(b: Extract<AstBlock, { kind: "quote" }>, blockId?: string): JSX.Element {
  const first = b.children[0];
  if (first && first.kind === "paragraph") {
    const lead = first.inline[0];
    if (lead && lead.k === "plain") {
      const m = /^\[!(\w+)\]\s*(.*)$/.exec(lead.text);
      if (m) {
        const type = m[1].toLowerCase();
        // The title is `m[2]` (the text after `[!TYPE]` on the first line); the
        // body is everything AFTER that first line — drop the lead `[!TYPE] …`
        // plain node and a following soft break so the title isn't repeated.
        let rest = first.inline.slice(1);
        if (rest[0]?.k === "break") rest = rest.slice(1);
        const bodyChildren: AstBlock[] = rest.length
          ? [{ kind: "paragraph", inline: rest }, ...b.children.slice(1)]
          : b.children.slice(1);
        return (
          <div class={`callout callout-${type}`}>
            <div class="callout-title">{m[2].trim() || type.toUpperCase()}</div>
            <div class="callout-body">{renderBlocks(bodyChildren, blockId)}</div>
          </div>
        );
      }
    }
  }
  return <blockquote class="md-quote">{renderBlocks(b.children, blockId)}</blockquote>;
}

function renderCustom(b: Extract<AstBlock, { kind: "custom" }>, blockId?: string): JSX.Element {
  const type = b.name.toLowerCase();
  if (CALLOUT_TYPES.includes(type)) {
    return (
      <div class={`callout callout-${type}`}>
        <div class="callout-title">{type.toUpperCase()}</div>
        <Show when={b.children.length > 0}>
          <div class="callout-body">{renderBlocks(b.children, blockId)}</div>
        </Show>
      </div>
    );
  }
  if (type === "quote") return <blockquote class="md-quote">{renderBlocks(b.children, blockId)}</blockquote>;
  return <>{renderBlocks(b.children, blockId)}</>;
}

// mldoc/lsdoc discards table column alignment, but Tine renders `:--`/`--:`
// alignment (a beyond-OG feature). Re-derive it from the block's raw separator
// row (matched by column count) so aligned tables don't regress.
function tableAligns(blockId: string | undefined, ncols: number): Align[] {
  if (!blockId) return [];
  const node = doc.byId[blockId];
  if (!node) return [];
  for (const line of node.raw.split("\n")) {
    if (isTableSep(line)) {
      const a = parseAligns(line);
      if (a.length === ncols) return a;
    }
  }
  return [];
}

function renderTable(b: Extract<AstBlock, { kind: "table" }>, blockId?: string): JSX.Element {
  const ncols = b.header?.length ?? b.rows[0]?.length ?? 0;
  const aligns = tableAligns(blockId, ncols);
  const al = (i: number) => (aligns[i] ? { "text-align": aligns[i]! } : undefined);
  return (
    <table class="md-table">
      <Show when={b.header}>
        <thead>
          <tr>
            <For each={b.header!}>{(cell, i) => <th style={al(i())}>{renderInlines(cell, blockId)}</th>}</For>
          </tr>
        </thead>
      </Show>
      <tbody>
        <For each={b.rows}>
          {(row) => (
            <tr>
              <For each={row}>{(cell, i) => <td style={al(i())}>{renderInlines(cell, blockId)}</td>}</For>
            </tr>
          )}
        </For>
      </tbody>
    </table>
  );
}

function renderProps(b: Extract<AstBlock, { kind: "properties" }>): JSX.Element {
  const visible = b.props.filter(([k]) => !HIDDEN_PROPS.has(k.toLowerCase()));
  return (
    <Show when={visible.length > 0}>
      <span class="block-properties">
        <For each={visible}>
          {([k, v]) => (
            <span class="block-property">
              <span class="block-property-key">{k}</span>{" "}
              <span class="block-property-val"><InlineText text={v} /></span>
            </span>
          )}
        </For>
      </span>
    </Show>
  );
}

// An in-block list from the AST (`ListItem[]`). The checkbox toggle still operates
// on the block's raw text — the AST carries no source line (contract R12): match
// the item's flattened text to its `[ ]`/`[x]` line in `raw` and flip that.
function AstList(props: { items: AstListItem[]; blockId?: string }): JSX.Element {
  const ordered = props.items[0]?.ordered ?? false;
  return (
    <Dynamic component={ordered ? "ol" : "ul"} class="md-list">
      <For each={props.items}>
        {(item) => (
          <li class="md-list-item" classList={{ "has-checkbox": item.checkbox !== undefined }}>
            <Show when={item.checkbox !== undefined}>
              <span
                class="block-checkbox"
                classList={{ checked: item.checkbox === true }}
                role="checkbox"
                aria-checked={item.checkbox === true}
                onClick={(e) => {
                  e.stopPropagation();
                  if (props.blockId) toggleAstCheckbox(props.blockId, item);
                }}
              />{" "}
            </Show>
            {renderBlocks(item.content, props.blockId)}
            <Show when={item.items.length > 0}>
              <AstList items={item.items} blockId={props.blockId} />
            </Show>
          </li>
        )}
      </For>
    </Dynamic>
  );
}

/** Render a block's body. Parses the (header-stripped) body text into lsdoc's AST
 *  and renders from it (renderBlocks); until the async parse lands — and in the
 *  mock harness, which can't run lsdoc — it falls back to the legacy line-scanning
 *  BodyContent so content is always visible. Input is `view.lines` (marker /
 *  scheduled / properties already stripped by blockView), so the AST renders only
 *  the visible body and block-header chrome stays in blockView/Block.tsx. */
export function AstBody(props: { lines: string[]; blockId?: string; format?: Format }): JSX.Element {
  const text = createMemo(() => props.lines.join("\n"));
  const [ast] = createResource(text, (t) => parseBlockBatched(t, props.format === "org"));
  return (
    <Show
      when={ast() && ast()!.length > 0}
      fallback={<BodyContent lines={props.lines} blockId={props.blockId} format={props.format} />}
    >
      {renderBlocks(ast()!, props.blockId)}
    </Show>
  );
}

function toggleAstCheckbox(blockId: string, item: AstListItem) {
  const para = item.content[0];
  const text = para && para.kind === "paragraph" ? astText(para.inline).trim() : "";
  const node = doc.byId[blockId];
  if (!node) return;
  const line = node.raw.split("\n").find((l) => {
    const m = /^\s*(?:[-+*]|\d+[.)])\s+\[[ xX]\]\s+(.*)$/.exec(l);
    return m != null && m[1].trim() === text;
  });
  if (line) toggleListItem(blockId, line);
}
