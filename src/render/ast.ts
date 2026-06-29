// lsdoc AST — TypeScript mirror of the FROZEN serde wire contract.
// ===========================================================================
// This is a HAND-MAINTAINED 1:1 mirror of lsdoc's serde-serialized parse tree,
// for deserializing what the Rust backend (`tine-core`) sends per block. The
// authoritative sources, in priority order, are:
//
//   1. /aux/koutecky/logseq/lsdoc/AST.md          — the render contract + vocab
//   2. /aux/koutecky/logseq/lsdoc/src/projection.rs — the Rust types (serde attrs)
//
// If either changes, THIS FILE MUST BE UPDATED BY HAND. There is no codegen.
// The contract is "frozen" only in the sense that every field is gated 0-diff
// against mldoc@1.5.7; new variants/fields can still be added (e.g. the `comment`
// block kind was added after AST.md's first draft) — keep this mirror in lockstep.
//
// ---------------------------------------------------------------------------
// THE SERDE ENCODING RULES (read before editing) — from AST.md §"Serde encoding"
// ---------------------------------------------------------------------------
//
//  * Enums are INTERNALLY TAGGED, but the discriminant key DIFFERS per enum:
//        Block  → "kind"      Inline → "k"      Url → "type"
//
//  * OMITTED == DEFAULT. Every Rust field with `skip_serializing_if` is dropped
//    from the JSON when it holds its default. So an ABSENT key must be read as:
//        Option<T>      absent → undefined  (None)
//        bool           absent → false
//        Vec<T>         absent → []         (empty array)
//        String         absent → ""         (empty string)
//    Consequently EVERY such field is typed `?` here, and each one documents the
//    default its absence implies. Do NOT "tidy" these into required fields.
//
//  * The fields that are ALWAYS present (no skip) are typed WITHOUT `?`. These
//    are the discriminant-carrying fields (the "Always-present fields" column in
//    AST.md): e.g. `inline` on a paragraph, `level` on a heading, `items` on a
//    list, `lang`/`code` on src, `ordered`/`indent`/`content`/`items` on a
//    ListItem. (Note: `size: Option<u32>` on Heading is ALSO always present —
//    serialized as `null` when None — because projection.rs does NOT skip it.
//    See the Heading comment.)
//
//  * `span` ([start,end] byte offsets) is emitted on blocks but is OUT OF THE
//    RENDER CONTRACT (excluded from the oracle diff). Tine renders read-only, so
//    it is ignored for rendering; typed here only for completeness. Inline nodes
//    carry NO span.
//
//  * Both Markdown and Org graphs produce the SAME AST. Format-specific origin
//    notes below ("md only" / "org only") describe what SOURCE produces a node,
//    not a shape difference.
// ===========================================================================

// ---------------------------------------------------------------------------
// Top level
// ---------------------------------------------------------------------------

/** One parse result: the block tree plus the OG-faithful ref set. The render
 *  path consumes `blocks`; the index path consumes `refs` (page/block names). */
export interface Projection {
  blocks: Block[];
  refs: Refs;
}

/** Extracted references, OG-faithful. `page` = `[[Page]]`/`#tag`/etc. names,
 *  `block` = `((uuid))` ids. Both arrays are always present (may be `[]`). */
export interface Refs {
  page: string[];
  block: string[];
}

/** Block source span `[startByte, endByte]`. Emitted but EXCLUDED from the
 *  render contract — do not rely on it for rendering. Inline nodes have none. */
export type Span = [number, number];

// ===========================================================================
// Block — discriminated union on "kind"
// ===========================================================================
//
// A single Tine block's raw content deserializes to a `Block[]` (a Projection's
// `blocks`). By convention the FIRST element is the block "header" — a `bullet`
// (md `-` / org `*`) or `heading` (page-level ATX/setext) — carrying the block's
// marker/priority/size/htags; subsequent elements are the body (paragraph, src,
// table, quote, list, properties, …). See ast-render-contract.md §"Block-header".
// ---------------------------------------------------------------------------

/** Block format selector — picks the parser (Markdown `-` / Org `*` bullet). */
export type Format = "md" | "org";

