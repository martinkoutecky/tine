// Block-body rendering: splits a block's text lines into paragraphs, fenced
// code blocks (syntax-highlighted), and markdown tables.

import { For, Show, createMemo, createResource, type JSX } from "solid-js";
import { InlineText } from "./inline";
import { backend } from "../backend";
import { evalCalc } from "../editor/calc";

type Align = "left" | "center" | "right" | null;

type BodySeg =
  | { kind: "lines"; lines: string[] }
  | { kind: "code"; lang: string; code: string }
  | { kind: "calc"; code: string }
  | { kind: "table"; rows: string[][]; aligns: Align[] }
  | { kind: "hr" }
  | { kind: "quote"; lines: string[] }
  | { kind: "callout"; kind2: string; title: string; lines: string[] };

function isTableRow(line: string): boolean {
  return /^\s*\|.*\|\s*$/.test(line);
}
function isTableSep(line: string): boolean {
  return /^\s*\|?[\s:|-]+\|?\s*$/.test(line) && line.includes("-");
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

export function segmentBody(lines: string[]): BodySeg[] {
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
        } else {
          segs.push({ kind: "lines", lines: inner }); // center / unknown → plain
        }
        continue;
      }
      // no matching END — fall through and treat as an ordinary line
    }
    buf.push(line);
  }
  flush();
  return segs;
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
        {(ln) => (
          <div class="calc-row" classList={{ "calc-error": !!ln.error }}>
            <span class="calc-in">{ln.input || " "}</span>
            <span class="calc-out">{ln.output ?? ""}</span>
          </div>
        )}
      </For>
    </div>
  );
}

export function BodyContent(props: { lines: string[]; blockId?: string }): JSX.Element {
  return (
    <For each={segmentBody(props.lines)}>
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
                  <For each={head}>{(c, i) => <th style={al(i())}><InlineText text={c} /></th>}</For>
                </tr>
              </thead>
              <tbody>
                <For each={body}>
                  {(row) => (
                    <tr>
                      <For each={row}>{(c, i) => <td style={al(i())}><InlineText text={c} /></td>}</For>
                    </tr>
                  )}
                </For>
              </tbody>
            </table>
          );
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
                    <InlineText text={line} />
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
                        <InlineText text={line} />
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
                <InlineText text={line} blockId={props.blockId} />
              </>
            )}
          </For>
        );
      }}
    </For>
  );
}
