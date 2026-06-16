// Block-body rendering: splits a block's text lines into paragraphs, fenced
// code blocks (syntax-highlighted), and markdown tables.

import { For, Show, type JSX } from "solid-js";
import hljs from "highlight.js/lib/common";
import { InlineText } from "./inline";

type Align = "left" | "center" | "right" | null;

type BodySeg =
  | { kind: "lines"; lines: string[] }
  | { kind: "code"; lang: string; code: string }
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
      segs.push({ kind: "code", lang, code: code.join("\n") });
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

function highlight(code: string, lang: string): string {
  try {
    if (lang && hljs.getLanguage(lang)) return hljs.highlight(code, { language: lang }).value;
    return hljs.highlightAuto(code).value;
  } catch {
    return code.replace(/&/g, "&amp;").replace(/</g, "&lt;");
  }
}

export function BodyContent(props: { lines: string[] }): JSX.Element {
  return (
    <For each={segmentBody(props.lines)}>
      {(seg) => {
        if (seg.kind === "code") {
          return (
            <pre class="code-block">
              <code class="hljs" innerHTML={highlight(seg.code, seg.lang)} />
            </pre>
          );
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
                <InlineText text={line} />
              </>
            )}
          </For>
        );
      }}
    </For>
  );
}
