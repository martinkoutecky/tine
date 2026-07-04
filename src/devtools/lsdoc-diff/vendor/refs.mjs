// OG-faithful reference extraction over mldoc's raw parseJson AST — a port of
// Logseq graph-parser block.cljs (get-page-reference / get-block-reference /
// get-tag), NOT the shallow Mldoc.getReferences (which over-reports macro refs and
// misses emphasis-nested refs). See bootstrap/FINDINGS.md (oracle verdict) and
// DECISIONS.md (reference semantics).
//
//   page refs:  Link Page_ref value; Tag (get-tag, un-bracketed); embed-macro arg
//               (un-bracketed) — ONLY name == "embed", not query/renderer.
//   block refs: Link Block_ref id; embed-macro ((uuid)) arg — BOTH parse-uuid-gated
//               (OG drops non-UUID block refs; raw mldoc keeps them).
//   Recurses into Emphasis/Paragraph/Heading/etc.; Src/Code/Latex are literal.

const UUID = /^[0-9a-fA-F]{8}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{12}$/;
const parseUuid = (s) => (typeof s === "string" && UUID.test(s.trim()) ? s.trim() : null);
const LITERAL = new Set(["Src", "Code", "Latex_Fragment", "Latex_Environment", "Export", "Export_Snippet", "Raw_Html", "Inline_Html"]);
const unbr = (s) => { const m = /^\[\[([\s\S]*)\]\]$/.exec((s ?? "").trim()); return m ? m[1] : s; };
const blockRefId = (s) => { const m = /^\(\(([\s\S]*)\)\)$/.exec((s ?? "").trim()); return m ? m[1].trim() : null; };

function getTag(inline) {
  if (!Array.isArray(inline)) return "";
  return inline.map((seg) => {
    if (!Array.isArray(seg)) return "";
    if (seg[0] === "Plain") return seg[1];
    if (seg[0] === "Link") return seg[1]?.full_text || "";
    if (seg[0] === "Nested_link") return seg[1]?.content || "";
    return "";
  }).join("");
}

function walk(node, out) {
  if (Array.isArray(node)) {
    if (typeof node[0] === "string") {
      const tag = node[0];
      if (tag === "Link") {
        const url = node[1]?.url;
        if (Array.isArray(url)) {
          if (url[0] === "Page_ref" && typeof url[1] === "string") out.page.push(url[1]);
          else if (url[0] === "Block_ref") { const id = parseUuid(url[1]); if (id) out.block.push(id); }
        }
        return;
      }
      if (tag === "Tag") { out.page.push(unbr(getTag(node[1]))); return; }
      if (tag === "Macro") {
        const { name, arguments: args } = node[1] || {};
        if (name === "embed" && Array.isArray(args)) {
          const joined = args.join(", ");
          const id = blockRefId(joined); const uid = parseUuid(id);
          if (uid) out.block.push(uid);                                   // {{embed ((uuid))}}
          else { const p = unbr(joined); if (p !== joined || /^\[\[/.test(joined)) out.page.push(p); } // {{embed [[Foo]]}}
        }
        return;
      }
      if (LITERAL.has(tag)) return;
      for (const x of node) walk(x, out);
    } else {
      for (const x of node) walk(x, out);
    }
  } else if (node && typeof node === "object") {
    for (const k of Object.keys(node)) walk(node[k], out);
  }
}

// Returns { page: string[], block: string[] } — deduped, sorted, for comparison.
export function extractRefs(ast) {
  const o = { page: [], block: [] };
  walk(ast, o);
  return {
    page: [...new Set(o.page)].sort(),
    block: [...new Set(o.block)].sort(),
  };
}
