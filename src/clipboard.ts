// Copying blocks to the clipboard with BOTH a text/plain (Logseq markdown) and a
// text/html flavor, so a paste into a rich editor (Word, Google Docs, an email)
// keeps the outline nesting instead of a flat blob. text/plain is always the
// reliable path (Tauri clipboard plugin); the html is best-effort via the async
// Clipboard API and degrades to text/plain-only where that isn't available
// (see backend.writeRich). Inline markdown (**bold**, [[links]]) is left literal in
// the html — this preserves STRUCTURE, not inline styling.

import { backend } from "./backend";

const esc = (s: string): string =>
  s.replace(/&/g, "&amp;").replace(/</g, "&lt;").replace(/>/g, "&gt;");

/** Convert a tab-indented Logseq outline (`- item`, `\t- child`, continuation
 *  lines) into nested `<ul><li>` HTML. */
export function outlineToHtml(md: string): string {
  const items: { depth: number; html: string }[] = [];
  for (const raw of md.split("\n")) {
    const m = /^(\t*)- ?(.*)$/.exec(raw);
    if (m) items.push({ depth: m[1].length, html: esc(m[2]) });
    else if (items.length && raw.trim()) items[items.length - 1].html += "<br>" + esc(raw.replace(/^[\t ]+/, ""));
  }
  if (!items.length) return "";
  let out = "";
  const stack: number[] = []; // depths of currently-open <ul>
  for (const it of items) {
    while (stack.length && stack[stack.length - 1] > it.depth) {
      out += "</li></ul>";
      stack.pop();
    }
    if (stack.length && stack[stack.length - 1] === it.depth) out += "</li>";
    else {
      out += "<ul>";
      stack.push(it.depth);
    }
    out += "<li>" + it.html;
  }
  while (stack.length) {
    out += "</li></ul>";
    stack.pop();
  }
  return out;
}

/** Put a block outline on the clipboard as text/plain (markdown) + text/html. */
export function copyOutline(md: string): Promise<void> {
  return backend().writeRich(md, outlineToHtml(md));
}