export type Block =
  | ParagraphBlock
  | HeadingBlock
  | BulletBlock
  | ListBlock
  | SrcBlock
  | QuoteBlock
  | CustomBlock
  | RawHtmlBlock
  | DisplayedMathBlock
  | DrawerBlock
  | DirectiveBlock
  | CommentBlock
  | ExampleBlock
  | LatexEnvBlock
  | PropertiesBlock
  | HrBlock
  | TableBlock
  | FootnoteDefBlock
  | HiccupBlock;

/** The discriminant string union, handy for exhaustive `switch` checks. */
export type BlockKind = Block["kind"];

/** A text paragraph. `inline` is the parsed inline run. */
export interface ParagraphBlock {
  kind: "paragraph";
  inline: Inline[];
  span?: Span;
}

/** An ATX `#…`/setext heading, or an org `*` headline mapped to a `#`-level.
 *  - `level` (always present): the heading level (1..6 for ATX).
 *  - `size`  (always present, may be `null`): the setext/ATX size — i.e. the raw
 *    `#`-count or setext rank. Serialized even when null (projection.rs does not
 *    skip it), so this field is REQUIRED here and may be `null`.
 *  Header fields (marker/priority/htags) appear when this heading is itself a
 *  block header. */
export interface HeadingBlock {
  kind: "heading";
  level: number;
  /** Always present; `null` when there is no setext/ATX size. */
  size: number | null;
  inline: Inline[];
  /** Task marker `TODO`/`DOING`/`DONE`/… — absent ⇒ no marker. */
  marker?: string;
  /** Org priority `[#A]` → `"A"` — absent ⇒ no priority. */
  priority?: string;
  /** Org headline tags `:tag1:tag2:` — absent ⇒ `[]` (no tags). */
  htags?: string[];
  span?: Span;
}

/** An outline bullet (md `-`) / org headline (`*`) — mldoc `Heading{unordered}`.
 *  This is the usual block-header node.
 *  - `level` (always present): outline/headline depth.
 *  - `size` (OPTIONAL, UNLIKE Heading.size): the heading level when the bullet's
 *    body is itself an ATX heading (`- ## Title` → 2), the raw `#`-count
 *    (uncapped — may exceed 6). ABSENT ⇒ the bullet body is not a heading.
 *  Header fields (marker/priority/htags) appear on the block header. */
export interface BulletBlock {
  kind: "bullet";
  level: number;
  /** ATX-heading level of the bullet body (`- ## T` → 2), uncapped; absent ⇒
   *  not a heading bullet. NOTE: optional here, but REQUIRED+nullable on Heading. */
  size?: number;
  inline: Inline[];
  /** Task marker — absent ⇒ no marker. */
  marker?: string;
  /** Org priority `[#A]` → `"A"` — absent ⇒ no priority. */
  priority?: string;
  /** Org headline tags — absent ⇒ `[]`. */
  htags?: string[];
  span?: Span;
}

/** A `*`/`+`/`N.` (md) or `-`/`+`/`N.` (org) list. NOTE: md `-` bullets are
 *  `BulletBlock`s (their own outline blocks), never `ListItem`s — so in-block
 *  lists use `+`/`*`/ordered in md, `-`/`+`/ordered in org. */
export interface ListBlock {
  kind: "list";
  items: ListItem[];
  span?: Span;
}

/** A fenced / `#+BEGIN_SRC` code block. `lang` may be `""` (untagged fence).
 *  A ```calc fence arrives here as `lang: "calc"` (Tine routes it to the
 *  calculator — see ast-render-contract.md). */
export interface SrcBlock {
  kind: "src";
  lang: string;
  code: string;
  span?: Span;
}

/** A `>` blockquote / `#+BEGIN_QUOTE`. `children` is a nested block tree. */
export interface QuoteBlock {
  kind: "quote";
  children: Block[];
  span?: Span;
}

/** A callout/admonition `#+BEGIN_X … #+END_X` where X != QUOTE
 *  (NOTE/TIP/WARNING/IMPORTANT/CAUTION/PINNED/CENTER/…). `name` is the X token
 *  (case as authored). mldoc emits `Custom`. NOTE: the GitHub-flavoured markdown
 *  callout `> [!NOTE]` is NOT this — that parses as a `quote`; see RISKS. */
