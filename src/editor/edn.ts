// I-4/I-12: authored EDN structure belongs to query_edn.rs, shared with native.
// These clients never discover delimiters or edit spans in JavaScript.
import { query_edn_json } from "../render/wasm/lsdoc_wasm.js";
import { QueryPrintRefusedError } from "../backend";

function call<T>(source: string, operation: string, value = ""): T {
  return JSON.parse(query_edn_json(source, operation, value)) as T;
}
export interface EdnOptions { title: string | null; collapsed: boolean; table: boolean }
export interface EdnForm { span: { start: number; end: number }; kind: string; children: EdnForm[] }
export function readEdn(source: string): EdnForm | null { return call(source, "read"); }
/** Slice a Rust byte span; pass pre-encoded bytes when reading several forms. */
export function ednSlice(source: string | Uint8Array, form: EdnForm): string {
  const bytes = typeof source === "string" ? new TextEncoder().encode(source) : source;
  return new TextDecoder().decode(bytes.subarray(form.span.start, form.span.end));
}
export function readEdnOptions(source: string): EdnOptions | null { return call(source, "options"); }
export function editEdnTitle(source: string, title: string): string {
  const edited = call<string | null>(source, "title", title);
  if (edited === null) throw new QueryPrintRefusedError("syntax", {
    kind: "syntax", message: "Unreadable EDN options; the query was not changed.",
  });
  return edited;
}
/** Escaped inner text, preserving the existing helper's public convention. */
export function quoteEdnString(source: string): string { return call<string>(source, "quote").slice(1, -1); }
export function unquoteEdnString(inner: string): string {
  const text = call<string | null>(`"${inner}"`, "unquote");
  if (text === null) throw new QueryPrintRefusedError("syntax", { kind: "syntax", message: "Unreadable EDN string." });
  return text;
}
export function splitTrailingMap(source: string): { form: string; opts: string } {
  const [form, opts] = call<[string, string]>(source, "split");
  return { form, opts };
}
