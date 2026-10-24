import { blockRegions, parseBlock } from "../render/parse";
import { utf8ToUtf16Cursor } from "../render/utf16Cursor";
import type { Inline } from "../render/ast";

export interface PreviewSpan {
  from: number;
  to: number;
  kind: "bold" | "italic" | "code" | "page" | "task";
  label?: string;
}

/** Experiment: lsdoc is the sole syntax authority. CM6 receives UTF-16 ranges. */
export function previewSpans(raw: string, format: "md" | "org"): PreviewSpan[] {
  const result: PreviewSpan[] = [];
  const cursor = utf8ToUtf16Cursor(raw);
  // parseBlock re-bullets trimmed raw with "- " / "* "; region ranges
  // already use raw coordinates. Same mapping as render/spans.ts.
  const lead = new TextEncoder().encode(raw.slice(0, raw.length - raw.trimStart().length)).length;
  const header = blockRegions(raw, format).header;
  if (header.marker_range && ["TODO", "DOING", "DONE"].includes(header.marker ?? "")) {
    result.push({ from: cursor(header.marker_range[0]), to: cursor(header.marker_range[1]), kind: "task", label: header.marker! });
  }
  const visit = (value: unknown): void => {
    if (Array.isArray(value)) { value.forEach(visit); return; }
    if (!value || typeof value !== "object") return;
    const n = value as Inline;
    if (n.span) {
      const from = cursor(n.span[0] - 2 + lead), to = cursor(n.span[1] - 2 + lead);
      if (n.k === "emphasis" && (n.emph === "Bold" || n.emph === "Italic")) {
        result.push({ from, to, kind: n.emph === "Bold" ? "bold" : "italic" });
      } else if (n.k === "code" || n.k === "verbatim") {
        result.push({ from, to, kind: "code" });
      } else if (n.k === "link" && n.url.type === "page_ref") {
        result.push({ from, to, kind: "page", label: n.url.v });
      }
    }
    Object.values(value).forEach(visit);
  };
  visit(parseBlock(raw, format === "org"));
  return result.sort((a, b) => a.from - b.from || b.to - a.to);
}
