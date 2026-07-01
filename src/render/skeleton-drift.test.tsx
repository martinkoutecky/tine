// ANTI-DRIFT GATE (M3 render contract, Option C2): lsdoc's canonical HTML skeleton
// (`render_html`) vs the frontend's REACTIVE skeleton (`renderBlocks` → jsdom), for the
// SAME block bodies, from the SAME vendored wasm the app ships.
// ===========================================================================
// Tine has two renderers conforming to ONE skeleton: lsdoc's static `render_html` (the
// HTML export consumes it) and the frontend's interactive SolidJS renderer. They MUST
// agree on structure — tags, classes, nesting, text — or the export and the app drift.
// This test renders each fixture both ways, NORMALIZES away the things that legitimately
// differ (lsdoc emits `data-*` hooks + escaped text for a static consumer; the frontend
// resolves refs/assets reactively, adds event handlers + interactive chrome, and shows
// `[[bracket]]` spans), and asserts the remaining skeletons are byte-identical.
//
// Scope: the deterministic, synchronous body constructs. Excluded by design — images /
// media / block-refs (async resource loads → jsdom can't resolve them) and properties
// (Block.tsx draws those as chrome, not via the body renderer); the export's decoration
// of those `data-*` hooks is covered in `crates/tine-core/src/publish.rs` tests instead.

import { describe, it, expect, beforeAll } from "vitest";
import { render } from "solid-js/web";
import { renderBlocks } from "./body";
import { typographic } from "./typography";
import { initParser, parseBlock } from "./parse";
import { render_block_html } from "./wasm/lsdoc_wasm.js";

beforeAll(async () => {
  await initParser();
});

// Interactive-only chrome the frontend adds and lsdoc (static) does not — removed before
// comparison.
const REMOVE_SELECTORS = [
  "button.code-copy",
  ".asset-action-bar",
  ".img-resize-grip",
  ".media-open-external",
  ".media-audio-widen",
  ".block-ref-preview",
];
// Frontend-only state-modifier classes (resolve-state / lazy-render markers).
const DROP_CLASSES = new Set(["ast-deferred", "ast-fallback", "block-ref-missing"]);

/** Reduce an HTML string to its structural skeleton: tag names + (sorted) classes +
 *  nesting + text, dropping ALL attributes (lsdoc's `data-*` + the frontend's handlers/
 *  href/style), unwrapping `.bracket` spans, and collapsing each `.math` node to a
 *  `<math:tex>` sentinel (the frontend shows raw tex; lsdoc carries it in `data-tex`). */
function skeleton(html: string): string {
  const root = document.createElement("div");
  root.innerHTML = html;
  for (const sel of REMOVE_SELECTORS) root.querySelectorAll(sel).forEach((e) => e.remove());
  // delta 2: the frontend wraps `[[ ]]` in dimmed `.bracket` spans; lsdoc emits plain text.
  root.querySelectorAll("span.bracket").forEach((e) => e.replaceWith(document.createTextNode(e.textContent ?? "")));
  return norm(root).replace(/\s+/g, " ").trim();
}

function norm(node: Node): string {
  let out = "";
  for (const child of Array.from(node.childNodes)) {
    if (child.nodeType === 3) {
      // Normalize the sanctioned Tine typographic replacement (render/typography.ts)
      // on TEXT nodes only (safe — never HTML attrs/comments): the frontend renders
      // `->`→`→` etc., lsdoc's reference is raw ASCII, so map both to glyphs. It's
      // idempotent on the already-transformed frontend side; a no-op for text with
      // no arrow/dash triggers (all current fixtures).
      out += typographic(child.textContent ?? "");
      continue;
    }
    if (child.nodeType !== 1) continue;
    const el = child as Element;
    const tag = el.tagName.toLowerCase();
    if (el.classList.contains("math")) {
      const tex = (el.getAttribute("data-tex") ?? el.textContent ?? "").trim();
      const disp = el.classList.contains("math-display") ? ":display" : "";
      out += `<math${disp}:${tex}>`;
      continue;
    }
    const classes = Array.from(el.classList).filter((c) => !DROP_CLASSES.has(c)).sort();
    const cls = classes.length ? "." + classes.join(".") : "";
    out += `<${tag}${cls}>${norm(el)}</${tag}>`;
  }
  return out;
}