export interface CustomBlock {
  kind: "custom";
  name: string;
  children: Block[];
  span?: Span;
}

/** Block-level raw HTML. `text` is the verbatim HTML. (Inline raw HTML is the
 *  `inline_html` Inline instead.) */
export interface RawHtmlBlock {
  kind: "raw_html";
  text: string;
  span?: Span;
}

/** A block-level `$$ … $$` (mldoc `Displayed_Math`). `text` is the raw TeX.
 *  (Inline `$$…$$` mixed with text is a `latex` Inline with mode "Displayed".) */
export interface DisplayedMathBlock {
  kind: "displayed_math";
  text: string;
  span?: Span;
}

/** An org drawer `:NAME: … :END:` (e.g. `:LOGBOOK:`). Content is OPAQUE — only
 *  the drawer `name` is carried (compared on name only). Renders to nothing. */
export interface DrawerBlock {
  kind: "drawer";
  name: string;
  span?: Span;
}

/** An org keyword line `#+KEY: value` (mldoc `Directive`). Page-level directives
 *  are usually surfaced as page properties; a block-level one is metadata. */
export interface DirectiveBlock {
  kind: "directive";
  name: string;
  value: string;
  span?: Span;
}

/** An org comment line `# text` (mldoc `Comment`). `text` is the raw content
 *  after `# ` (leading stripped, trailing kept). NOT inline-parsed, NOT rendered. */
export interface CommentBlock {
  kind: "comment";
  text: string;
  span?: Span;
}

/** An org `#+BEGIN_EXAMPLE … #+END_EXAMPLE` / fixed-width `:` block (mldoc
 *  `Example`). `code` is the literal content. */
export interface ExampleBlock {
  kind: "example";
  code: string;
  span?: Span;
}

/** A LaTeX environment block `\begin{X} … \end{X}` (mldoc `Latex_Environment`).
 *  `name` is lowercased; `content` is the full environment body. */
export interface LatexEnvBlock {
  kind: "latex_env";
  name: string;
  content: string;
  span?: Span;
}

/** A `key:: value` (md) / org `:PROPERTIES:` properties block. `props` is an
 *  ORDERED list of `[key, value]` pairs (order preserved from source). This is
 *  its OWN block — properties are NOT attached to the header node. Tine filters
 *  internal keys (id/collapsed/…) before rendering. */
export interface PropertiesBlock {
  kind: "properties";
  props: Array<[string, string]>;
  span?: Span;
}

/** A horizontal rule. No fields besides the (excluded) span. */
export interface HrBlock {
  kind: "hr";
  span?: Span;
}

/** A table.
 *  - `header` (always present, may be `null`): the header row, a list of cells,
 *    each cell an inline run. `null` ⇒ no header row.
 *  - `rows` (always present): the body rows; each row a list of cells; each cell
 *    an inline run. (May be `[]`.)
 *  NO COLUMN ALIGNMENT is available — mldoc 1.5.7 discards it. See RISKS: Tine's
 *  current renderer applies `:--`/`--:` alignment, which the AST cannot provide. */
export interface TableBlock {
  kind: "table";
  /** Header row (cells × inline-runs), or `null` for a header-less table. */
  header: Inline[][] | null;
  /** Body rows (rows × cells × inline-runs). Always present; may be `[]`. */
  rows: Inline[][][];
  span?: Span;
}

/** A footnote definition `[^id]: body` / org `[fn:id] body`. `name` is the id,
 *  `inline` is the definition body. */
export interface FootnoteDefBlock {
  kind: "footnote_def";
  name: string;
  inline: Inline[];
  span?: Span;
}

/** A Clojure-hiccup block `[:tag …]` (lsdoc v0.1.4). `v` is the raw bracket text,
 *  verbatim — children are NOT parsed (opaque), and no refs come from it. We render
 *  it as literal text for now (OG renders it as real HTML — a faithful hiccup→HTML
 *  transform is a possible later upgrade). Absent from every real graph; an edge
 *  construct. */
