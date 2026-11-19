import { For, Match, Switch, createMemo, type JSX } from "solid-js";
import type { Format } from "../render/ast";
import { AstBody } from "../render/body";
import { openingSegments } from "../editor/properties";

type Piece = { kind: "group" | "text" | "blank"; text: string };

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

/** One text piece, read as untitled body text (`parseBlock`): a task word
 *  or priority on its first line is text like any other, so nothing is lifted. */
function textPiece(text: string, format: Format): JSX.Element {
  return <AstBody raw={text} format={format} untitled />;
}

/** A page's opening text (D-20261119-PG): each property group after the
 *  header shows exactly as written, one line per source line, and the text
 *  between renders as body text does (queries, macros and all), with no
 *  block header: a task word or priority is ordinary text. One parser answer
 *  per text, then one parse per piece. */
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
