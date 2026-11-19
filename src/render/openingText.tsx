import { createMemo, Show, type JSX } from "solid-js";
import type { Format } from "../types";
import type { Inline } from "./ast";
import { EmojiText } from "./emoji";
import { renderInlines } from "./inline";
import { blockRegions, isQuarantined, parseBlock, parserReady } from "./parse";
import { rebulletedSourceByteToRawByte } from "./spans";
import { utf8ToUtf16Cursor } from "./utf16Cursor";

/** A page's opening text exactly as written (D-20261119-PG), for every row
 *  that shows it: the page's own opening row and the synthetic preamble rows
 *  of Linked References and query results. One parse of the whole text
 *  renders its prose as one inline run, as `InlineText` does. What that parse
 *  would lift out of the run stays as source text instead: a leading task
 *  marker or priority, and every later Markdown property group (in opening
 *  text it is prose, never properties, so it has no chips). O(text). */
export function OpeningText(props: { text: string; format: Format; blockId?: string }): JSX.Element {
  const inlines = createMemo((): Inline[] | null => {
    if (!parserReady()) return null;
    const raw = props.text;
    const blocks = parseBlock(raw, props.format === "org");
    if (isQuarantined(blocks)) return null;
    const at = utf8ToUtf16Cursor(raw);
    const { marker_range, priority_range } = blockRegions(raw, props.format).header;
    const out: Inline[] = [];
    const lead = Math.max(marker_range?.[1] ?? 0, priority_range?.[1] ?? 0);
    if (lead) out.push({ k: "plain", text: raw.slice(0, at(lead) + (raw[at(lead)] === " " ? 1 : 0)) });
    // The marker heads the first line: its run continues that line.
    let sameLine = lead > 0;
    const append = (run: Inline[]) => {
      if (!run.length) return;
      const last = out[out.length - 1];
      if (!sameLine && last && last.k !== "break" && run[0].k !== "break") out.push({ k: "break" });
      sameLine = false;
      out.push(...run);
    };
    for (const b of blocks) {
      if (b.kind === "paragraph" || b.kind === "bullet" || b.kind === "heading") append(b.inline);
      else if (b.kind === "properties" && b.span && props.format === "md") {
        const [start, end] = b.span.map(byte => at(rebulletedSourceByteToRawByte(raw, byte)));
        const lines = raw.slice(start, end).split("\n");
        append(lines.flatMap((text, i): Inline[] => i === 0 ? [{ k: "plain", text }]
          : text || i < lines.length - 1 ? [{ k: "break" }, { k: "plain", text }] : [{ k: "break" }]));
      }
    }
    while (out[out.length - 1]?.k === "break") out.pop();
    return out;
  });
  return (
    <Show when={inlines()?.length} fallback={<EmojiText text={props.text} />}>
      {renderInlines(inlines()!, props.blockId, false, false, props.format, props.text)}
    </Show>
  );
}
