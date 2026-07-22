// Normalize mldoc's parseJson AST into the "observable projection" that lsdoc's
// Rust side also emits, so the two can be diffed structurally. See DECISIONS.md
// ("Oracle granularity"). We compare on:
//   - block tree: kind, level/ordered, lang/code, props, table cells, span
//   - inline tree: kind + payload + order + nesting (NO inline spans — mldoc
//     emits none; lsdoc keeps its own but does not diff them here)
//
// Shape of mldoc parseJson output: top level is an array of [node, {start_pos,
// end_pos}]. Block nodes nested inside Quote/List item content are BARE (no pos
// wrapper). Hence normTop (wrapped) vs normNode (bare).

// ---- inline ---------------------------------------------------------------

function normUrl(url) {
  if (!Array.isArray(url)) return { type: "raw", v: url };
  const [ty, val] = url;
  switch (ty) {
    case "Page_ref": return { type: "page_ref", v: val };
    case "Block_ref": return { type: "block_ref", v: val };
    case "Search": return { type: "search", v: val };
    case "File": return { type: "file", v: val };
    case "Complex": return { type: "complex", protocol: val?.protocol, link: val?.link };
    default: return { type: String(ty).toLowerCase(), v: val };
  }
}

export function normInline(seg) {
  if (!Array.isArray(seg)) return { k: "raw", v: seg };
  const t = seg[0];
  switch (t) {
    case "Plain": return { k: "plain", text: seg[1] };
    case "Code": return { k: "code", text: seg[1] };
    case "Verbatim": return { k: "verbatim", text: seg[1] };
    case "Break_Line": return { k: "break" };
    case "Hard_Break_Line": return { k: "hardbreak" };
    case "Emphasis": {
      const kind = seg[1]?.[0]?.[0];          // "Bold" | "Italic" | "Strike_through" | "Highlight" | "Underline"
      const kids = (seg[1]?.[1] ?? []).map(normInline);
      return { k: "emphasis", emph: kind, children: kids };
    }
    case "Link": {
      const o = seg[1] ?? {};
      const r = { k: "link", url: normUrl(o.url), label: (o.label ?? []).map(normInline), full: o.full_text };
      // image-ness: mldoc has no native bit — derive from the leading `!` of full_text
      // (the markdown image syntax `![…](…)`). lsdoc derives the same; both omit false.
      if (typeof o.full_text === "string" && o.full_text.startsWith("!")) r.image = true;
      if (o.metadata) r.metadata = o.metadata;     // mldoc emits "" for none → omit
      if (o.title != null) r.title = o.title;       // raw inner, only when present
      return r;
    }
    case "Subscript": return { k: "subscript", children: (seg[1] ?? []).map(normInline) };
    case "Superscript": return { k: "superscript", children: (seg[1] ?? []).map(normInline) };
    case "Nested_link": return { k: "nested_link", content: seg[1]?.content };
    case "Tag": return { k: "tag", children: (seg[1] ?? []).map(normInline) };
    case "Macro": return { k: "macro", name: seg[1]?.name, args: seg[1]?.arguments ?? [] };
    case "Latex_Fragment": return { k: "latex", mode: seg[1]?.[0], body: seg[1]?.[1] };
    case "Timestamp": return { k: "timestamp", ts: seg[1]?.[0], date: seg[1]?.[1] };
    case "Cookie": {
      const c = seg[1] ?? [];
      const r = { k: "cookie", kind: c[0], value: c[1] };
      if (c[0] === "Absolute") r.total = c[2];
      return r;
    }
    case "Footnote_Reference": return { k: "fnref", name: seg[1]?.name };
    case "Export_Snippet": return { k: "export_snippet", name: seg[1], content: seg[2] };
    case "Target": return { k: "target", text: seg[1] };
    case "Radio_Target": return { k: "target", text: seg[1] };
    case "Email": return { k: "email", text: seg[1] };
    case "Inline_Hiccup": return { k: "hiccup", v: seg[1] };
    case "Inline_Html": return { k: "inline_html", text: seg[1] };
    case "Entity": {
      const e = seg[1] ?? {};
      return { k: "entity", name: e.name, latex: e.latex, latex_mathp: e.latex_mathp,
               html: e.html, ascii: e.ascii, unicode: e.unicode };
    }
    default: return { k: "inline:" + t, v: seg[1] };
  }
}

// ---- block ----------------------------------------------------------------

// One list item: block-shaped `content`, recursively-nested `items` (list-items,
// NOT blocks), and the def-list `name` (inline term). Matches lsdoc's ListItem.
export function normItem(it) {
  return {
    ordered: it.ordered, number: it.number, indent: it.indent,
    checkbox: it.checkbox,                          // false/true; absent (undefined) → no checkbox
    content: (it.content ?? []).map(normNode),
    items: (it.items ?? []).map(normItem),
    name: (it.name ?? []).map(normInline),
  };
}

