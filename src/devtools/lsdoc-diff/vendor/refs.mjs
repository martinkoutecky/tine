// OG-faithful reference extraction over mldoc's raw parseJson AST — a port of
// Logseq graph-parser block.cljs (get-page-reference / get-block-reference /
// get-tag), NOT the shallow Mldoc.getReferences (which over-reports macro refs and
// misses emphasis-nested refs). See bootstrap/FINDINGS.md (oracle verdict) and
// DECISIONS.md (reference semantics).
//
//   page refs:  OG get-page-reference: Page_ref, Search, Org Search, File label,
//               Nested_link, Tag, and embed-macro arg.
//   block refs: OG get-block-reference: Block_ref, id:// links, UUID-ish link
//               targets, and embed-macro ((uuid)) arg — UUID-gated.
//   Recurses into Emphasis/Paragraph/Heading/etc.; Src/Code/Latex are literal.

const UUID = /^[0-9a-fA-F]{8}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{12}$/;
const parseUuid = (s) => (typeof s === "string" && UUID.test(s.trim()) ? s.trim().toLowerCase() : null);
const LITERAL = new Set(["Src", "Code", "Latex_Fragment", "Latex_Environment", "Export", "CommentBlock", "Export_Snippet", "Raw_Html", "Inline_Html"]);
const unbr = (s) => { const m = /^\[\[([\s\S]*)\]\]$/.exec((s ?? "").trim()); return m ? m[1] : s; };
const pageName = (s) => { const m = /^\[\[([\s\S]*)\]\]$/.exec((s ?? "").trim()); return m ? m[1] : null; };
const blockRefId = (s) => { const m = /^\(\(([\s\S]*)\)\)$/.exec((s ?? "").trim()); return m ? m[1].trim() : null; };
const pageRefValue = (s) => blockRefId(s) || unbr(s);
const pageRefShaped = (s) => typeof s === "string" && s.trim().startsWith("[[") && s.trim().endsWith("]]");
const localAsset = (s) => typeof s === "string" && s.replace(/^[./]*/, "").startsWith("assets");
const drawPath = (s) => typeof s === "string" && s.startsWith("draws");

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

function firstLabelValue(label) {
  if (!Array.isArray(label) || !Array.isArray(label[0])) return null;
  const [tag, value] = label[0];
  if (typeof value === "string") return value;
  if (tag === "Nested_link") return value?.content || null;
  if (tag === "Tag") return getTag(value);
  return null;
}

function pageRefFromLink(data, format) {
  const url = data?.url;
  if (!Array.isArray(url)) return null;
  const [type, value] = url;
  if (type === "Page_ref" && typeof value === "string" && !localAsset(value) && !drawPath(value)) return value;
  if (type === "Search" && pageRefShaped(value)) return unbr(value);
  if (type === "Search" && format === "org" && typeof value === "string" && !localAsset(value)) return value;
  if (type === "File") return firstLabelValue(data.label);
  return null;
}

function blockRefFromLink(data) {
  const url = data?.url;
  if (!Array.isArray(url)) return null;
  const [type, value] = url;
  if (type === "Block_ref") return parseUuid(value);
  if (type === "Complex" && value?.protocol === "id") return parseUuid(value.link);
  if (typeof value === "string") return parseUuid(blockRefId(value) || value);
  return null;
}

function propertyPageRefFromInline(node) {
  if (!Array.isArray(node) || typeof node[0] !== "string") return null;
  const [tag, data] = node;
  if (tag === "Link") {
    const url = data?.url;
    if (!Array.isArray(url)) return null;
    return url[0] === "Page_ref" || url[0] === "Search" ? url[1] : null;
  }
  if (tag === "Nested_link") return pageName(data?.content);
  if (tag === "Tag") {
    const first = Array.isArray(data) ? data[0] : null;
    if (!Array.isArray(first)) return null;
    if (first[0] === "Plain") return first[1];
    return propertyPageRefFromInline(first);
  }
  return null;
}

function pushPropertyPage(out, page) {
  if (typeof page !== "string") return;
  const trimmed = page.trim();
  if (trimmed) out.page.push(trimmed);
}

function walkBlockRefs(node, out) {
  if (Array.isArray(node)) {
    if (typeof node[0] === "string") {
      const tag = node[0];
      if (!LITERAL.has(tag)) {
        for (const x of node.slice(1)) walkBlockRefs(x, out);
      }
      if (tag === "Link") {
        const id = blockRefFromLink(node[1]);
        if (id) out.block.push(id);
        return;
      }
      if (tag === "Macro") {
        const { name, arguments: args } = node[1] || {};
        if (name === "embed" && Array.isArray(args)) {
          const uid = parseUuid(blockRefId(args[0]));
          if (uid) out.block.push(uid);
        }
        return;
      }
      return;
    }
    for (const x of node) walkBlockRefs(x, out);
  } else if (node && typeof node === "object") {
    for (const k of Object.keys(node)) walkBlockRefs(node[k], out);
  }
}

function addPropertyRefs(properties, out) {
  for (const prop of properties ?? []) {
    const refsAst = Array.isArray(prop) ? prop[2] : null;
    if (!Array.isArray(refsAst)) continue;
    for (const seg of refsAst) pushPropertyPage(out, propertyPageRefFromInline(seg));
    walkBlockRefs(refsAst, out);
  }
}

function walk(node, out, format) {
  if (Array.isArray(node)) {
    if (typeof node[0] === "string") {
      const tag = node[0];
      if (tag === "Property_Drawer") {
        addPropertyRefs(node[1], out);
        return;
      }
      if (!LITERAL.has(tag)) {
        for (const x of node.slice(1)) walk(x, out, format);
      }
      if (tag === "Link") {
        const page = pageRefFromLink(node[1], format);
        if (page) out.page.push(pageRefValue(page));
        const id = blockRefFromLink(node[1]);
        if (id) out.block.push(id);
        return;
      }
      if (tag === "Nested_link") { out.page.push(pageRefValue(node[1]?.content || "")); return; }
      if (tag === "Tag") { out.page.push(pageRefValue(getTag(node[1]))); return; }
      if (tag === "Macro") {
        const { name, arguments: args } = node[1] || {};
        if (name === "embed" && Array.isArray(args)) {
          const joined = args.join(", ");
          out.page.push(pageRefValue(joined));
          const uid = parseUuid(blockRefId(args[0]));
          if (uid) out.block.push(uid);                                   // {{embed ((uuid))}}
        }
        return;
      }
      if (LITERAL.has(tag)) return;
    } else {
      for (const x of node) walk(x, out, format);
    }
  } else if (node && typeof node === "object") {
    for (const k of Object.keys(node)) walk(node[k], out, format);
  }
}

// Returns { page: string[], block: string[] } — deduped, sorted, for comparison.
export function extractRefs(ast, format = "md") {
  const o = { page: [], block: [] };
  walk(ast, o, format);
  return {
    page: [...new Set(o.page)].sort(),
    block: [...new Set(o.block)].sort(),
  };
}
