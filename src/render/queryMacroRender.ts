import type { Format, Inline } from "./ast";
import { isQueryMacroName, queryMacroExtents, type MacroExtent } from "../editor/queryMacro";
import { utf8ByteLength, utf8ByteToUtf16Offset } from "./spans";
import { parse_inline_json } from "./wasm/lsdoc_wasm.js";

function shiftInline(inline: Inline, by: number): Inline {
  const shifted = { ...inline, span: inline.span && [inline.span[0] + by, inline.span[1] + by] } as Inline;
  if ("children" in shifted) shifted.children = shifted.children.map((child) => shiftInline(child, by));
  if (shifted.k === "link" && shifted.label) shifted.label = shifted.label.map((child) => shiftInline(child, by));
  if (shifted.k === "plain" && shifted.span_map)
    shifted.span_map = shifted.span_map.map(([text, source, length]) => [text, source + by, length]);
  return shifted;
}

/** Overlay the query reader's complete spans on lsdoc's accepted macro nodes.
 * lsdoc ends a macro at the first `}}`, including a map's closing brace.
 * Only bytes claimed by the shared reader are removed from following nodes. */
export function queryMacroRenderRun(inlines: Inline[], raw: string, format?: Format): { inline: Inline; extent?: MacroExtent }[] {
  const shift = 2 - utf8ByteLength(raw.slice(0, raw.length - raw.trimStart().length));
  // Extents are ordered and disjoint: convert coordinates in one forward pass.
  let unit = 0, byte = 0;
  const toSourceByte = (target: number) => {
    byte += utf8ByteLength(raw.slice(unit, target));
    unit = target;
    return byte + shift;
  };
  const extents = new Map(queryMacroExtents(raw).map((extent) => [
    toSourceByte(extent.start),
    { extent, end: toSourceByte(extent.end) },
  ]));
  const reconcile = (nodes: Inline[]): { inline: Inline; extent?: MacroExtent }[] => {
    const out: { inline: Inline; extent?: MacroExtent }[] = [];
    let claimed = -1;
    for (let inline of nodes) {
      if (inline.span && inline.span[0] < claimed) {
        if (inline.span[1] <= claimed) continue;
        if (inline.k === "plain") {
          const map = inline.span_map ?? [[0, inline.span[0], utf8ByteLength(inline.text)]];
          const survivor = map.find(([, source, length]) => source + length > claimed);
          if (!survivor) continue;
          const cut = survivor[0] + Math.max(0, claimed - survivor[1]);
          inline = { ...inline, text: inline.text.slice(utf8ByteToUtf16Offset(inline.text, cut)),
            span: [claimed, inline.span[1]],
            span_map: map.flatMap(([text, source, length]) => {
              const skipped = Math.max(0, claimed - source);
              return skipped >= length ? [] : [[text + skipped - cut, source + skipped, length - skipped]];
            }),
          };
        } else {
          // Markup can open inside a truncated argument and close after the real
          // macro. Parse only the surviving bytes: that opener belongs to the
          // query, so retaining its AST wrapper would leak or restyle the suffix.
          const suffix = raw.slice(utf8ByteToUtf16Offset(raw, claimed - shift),
            utf8ByteToUtf16Offset(raw, inline.span[1] - shift));
          const parsed = JSON.parse(parse_inline_json(suffix, format === "org")) as Inline[] | null;
          const surviving: Inline[] = parsed ?? [{ k: "plain", text: suffix, span: [0, utf8ByteLength(suffix)] }];
          out.push(...reconcile(surviving.map((node) => shiftInline(node, claimed))));
          continue;
        }
      }
      const recovered = inline.k === "macro" && isQueryMacroName(inline.name) && inline.span
        ? extents.get(inline.span[0]) : undefined;
      if (recovered) claimed = recovered.end;
      out.push({ inline, extent: recovered?.extent });
    }
    return out;
  };
  return reconcile(inlines);
}