export interface HiccupBlock {
  kind: "hiccup";
  v: string;
  span?: Span;
}

// ===========================================================================
// ListItem — an element of ListBlock.items
// ===========================================================================

export interface ListItem {
  /** Always present. `true` for `N.`/`N)` numbered items, else `false`. */
  ordered: boolean;
  /** The explicit number for `N.` items; absent ⇒ unnumbered. */
  number?: number;
  /** Always present. Leading-whitespace columns (drives nesting). */
  indent: number;
  /** Always present. The item body — usually a single `paragraph`. */
  content: Block[];
  /** Always present. Nested child items (may be `[]`). */
  items: ListItem[];
  /** Markdown definition-list term (`term\n: def`). Absent ⇒ `[]` (not a def). */
  name?: Inline[];
  /** Task checkbox: `[ ]`→`false`, `[x]`/`[X]`→`true`. Absent ⇒ no checkbox.
   *  (md `-` bullets are BulletBlocks, never list items, so never carry one.) */
  checkbox?: boolean;
}

// ===========================================================================
// Inline — discriminated union on "k"
// ===========================================================================

export type Inline =
  | PlainInline
  | CodeInline
  | VerbatimInline
  | BreakInline
  | HardBreakInline
  | EmphasisInline
  | SubscriptInline
  | SuperscriptInline
  | LinkInline
  | NestedLinkInline
  | TargetInline
  | TagInline
  | MacroInline
  | LatexInline
  | TimestampInline
  | FnrefInline
  | InlineHtmlInline
  | EmailInline
  | EntityInline
  | HiccupInline;

/** The discriminant string union, handy for exhaustive `switch` checks. */
export type InlineKind = Inline["k"];

/** Literal text. Wrap in <EmojiText> when rendering (Twemoji parity). */
export interface PlainInline {
  k: "plain";
  text: string;
}

/** Inline code `` `code` `` / org `~code~`. Literal (no nested parse). */
export interface CodeInline {
  k: "code";
  text: string;
}

/** Org `=verbatim=`. Distinct node from `code`; renders the same (`<code>`). */
export interface VerbatimInline {
  k: "verbatim";
  text: string;
}

/** A SOFT line break (`keep_line_break`). Renders as whitespace (a space), NOT
 *  a `<br>` — see ast-render-contract.md. No fields. */
export interface BreakInline {
  k: "break";
}

/** A HARD break (trailing `\` or two spaces). Renders as `<br>`. No fields. */
export interface HardBreakInline {
  k: "hardbreak";
}

/** Emphasis. `emph` is one of the EXACT strings:
 *    "Bold" | "Italic" | "Strike_through" | "Highlight" | "Underline"
 *  (md emits the first four; org adds Underline). `children` is the inner run;
 *  nesting is real, e.g. `***x***` → Italic[ Bold[x] ]. */
export interface EmphasisInline {
  k: "emphasis";
  emph: EmphKind;
  children: Inline[];
}

/** The exact `emphasis.emph` vocabulary (AST.md §Vocabularies). */
export type EmphKind =
  | "Bold"
  | "Italic"
  | "Strike_through"
  | "Highlight"
  | "Underline";

/** Org subscript `_x` / `_{x}` (mldoc Subscript). `children` re-parsed inline. */
export interface SubscriptInline {
  k: "subscript";
  children: Inline[];
}

/** Org superscript `^x` / `^{x}` (mldoc Superscript). `children` re-parsed. */
export interface SuperscriptInline {
  k: "superscript";
  children: Inline[];
}

/** A link / image / page-ref / block-ref / autolink — the render-critical node.
 *  See the Url union for the destination shape. Image-ness is the `image` flag
 *  (no need to sniff `full`); size from `metadata`; tooltip from `title`. */
