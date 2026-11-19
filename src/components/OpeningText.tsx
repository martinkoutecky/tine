import { For, Match, Switch, createMemo, type JSX } from "solid-js";
import type { Format } from "../render/ast";
import { AstBody } from "../render/body";
import { openingSegments } from "../editor/properties";
import { blockRegions, parserReady } from "../render/parse";
import { utf8ToUtf16Cursor } from "../render/utf16Cursor";

type Piece = { kind: "group" | "text" | "blank"; text: string };

/** The task word and/or priority a block parse lifts off `text`'s first line
 *  (a block body never draws them: they are a block's chrome), as their exact
 *  source with the blanks after them. Opening text is shown as written
 *  (D-20261119-PG), so they print as text, with no task behaviour. `ownLine`
 *  when the rest of their line is empty. Reads the parse `AstBody` caches. */
function liftedLead(text: string, format: Format): { text: string; ownLine: boolean } | null {
  if (!parserReady()) return null;
  const { marker_range, priority_range } = blockRegions(text, format).header;
  const lifted = Math.max(marker_range?.[1] ?? 0, priority_range?.[1] ?? 0);
  if (!lifted) return null;
  let end = utf8ToUtf16Cursor(text)(lifted);
  while (text[end] === " " || text[end] === "\t") end++;
  return { text: text.slice(0, end), ownLine: end >= text.length || text[end] === "\n" || text[end] === "\r" };
}

/** The pieces of opening text in source order: each group's lines, each
 *  block piece without its edge line ends, and one blank piece per blank
 *  source line between them (a group ends with its own line end). */
function piecesOf(raw: string, format: Format): Piece[] {
  const out: Piece[] = [];
  const blanks = (n: number) => { for (let i = 0; i < n; i++) out.push({ kind: "blank", text: "" }); };
  const lineEnds = (s: string) => (s.match(/\n/g) ?? []).length;
  for (const s of openingSegments(raw, format)) {
    if (s.group) { out.push({ kind: "group", text: s.text.replace(/\r?\n$/, "").replace(/\r\n/g, "\n") }); continue; }
    const { start, end } = contentLines(s.text);
    if (start === end) { blanks(lineEnds(s.text)); continue; }
    blanks(lineEnds(s.text.slice(0, start)));
    out.push({ kind: "text", text: s.text.slice(start, end) });
    blanks(Math.max(0, lineEnds(s.text.slice(end)) - 1));
  }
  return out;
}

/** The lines of `text` from its first to its last non-blank one, without the
 *  last one's line end: its spaces and tabs are kept as written. One scan in
 *  from each edge (a regex trim retries at every whitespace character). */
function contentLines(text: string): { start: number; end: number } {
  const blank = (i: number) => /\s/.test(text[i]);
  let last = text.length;
  while (last > 0 && blank(last - 1)) last--;
  let first = 0;
  while (first < last && blank(first)) first++;
  if (first === last) return { start: 0, end: 0 };
  const start = text.lastIndexOf("\n", first) + 1;
  let end = text.indexOf("\n", last);
  if (end < 0) end = text.length;
  if (text[end - 1] === "\r") end--;
  return { start, end };
}

function textPiece(text: string, format: Format): JSX.Element {
  const lead = liftedLead(text, format);
  const body = <AstBody raw={text} format={format} />;
  if (!lead) return body;
  const shown = <span class="source-text" style={{ "white-space": "pre-wrap" }}>{lead.text}</span>;
  return lead.ownLine && lead.text.length < text.trimEnd().length ? [shown, <br />, body] : [shown, body];
}

/** A page's opening text (D-20261119-PG): each property group after the
 *  header shows exactly as written, one line per source line, and the text
 *  between renders as any block body does (queries, macros and all), its
 *  lifted task word or priority printed as text. One parser answer per
 *  text, then one block parse per piece. */
export function OpeningText(props: { raw: string; format: Format }): JSX.Element {
  const pieces = createMemo(() => piecesOf(props.raw, props.format));
  return (
    <For each={pieces()}>
      {(p) => (
        <Switch>
          <Match when={p.kind === "group"}><div class="source-text opening-group" style={{ "white-space": "pre-wrap" }}>{p.text}</div></Match>
          <Match when={p.kind === "text"}><div class="opening-text">{textPiece(p.text, props.format)}</div></Match>
          <Match when={p.kind === "blank"}><div class="opening-blank"><br /></div></Match>
        </Switch>
      )}
    </For>
  );
}