/** The displayed heading level (1–6) of the first block, mirroring the facet cache —
 *  lsdoc v0.2.3 `render_html` wraps a heading bullet in `heading-text` itself, so the
 *  frontend (which renders the AST) must be given the same level to wrap block 0. */
function headingLevelOf(blocks: ReturnType<typeof parseBlock>): number | null {
  const h = blocks[0];
  if (!h) return null;
  const size = h.kind === "bullet" ? h.size ?? null : h.kind === "heading" ? h.size ?? h.level : null;
  return size != null && size >= 1 && size <= 6 ? size : null;
}

function frontendSkeleton(raw: string): string {
  const div = document.createElement("div");
  const blocks = parseBlock(raw, false);
  const dispose = render(() => renderBlocks(blocks, undefined, headingLevelOf(blocks)), div);
  const out = div.innerHTML;
  dispose();
  return skeleton(out);
}

const lsdocSkeleton = (raw: string): string => skeleton(render_block_html(raw, false));

// Each fixture is ONE block body (the raw a Tine block carries, sans bullet). ASCII only
// (emoji → Twemoji <img> on the frontend); no images / block-refs / properties (see header).
const FIXTURES: Record<string, string> = {
  "plain + emphasis + inline code": "normal **bold** and *italic* and ~~strike~~ and ==hl== and `code` here",
  "soft-break joins lines": "first line\nsecond line\nthird line",
  "page ref (delta 2: bracket spans)": "see [[My Page]] for details",
  "page ref with alias": "see [[Target Page][the alias]] here",
  "tag + bracket tag": "about #project and #[[multi word topic]] today",
  "external link": "visit [the example site](https://example.com/a/b) now",
  "heading marker stripped": "### A Heading Line",
  "subscript / superscript": "H~2~O and x^2^ and a_1",
  "inline math (delta: data-tex)": "Euler $e^{i}+1=0$ inline",
  "display math": "energy $$E=mc^2$$ here",
  "unordered in-block list": "* first item\n* second item\n* third item",
  "ordered in-block list": "1. alpha\n2. beta\n3. gamma",
  "nested list": "* parent\n    * child a\n    * child b\n* sibling",
  "checkbox list": "* [ ] todo item\n* [x] done item",
  "definition list (delta 3: term)": "Coffee\n: A hot drink\nTea\n: Another drink",
  "markdown table": "| Name | Age |\n| --- | --- |\n| Ann | 30 |\n| Bob | 25 |",
  "github callout (delta 5)": "> [!NOTE] Heads up\n> the body of the note\n> second body line",
  "github callout markup title": "> [!TIP] Use **bold** and `code`\n> body line",
  // lsdoc v0.2.4: org-style `#+BEGIN_<TYPE>` admonitions parse as callouts in MD
  // mode too (not just org), matching org. Locks that md↔frontend parity.
  "md org-admonition callout": "#+BEGIN_NOTE\nbody of the note\n#+END_NOTE",
  "md org-admonition tip": "#+BEGIN_TIP\nthis is a tip\n#+END_TIP",
  "github callout no title": "> [!WARNING]\n> be careful here",
  "plain blockquote": "> just a quote\n> over two lines",
  "horizontal rule": "text above\n\n---\n\ntext below",
  "active timestamp": "meeting <2026-06-30 Tue 14:00>",
  "timestamp range (delta 4)": "window <2026-01-01 Thu>--<2026-01-02 Fri> open",
  "inactive timestamp": "noted [2026-06-30 Tue]",
};

describe("skeleton drift: lsdoc render_html vs frontend renderBlocks", () => {
  for (const [name, raw] of Object.entries(FIXTURES)) {
    it(name, () => {
      const lsdoc = lsdocSkeleton(raw);
      const front = frontendSkeleton(raw);
      expect(front, `\n  lsdoc:    ${lsdoc}\n  frontend: ${front}\n`).toBe(lsdoc);
    });
  }
});
