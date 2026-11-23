// Logseq image braces use EDN delimiters and also accept bare CSS lengths
// and repeated keys. The shared reader supplies the ordered entries.
import { ednSlice, readImageMetaSpans, unquoteEdnString } from "../editor/edn";

const SIZE = /^(?:\d+%?|\d+px)$/;

function sizeOf(kind: string, text: string): string | undefined {
  let value = text;
  // The reader already accepted this string, so unquoting cannot refuse.
  if (kind === "string") value = unquoteEdnString(text.slice(1, -1));
  else if (kind !== "atom") return undefined;
  if (!SIZE.test(value)) return undefined;
  return /^\d+$/.test(value) ? `${value}px` : value;
}

/** Width/height CSS lengths of an image metadata brace; a bare number is px.
 * An unreadable brace yields no size. O(brace bytes) with one WASM read. */
export function parseImageMetaBrace(brace: string | undefined): { width?: string; height?: string } {
  if (!brace) return {};
  const form = readImageMetaSpans(brace);
  if (!form || form.kind !== "map") return {};
  const bytes = new TextEncoder().encode(brace);
  const out: { width?: string; height?: string } = {};
  for (let i = 0; i + 1 < form.children.length; i += 2) {
    const key = ednSlice(bytes, form.children[i]);
    if ((key !== ":width" && key !== ":height") || out[key.slice(1) as "width" | "height"] !== undefined) continue;
    const value = form.children[i + 1];
    const size = sizeOf(value.kind, ednSlice(bytes, value));
    if (size !== undefined) out[key.slice(1) as "width" | "height"] = size;
  }
  return out;
}
