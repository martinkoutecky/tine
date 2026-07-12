import DOMPurify from "dompurify";
import TurndownService from "turndown";
import { gfm } from "turndown-plugin-gfm";
import { parseOutline, type OutlineNode } from "./outline";

export const HTML_PASTE_LIMITS = {
  inputBytes: 2 * 1024 * 1024,
  nodes: 20_000,
  depth: 80,
  outputBytes: 2 * 1024 * 1024,
} as const;

const TAGS = [
  "p", "div", "br", "h1", "h2", "h3", "h4", "h5", "h6",
  "ul", "ol", "li", "blockquote", "pre", "code", "a", "strong", "b",
  "em", "i", "del", "s", "strike", "hr", "table", "thead", "tbody",
  "tfoot", "tr", "th", "td", "caption", "span",
];
const ATTRS = ["href", "title", "colspan", "rowspan"];
const STRUCTURAL = "p,h1,h2,h3,h4,h5,h6,ul,ol,li,blockquote,pre,table";

function withinDomBounds(root: ParentNode): boolean {
  let count = 0;
  const stack: Array<{ node: Node; depth: number }> = [...root.childNodes].map((node) => ({ node, depth: 1 }));
  while (stack.length) {
    const { node, depth } = stack.pop()!;
    count += 1;
    if (count > HTML_PASTE_LIMITS.nodes || depth > HTML_PASTE_LIMITS.depth) return false;
    for (const child of node.childNodes) stack.push({ node: child, depth: depth + 1 });
  }
  return true;
}

/** Deterministically preserve explicit clipboard HTML structure. Returns null
 * when HTML is absent, unsafe/oversized, or no richer than the plain flavor. */
export function structuredHtmlOutline(html: string, plain: string): OutlineNode[] | null {
  if (!html.trim() || new TextEncoder().encode(html).byteLength > HTML_PASTE_LIMITS.inputBytes) return null;
  const safe = DOMPurify.sanitize(html, {
    ALLOWED_TAGS: TAGS,
    ALLOWED_ATTR: ATTRS,
    ALLOW_DATA_ATTR: false,
  });
  const doc = new DOMParser().parseFromString(`<body>${safe}</body>`, "text/html");
  if (!withinDomBounds(doc.body)) return null;
  const structural = doc.body.querySelectorAll(STRUCTURAL);
  if (!structural.length) return null;

  const service = new TurndownService({
    bulletListMarker: "-",
    codeBlockStyle: "fenced",
    headingStyle: "atx",
    emDelimiter: "_",
    strongDelimiter: "**",
  });
  service.use(gfm);
  let markdown: string;
  try {
    markdown = service.turndown(doc.body).replace(/\n{3,}/g, "\n\n").trim();
  } catch {
    return null;
  }
  if (!markdown || new TextEncoder().encode(markdown).byteLength > HTML_PASTE_LIMITS.outputBytes) return null;
  // A lone wrapper that converts to the exact plain text adds no useful
  // semantics; retain the browser/native plain path in that case.
  if (structural.length === 1 && markdown === plain.trim()) return null;
  const outline = parseOutline(markdown);
  return outline.length ? outline : null;
}