export interface LinkInline {
  k: "link";
  /** Always present. The destination. */
  url: Url;
  /** Always present. Raw source text of the link (incl. leading `!` / metadata). */
  full: string;
  /** The rendered label (inline children). Absent ⇒ `[]` (bare ref: render the
   *  destination itself, e.g. `[[Page]]` shows the page name). */
  label?: Inline[];
  /** `![…](…)` markdown image. Present only when `true`; absent ⇒ `false` (link).
   *  Derived from `full`'s leading `!`; md only. */
  image?: boolean;
  /** Raw Logseq media metadata `{:width … :height …}` (braces included). Absent
   *  ⇒ `""` (no metadata). Parse with the same EDN-ish reader Tine already uses. */
  metadata?: string;
  /** CommonMark link title `[l](u "title")` — raw inner (no quotes, not
   *  unescaped). Absent ⇒ no title. */
  title?: string;
}

/** Logseq nested page ref `[[a [[b]] c]]` — the raw inner is kept verbatim in
 *  `content`. (Current renderer has no equivalent; see RISKS.) */
export interface NestedLinkInline {
  k: "nested_link";
  content: string;
}

/** Org dedicated/radio target `<<name>>` (mldoc Target). Renders as its text. */
export interface TargetInline {
  k: "target";
  text: string;
}

/** A `#tag` / `#[[bracket tag]]`. `children` is the tag name as an inline run
 *  (so `#[[a b]]` keeps spaces). The ref name for routing is the joined text. */
export interface TagInline {
  k: "tag";
  children: Inline[];
}

/** A `{{name arg1, arg2}}` macro (incl. `{{embed …}}`, `{{query …}}`).
 *  `name` is the macro name, `args` are the mldoc-split args (NOT a raw body —
 *  args were split honouring `[[..]]`, `((..))`, `"..."`; commas elsewhere
 *  split). To reconstruct a body string for a query/embed/user-macro that wants
 *  one, use `name + " " + args.join(", ")`. See RISKS for the round-trip caveat. */
export interface MacroInline {
  k: "macro";
  name: string;
  /** Always present; may be `[]` for an argument-less macro. */
  args: string[];
}

/** Inline LaTeX. `mode` ∈ {"Inline","Displayed"} (the EXACT strings). `$x$` /
 *  `\(x\)` → Inline; `$$x$$` / `\[x\]` → Displayed. `body` is the raw TeX. */
export interface LatexInline {
  k: "latex";
  mode: LatexMode;
  body: string;
}

/** The exact `latex.mode` vocabulary. */
export type LatexMode = "Inline" | "Displayed";

/** An org timestamp. `ts` ∈ {"Date","Range","Scheduled","Deadline","Closed"}
 *  (the EXACT strings). `date` is mldoc's RAW date/range record, declared OPAQUE
 *  (see TimestampValue). SCHEDULED/DEADLINE/CLOSED and ranges all flow through
 *  here — the `ts` tag distinguishes them; there is no separate heading-meta
 *  field, so planner badges render from this inline. */
export interface TimestampInline {
  k: "timestamp";
  ts: TimestampTs;
  date: TimestampValue;
}

/** The exact `timestamp.ts` vocabulary. */
export type TimestampTs = "Date" | "Range" | "Scheduled" | "Deadline" | "Closed";

/** A `{date,wday,active,time?,repetition?}` calendar point (mldoc raw record). */
export interface TimestampPoint {
  /** The calendar date. */
  date: { year: number; month: number; day: number };
  /** Weekday string as mldoc emits it (e.g. "Fri"); may be absent/empty. */
  wday?: string;
  /** `true` for active `<…>`, `false` for inactive `[…]`. Drives badge styling. */
  active?: boolean;
  /** Optional time-of-day. */
  time?: { hour: number; min: number };
  /** Optional repeater (e.g. `+1w`); shape is mldoc-raw — treat as opaque. */
  repetition?: unknown;
}

/** `timestamp.date` is OPAQUE for rendering (passed through as mldoc JSON).
 *  - For `ts` ∈ {Date,Scheduled,Deadline,Closed}: a single `TimestampPoint`.
 *  - For `ts: "Range"`: `{ start: TimestampPoint, stop: TimestampPoint }`.
 *  A renderer that needs a display string must FORMAT this record itself — the
 *  AST carries NO raw timestamp string (see RISKS). Typed loosely on purpose. */