export function normNode(node) {
  if (!Array.isArray(node)) return { kind: "raw", v: node };
  const t = node[0];
  switch (t) {
    case "Paragraph":
      return { kind: "paragraph", inline: (node[1] ?? []).map(normInline) };
    case "Heading": {
      const h = node[1] ?? {};
      const b = h.unordered
        ? { kind: "bullet", level: h.level }
        : { kind: "heading", level: h.level, size: h.size };
      // a bullet whose body is an ATX heading carries `size` (the #-count); plain
      // bullets have size:null in mldoc → omit so it matches lsdoc's skipped `None`.
      if (h.unordered && h.size != null) b.size = h.size;
      b.inline = (h.title ?? []).map(normInline);
      if (h.tags?.length) b.htags = h.tags;       // org `:tag1:tag2:` on a headline
      if (h.marker) b.marker = h.marker;          // org TODO/DOING/DONE/… (also md)
      if (h.priority) b.priority = h.priority;    // org `[#A]`
      return b;
    }
    case "List": {
      // ordered/explicit list: array of items, each with block content + NESTED
      // sub-items (mldoc folds a deeper-indented item into the preceding item's
      // `items` sub-array). `content` is block-shaped (normNode); `items` are
      // list-items (recurse via normItem, NOT normNode). `name` carries the markdown
      // definition-list term (empty for normal items).
      return { kind: "list", items: (node[1] ?? []).map(normItem) };
    }
    case "Src": {
      const s = node[1] ?? {};
      return { kind: "src", lang: s.language ?? "", code: (s.lines ?? []).join("") };
    }
    case "Quote":
      return { kind: "quote", children: (node[1] ?? []).map(normNode) };
    case "Custom":
      // ["Custom", name, _opts, [children], raw] — callout blocks (#+BEGIN_X);
      // BEGIN_QUOTE is emitted as Quote instead, so this is NOTE/TIP/WARNING/etc.
      return { kind: "custom", name: node[1], children: (node[3] ?? []).map(normNode) };
    case "Export": {
      const block = { kind: "export", name: node[1] ?? "", content: node[3] ?? "" };
      if (node[2] != null) block.options = node[2];
      return block;
    }
    case "CommentBlock":
      return { kind: "comment_block", content: (node[1] ?? []).join("") };
    case "Property_Drawer":
      return { kind: "properties", props: (node[1] ?? []).map((p) => [p[0], p[1]]) };
    case "Horizontal_Rule":
      return { kind: "hr" };
    case "Table": {
      const tb = node[1] ?? {};
      const cells = (rows) => (rows ?? []).map((r) => r.map((c) => (c ?? []).map(normInline)));
      return {
        kind: "table",
        header: tb.header ? tb.header.map((c) => (c ?? []).map(normInline)) : null,
        rows: (tb.groups ?? []).flatMap((g) => cells(g)),
      };
    }
    case "Footnote_Definition":
      return { kind: "footnote_def", name: node[1], inline: (node[2] ?? []).map(normInline) };
    case "Raw_Html":
      return { kind: "raw_html", text: node[1] };
    case "Displayed_Math":
      return { kind: "displayed_math", text: node[1] };
    case "Latex_Environment":
      // ["Latex_Environment", name, options(null), content]
      return { kind: "latex_env", name: node[1], content: node[3] };
    case "Directive":
      // org `#+KEY: value` -> ["Directive", key, value]
      return { kind: "directive", name: node[1], value: node[2] };
    case "Comment":
      // org `# text` -> ["Comment", text]
      return { kind: "comment", text: node[1] };
    case "Example":
      // org `#+BEGIN_EXAMPLE … #+END_EXAMPLE` -> ["Example", [lines…]]
      return { kind: "example", code: (node[1] ?? []).join("") };
    case "Hiccup":
      // block-level Clojure-hiccup `[:tag …]`: `v` is the RAW bracket text verbatim
      // (mldoc does not parse the children). Mirrors lsdoc's `Block::Hiccup { v }`.
      return { kind: "hiccup", v: node[1] };
    case "Drawer":
      // Content treated as opaque (logbook/clock metadata — not indexed/rendered);
      // compare on name only. See DECISIONS.md.
      return { kind: "drawer", name: node[1] };
    default:
      return { kind: "block:" + t, v: node[1] };
  }
}

export function normTop(item) {
  // item = [node, {start_pos, end_pos}]
  const [node, pos] = item;
  const b = normNode(node);
  b.span = pos ? [pos.start_pos, pos.end_pos] : null;
  return b;
}

// ---- cleanup (applied to BOTH mldoc + lsdoc projections) -------------------
// Removes cosmetic noise that isn't part of the observable contract: empty
// Plain "" segments, and link labels that are just an empty plain.

export function cleanInlines(arr) {
  if (!Array.isArray(arr)) return arr;
  const out = [];
  for (const seg of arr) {
    if (seg.k === "plain" && seg.text === "") continue;
    if (seg.children) seg.children = cleanInlines(seg.children);
    if (seg.label) {
      seg.label = cleanInlines(seg.label);
      if (seg.label.length === 0) delete seg.label;
    }
    out.push(seg);
  }
  return out;
}

// Clean one list item: block-shaped `content`, recursively-nested `items` (list-items,
// cleaned as items — NOT blocks), and the def-list term `name` (dropped when empty,
// since mldoc emits `name: []` for every non-def item while lsdoc omits it).
function cleanItem(it) {
  const cleaned = {
    ...it,
    content: (it.content ?? []).map(cleanBlock),
    items: (it.items ?? []).map(cleanItem),
  };
  const name = cleanInlines(it.name ?? []);
  if (name.length) cleaned.name = name;
  else delete cleaned.name;
  return cleaned;
}

export function cleanBlock(b) {
  if (!b || typeof b !== "object") return b;
  if (b.inline) b.inline = cleanInlines(b.inline);
  if (b.children) b.children = b.children.map(cleanBlock);
  if (b.items) b.items = b.items.map(cleanItem);
  // Table cells carry the same cosmetic empty-Plain noise (mldoc emits a `[Plain ""]`
  // for an empty cell `||`; lsdoc emits `[]`) — clean each cell the same way.
  if (b.kind === "table") {
    if (b.header) b.header = b.header.map(cleanInlines);
    if (b.rows) b.rows = b.rows.map((r) => r.map(cleanInlines));
  }
  return b;
}

export function normalizeAst(ast) {
  return (ast ?? []).map(normTop).map(cleanBlock);
}