export type TimestampValue =
  | TimestampPoint
  | { start: TimestampPoint; stop: TimestampPoint }
  // Fallback: anything mldoc emits that doesn't match the shapes above. Always
  // guard with runtime checks before reading fields.
  | Record<string, unknown>;

/** A footnote reference `[^id]` / `[fn:id]`. `name` is the id. */
export interface FnrefInline {
  k: "fnref";
  name: string;
}

/** Inline raw HTML, e.g. `<span class="x">…</span>` / org `@@html:…@@`
 *  (mldoc Inline_Html). `text` is verbatim. Tine re-detects the `<iframe>`
 *  subset from this (https-only sandbox); other raw HTML renders as text. */
export interface InlineHtmlInline {
  k: "inline_html";
  text: string;
}

/** An email autolink `<a@b.com>` (mldoc Email). `text` is mldoc's RAW address
 *  record, declared OPAQUE (see EmailValue). */
export interface EmailInline {
  k: "email";
  text: EmailValue;
}

/** `email.text` is OPAQUE (mldoc's address record). mldoc typically emits a
 *  `{local_part, domain, …}`-shaped object; the address string can be
 *  reconstructed from its parts. Typed loosely — guard before reading. */
export type EmailValue =
  | { local_part?: string; domain?: string; [k: string]: unknown }
  | string
  | Record<string, unknown>;

/** A LaTeX named entity `\Delta` / `\Delta{}` (mldoc Entity), resolved from
 *  lsdoc's 339-entry table. Carries every mldoc field. Render `unicode` (e.g.
 *  "Δ") for display, or `html` for an HTML entity, per the renderer's choice.
 *  NOTE: the current renderer has NO entity handling — `\Delta` renders literal
 *  today; this node CHANGES that. See RISKS. */
export interface EntityInline {
  k: "entity";
  name: string;
  /** The LaTeX source, e.g. "\\Delta". */
  latex: string;
  /** Whether the latex must be in math mode. */
  latex_mathp: boolean;
  /** HTML entity, e.g. "&Delta;". */
  html: string;
  /** ASCII fallback. */
  ascii: string;
  /** The resolved Unicode glyph, e.g. "Δ" — the usual thing to render. */
  unicode: string;
}

/** An inline Clojure-hiccup `[:tag …]` (lsdoc v0.1.4). `v` is the raw bracket text,
 *  verbatim (children unparsed, no refs). Rendered as literal text for now — see
 *  [[HiccupBlock]] for the OG-HTML upgrade note. Edge construct, absent from real graphs. */
export interface HiccupInline {
  k: "hiccup";
  v: string;
}

// ===========================================================================
// Url — discriminated union on "type" (the destination of a LinkInline)
// ===========================================================================

export type Url =
  | PageRefUrl
  | BlockRefUrl
  | SearchUrl
  | FileUrl
  | ComplexUrl;

/** The discriminant string union. */
export type UrlType = Url["type"];

/** `[[Page]]` — a page reference. `v` is the page name. */
export interface PageRefUrl {
  type: "page_ref";
  v: string;
}

/** `((uuid))` — a block reference. `v` is the block id. */
export interface BlockRefUrl {
  type: "block_ref";
  v: string;
}

/** A bare/relative destination with no protocol (incl. most image paths,
 *  e.g. `../assets/x.png`). `v` is the raw destination. */
export interface SearchUrl {
  type: "search";
  v: string;
}

/** An org `file:…` destination. `v` is the path (without the `file:` scheme as
 *  mldoc strips it; confirm against source if it matters). */
export interface FileUrl {
  type: "file";
  v: string;
}

/** A `proto://…` URL (http(s), mailto via `<…>` handled as email, etc.).
 *  Both fields are optional in the Rust type (Option<String>); reconstruct the
 *  href as `protocol + "://" + link` when both are present, else fall back to
 *  whatever is available / the link's `full`. */
export interface ComplexUrl {
  type: "complex";
  /** e.g. "https". Absent ⇒ unknown protocol. */
  protocol?: string;
  /** The protocol-relative remainder, e.g. "example.com/x". Absent ⇒ none. */
  link?: string;
}
