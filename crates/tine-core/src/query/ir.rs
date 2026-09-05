//! The ONE query intermediate representation (SPEC §3, I-12).
//!
//! A query is an **anchored filter**: one row type ([`Anchor`]), a boolean tree
//! ([`Filter`]), and leaves that are either a comparison on the anchor row's own
//! attributes or a quantifier over ONE of its relations. Three things produce it
//! (the legacy OG `{{query}}` parser, the TQL parser, the recognised
//! `#+BEGIN_QUERY` subset) and the walk, the SQL lowering, the builder and cache
//! invalidation all consume this one value — `Pred`, the TypeScript `Clause`
//! union and the datalog subset collapsed onto it.
//!
//! **The wire format is fixed by SPEC §3.1 and is not a lane's choice.** Every
//! enum is internally tagged on `kind` with `snake_case` names and every struct
//! field is `snake_case`, because the frontend mirror in
//! `src/editor/queryBuilder.ts` is hand-written against it and pinned by the
//! golden fixtures in `crates/tine-core/tests/fixtures/query-ir/` (read from
//! both sides).
//!
//! [`Span`] offsets are **UTF-16 code units** into the original source text: the
//! consumer is JavaScript, so the conversion from the byte offsets the parsers
//! compute happens once, here at the boundary ([`Span::from_byte_range`]).

use serde::{Deserialize, Serialize};

/// A source span, in UTF-16 code units into the ORIGINAL source text.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Span {
    pub start: u32,
    pub end: u32,
}

impl Span {
    /// Convert a byte range in `source` into UTF-16 code-unit offsets. Out-of-range
    /// or non-boundary inputs are clamped rather than panicking: a span is a
    /// presentation hint, never a correctness input.
    pub fn from_byte_range(source: &str, start: usize, end: usize) -> Span {
        let units = |byte: usize| -> u32 {
            let byte = byte.min(source.len());
            let mut count = 0usize;
            for (index, ch) in source.char_indices() {
                if index >= byte {
                    break;
                }
                count += ch.len_utf16();
            }
            u32::try_from(count).unwrap_or(u32::MAX)
        };
        let start_units = units(start);
        let end_units = units(end.max(start));
        Span {
            start: start_units,
            end: end_units.max(start_units),
        }
    }
}

/// The row type a query selects. Required in the IR and enforced by Tine (Q4).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Anchor {
    Block,
    Page,
}

/// The attributes of a block row and of a page row, plus the three attributes of
/// the elements relations yield (`name` for ref/tag elements, `key`/`value`/
/// `atom_count` for property elements). Which set is legal is decided by the row
/// the leaf sits on, never by a second enum (I-12).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Attr {
    // block row
    Content,
    Task,
    Priority,
    Scheduled,
    Deadline,
    // page row
    Name,
    Journal,
    Day,
    Namespace,
    // property element
    Key,
    Value,
    AtomCount,
}

/// The declared type of what a leaf compares (SPEC §4.2.3 operator × type matrix).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ValueType {
    Text,
    Number,
    Date,
    Checkbox,
    Ref,
}

impl ValueType {
    pub fn label(self) -> &'static str {
        match self {
            ValueType::Text => "text",
            ValueType::Number => "number",
            ValueType::Date => "date",
            ValueType::Checkbox => "checkbox",
            ValueType::Ref => "ref",
        }
    }
}

impl Attr {
    /// The fixed type of an attribute. `Value` (a property atom) has no fixed
    /// type — it is the property's effective type (§6) and is `None` here.
    pub fn fixed_type(self) -> Option<ValueType> {
        match self {
            Attr::Content
            | Attr::Task
            | Attr::Priority
            | Attr::Name
            | Attr::Namespace
            | Attr::Key => Some(ValueType::Text),
            Attr::Scheduled | Attr::Deadline | Attr::Day => Some(ValueType::Date),
            Attr::Journal => Some(ValueType::Checkbox),
            Attr::AtomCount => Some(ValueType::Number),
            Attr::Value => None,
        }
    }

    /// The TQL spelling of this attribute on the row it belongs to.
    pub fn tql_name(self) -> &'static str {
        match self {
            Attr::Content => "content",
            Attr::Task => "task",
            Attr::Priority => "priority",
            Attr::Scheduled => "scheduled",
            Attr::Deadline => "deadline",
            Attr::Name => "name",
            Attr::Journal => "journal",
            Attr::Day => "day",
            Attr::Namespace => "namespace",
            Attr::Key => "key",
            Attr::Value => "value",
            Attr::AtomCount => "atom_count",
        }
    }
}

/// One relation of the anchor row. Bare identifiers inside a relation predicate
/// bind to the ELEMENT, never to the outer row (SPEC §3.2).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Rel {
    /// OG `:block/path-refs`: this block's refs, every ancestor's, and its page.
    Refs,
    /// The block's own inline `#tag` / Org headline tags (Tine-only leaf, Q2).
    Tags,
    /// Property elements of the owner (block or page).
    Props,
    /// Direct children of a block (A1).
    Children,
    /// Every block of a page (`@page` anchor).
    Blocks,
    /// The owning page of a block (to-one).
    Page,
}

impl Rel {
    pub fn tql_name(self) -> &'static str {
        match self {
            Rel::Refs => "refs",
            Rel::Tags => "tags",
            Rel::Props => "props",
            Rel::Children => "children",
            Rel::Blocks => "blocks",
            Rel::Page => "page",
        }
    }
}

/// OData §5.1.1.13 quantifiers: `Any` is false and `Every` true on an empty
/// collection (Q5).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Quant {
    Any,
    None,
    Every,
}

/// The complete comparison vocabulary (SPEC §3.3, M19). `Contains`/`EndsWith`
/// are `Like` patterns; `StartsWith` is the range-lowerable `like 'p%'` case.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CmpOp {
    Eq,
    NotEq,
    Lt,
    Le,
    Gt,
    Ge,
    Between,
    In,
    NotIn,
    Like,
    StartsWith,
    /// Full-text match on `content`. The lowering (P1) is FTS5; the walk
    /// implements it with Tine's own friendly-search matcher, which is what
    /// today's `(search "…")` head already means.
    Match,
    /// A case-sensitive Rust regex over `content` — Tine's `(content-regex "…")`
    /// head (§4.1 "Tine's extensions accepted today"). Never OG-expressible and
    /// deliberately absent from TQL v1's vocabulary.
    Regex,
    IsSet,
    IsNotSet,
    IsBlank,
}

impl CmpOp {
    /// Whether this operator takes `Value::None` — and only `Value::None` (C1).
    pub fn is_presence(self) -> bool {
        matches!(self, CmpOp::IsSet | CmpOp::IsNotSet | CmpOp::IsBlank)
    }
}

/// A comparison operand. `Date` carries the UNRESOLVED literal (`-7d`,
/// `today`, `2026-09-04`); resolution happens at evaluation time in local time
/// from the evaluation's `today`, so a cached IR does not pin a day.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Value {
    Text {
        text: String,
    },
    Number {
        number: f64,
    },
    Date {
        literal: String,
    },
    Bool {
        #[serde(rename = "bool")]
        value: bool,
    },
    List {
        items: Vec<Value>,
    },
    /// The operand of `IsSet` / `IsNotSet` / `IsBlank` and of nothing else.
    None,
}

impl Value {
    pub fn text(value: impl Into<String>) -> Value {
        Value::Text { text: value.into() }
    }
    pub fn date(literal: impl Into<String>) -> Value {
        Value::Date {
            literal: literal.into(),
        }
    }
}

/// A self-contained test on the current row.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Leaf {
    /// A comparison on one of the current row's own attributes.
    Attr { attr: Attr, op: CmpOp, value: Value },
    /// A quantifier over ONE relation of the current row.
    Rel {
        rel: Rel,
        quant: Quant,
        pred: Box<Filter>,
    },
}

/// The boolean tree. Identity elements are explicit: `And([])` is [`Filter::True`]
/// and `Or([])` is [`Filter::False`] after [`Query::normalized`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Filter {
    And {
        items: Vec<Filter>,
    },
    Or {
        items: Vec<Filter>,
    },
    Not {
        inner: Box<Filter>,
    },
    Leaf {
        leaf: Leaf,
    },
    /// Q12: present, round-trips, structurally omitted at evaluation (§3.5).
    Off {
        inner: Box<Filter>,
    },
    /// An unparsed or unknown span. ALWAYS paired with a diagnostic.
    ///
    /// **Lossless by contract (§4.3.2, R4).** `text` is the exact UTF-8 payload
    /// the author wrote and `kind` is the diagnostic that rejected it; both
    /// survive every save, reopen and neighbouring edit, serialized as the
    /// `raw_hex('<kind>', '<hex>')` capsule. The `span` is presentation
    /// metadata and may be regenerated, and the disabled state is derived from
    /// the CURRENT tree (whether an `Off` encloses this node), never stored.
    Raw {
        text: String,
        /// The retained diagnostic kind. **Named `diagnostic_kind` on the wire**
        /// because `kind` is already this enum's internal tag (`"kind": "raw"`),
        /// which §3.1 fixes and a lane may not change.
        #[serde(rename = "diagnostic_kind")]
        kind: DiagnosticKind,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        span: Option<Span>,
    },
    True,
    False,
}

impl Filter {
    pub fn and(items: Vec<Filter>) -> Filter {
        Filter::And { items }
    }
    pub fn or(items: Vec<Filter>) -> Filter {
        Filter::Or { items }
    }
    pub fn not(inner: Filter) -> Filter {
        Filter::Not {
            inner: Box::new(inner),
        }
    }
    pub fn off(inner: Filter) -> Filter {
        Filter::Off {
            inner: Box::new(inner),
        }
    }
    pub fn leaf(leaf: Leaf) -> Filter {
        Filter::Leaf { leaf }
    }
    /// A preservation capsule: the exact payload plus the kind that rejected it
    /// (§4.3.2). Spans are attached separately because they are presentation.
    pub fn raw(text: impl Into<String>, kind: DiagnosticKind) -> Filter {
        Filter::Raw {
            text: text.into(),
            kind,
            span: None,
        }
    }
    pub fn attr(attr: Attr, op: CmpOp, value: Value) -> Filter {
        Filter::leaf(Leaf::Attr { attr, op, value })
    }
    pub fn rel(rel: Rel, quant: Quant, pred: Filter) -> Filter {
        Filter::leaf(Leaf::Rel {
            rel,
            quant,
            pred: Box::new(pred),
        })
    }

    /// The `Rel refs Any(name = x)` leaf both `[[x]]` and `#x` mean (Q2).
    pub fn page_ref(name: impl Into<String>) -> Filter {
        Filter::rel(
            Rel::Refs,
            Quant::Any,
            Filter::attr(Attr::Name, CmpOp::Eq, Value::text(name)),
        )
    }

    /// **Structural omission (§3.5, N1/M17).** `Off` subtrees are removed
    /// bottom-up BEFORE evaluation: an `And`/`Or` whose children are all removed
    /// is removed, `Not(<removed>)` is removed, and a `Rel` whose `pred` is
    /// entirely removed is removed (never `Any(True)`). `None` means "removed";
    /// a removed root is [`Filter::True`] at the call site.
    pub fn without_off(&self) -> Option<Filter> {
        match self {
            Filter::Off { .. } => None,
            // An ORIGINALLY EMPTY group is an active constant, not a group
            // emptied by disabling its children (§3.5). `or()` is false and
            // stays false; removing it would make a false query answer `True`.
            Filter::And { items } if items.is_empty() => Some(Filter::True),
            Filter::Or { items } if items.is_empty() => Some(Filter::False),
            Filter::And { items } => {
                let kept: Vec<Filter> = items.iter().filter_map(Filter::without_off).collect();
                (!kept.is_empty()).then(|| Filter::And { items: kept })
            }
            Filter::Or { items } => {
                let kept: Vec<Filter> = items.iter().filter_map(Filter::without_off).collect();
                (!kept.is_empty()).then(|| Filter::Or { items: kept })
            }
            Filter::Not { inner } => inner.without_off().map(Filter::not),
            Filter::Leaf {
                leaf: Leaf::Rel { rel, quant, pred },
            } => pred
                .without_off()
                .map(|pred| Filter::rel(*rel, *quant, pred)),
            other => Some(other.clone()),
        }
    }

    /// Whether any leaf anywhere in the tree (including inside `Off`) is a
    /// property leaf. Drives the registry-generation term of the cache key (C6).
    pub fn has_props_leaf(&self) -> bool {
        self.any_leaf(&mut |leaf| {
            matches!(
                leaf,
                Leaf::Rel {
                    rel: Rel::Props,
                    ..
                }
            )
        })
    }

    /// Depth-first existential over every leaf of the tree.
    pub fn any_leaf(&self, test: &mut impl FnMut(&Leaf) -> bool) -> bool {
        match self {
            Filter::Leaf { leaf } => {
                if test(leaf) {
                    return true;
                }
                match leaf {
                    Leaf::Rel { pred, .. } => pred.any_leaf(test),
                    Leaf::Attr { .. } => false,
                }
            }
            Filter::And { items } | Filter::Or { items } => {
                items.iter().any(|item| item.any_leaf(test))
            }
            Filter::Not { inner } | Filter::Off { inner } => inner.any_leaf(test),
            Filter::Raw { .. } | Filter::True | Filter::False => false,
        }
    }

    /// The key a `props` relation predicate selects. §3.3 writes every property
    /// form as `key = 'k'` conjoined with at most one atom test, so the key
    /// equality is what scopes the quantifier — this is the ONE reader of that
    /// convention (the walk, the candidate planner and both printers use it).
    pub fn props_key(&self) -> Option<String> {
        match self {
            Filter::Leaf {
                leaf:
                    Leaf::Attr {
                        attr: Attr::Key,
                        op: CmpOp::Eq,
                        value: Value::Text { text },
                    },
            } => Some(text.clone()),
            Filter::And { items } => items.iter().find_map(Filter::props_key),
            _ => None,
        }
    }

    /// Everything in a `props` predicate EXCEPT the key equality: the atom test,
    /// or `None` when the leaf is bare presence.
    pub fn props_atom_test(&self) -> Option<Filter> {
        match self {
            Filter::And { items } => {
                let rest: Vec<Filter> = items
                    .iter()
                    .filter(|item| item.props_key().is_none())
                    .cloned()
                    .collect();
                match rest.len() {
                    0 => None,
                    1 => rest.into_iter().next(),
                    _ => Some(Filter::And { items: rest }),
                }
            }
            other if other.props_key().is_some() => None,
            other => Some(other.clone()),
        }
    }

    /// The name a `refs` / `tags` relation predicate selects (`name = 'x'`, the
    /// only predicate v1 accepts inside those relations).
    pub fn ref_name(&self) -> Option<String> {
        match self {
            Filter::Leaf {
                leaf:
                    Leaf::Attr {
                        attr: Attr::Name,
                        op: CmpOp::Eq,
                        value: Value::Text { text },
                    },
            } => Some(text.clone()),
            _ => None,
        }
    }

    /// `normalized()`'s tree rule (§3.5, R2). **Omission-safe by construction.**
    ///
    /// In order: an ORIGINALLY EMPTY `And([])` / `Or([])` becomes `True` /
    /// `False` — including inside a relation predicate, because those are active
    /// constants and are exactly what a group emptied by disabling its children
    /// is not; then same-kind groups flatten, singleton groups collapse and
    /// `Off(Off(x))` collapses to one `Off`. Child order is preserved.
    ///
    /// **No boolean identity removal, absorption, De Morgan rewriting or
    /// constant folding happens here.** This is the stored, editable form: an
    /// operand the author wrote must still be there when the tree is printed
    /// back, and `Off` is structural omission rather than a truth value, so
    /// dropping a `True` next to an `Off` sibling silently changes the query's
    /// answer. `Or(False, Off(True))` stays false and `Not(And(True, Off(True)))`
    /// stays false; under the old identity-dropping rule both normalized to a
    /// fully disabled root, which evaluates as `True` (§3.5). Ordinary boolean
    /// reduction is legal only on the executable tree, AFTER
    /// [`Filter::without_off`].
    fn normalize_tree(&self) -> Filter {
        match self {
            // Originally empty: an active constant, before anything else.
            Filter::And { items } if items.is_empty() => Filter::True,
            Filter::Or { items } if items.is_empty() => Filter::False,
            Filter::And { items } => {
                let mut out: Vec<Filter> = Vec::new();
                for item in items {
                    match item.normalize_tree() {
                        Filter::And { items } => out.extend(items),
                        other => out.push(other),
                    }
                }
                match out.len() {
                    // Unreachable: a nonempty group's children each normalize to
                    // at least one operand, because nothing is dropped.
                    0 => Filter::True,
                    1 => out.into_iter().next().expect("one"),
                    _ => Filter::And { items: out },
                }
            }
            Filter::Or { items } => {
                let mut out: Vec<Filter> = Vec::new();
                for item in items {
                    match item.normalize_tree() {
                        Filter::Or { items } => out.extend(items),
                        other => out.push(other),
                    }
                }
                match out.len() {
                    0 => Filter::False,
                    1 => out.into_iter().next().expect("one"),
                    _ => Filter::Or { items: out },
                }
            }
            Filter::Not { inner } => Filter::not(inner.normalize_tree()),
            Filter::Off { inner } => match inner.normalize_tree() {
                // `off(off(x))` is one `off` (§4.3 K10).
                Filter::Off { inner } => Filter::Off { inner },
                other => Filter::off(other),
            },
            Filter::Leaf {
                leaf: Leaf::Rel { rel, quant, pred },
            } => Filter::rel(*rel, *quant, pred.normalize_tree()),
            // The payload and its kind are the preserved value; only the span is
            // presentation metadata (§4.3.2).
            Filter::Raw { text, kind, .. } => Filter::Raw {
                text: text.clone(),
                kind: *kind,
                span: None,
            },
            other => other.clone(),
        }
    }
}

/// Where a `Query` came from, and the bytes needed to re-emit it unchanged.
///
/// **Every macro-carried source has the same two fields** (§3.1, X4/W2): a
/// trailing options map is a property of being written inside a `{{…}}` macro,
/// not of the OG DSL, so TQL and advanced forms carry one too. `original` is the
/// exact form slice WITHOUT that map, and `og_options` is the map INCLUDING its
/// braces, verbatim and opaque — EDN comments and unknown keys and all. The map
/// has exactly ONE owner, the Rust parser: after this wave nothing outside
/// `query_parse` splits a query argument, and every macro printer re-appends it
/// once.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Source {
    /// The OG DSL, from a `{{query …}}` macro.
    Og {
        original: String,
        #[serde(default)]
        og_options: String,
    },
    /// TQL, from a `{{tine-query …}}` macro or the text pane.
    Tql {
        original: String,
        #[serde(default)]
        og_options: String,
    },
    /// Datalog. `original` is the COMPLETE authored advanced form, including
    /// `:query` / `:inputs` when present (§4.4) — a whole `{:query …}` map is
    /// the FORM, and only a map that FOLLOWS it is options.
    Advanced {
        original: String,
        #[serde(default)]
        og_options: String,
    },
    /// Built in the UI: no authored text to preserve.
    Builder,
}

impl Source {
    /// The opaque trailing options map, or `""` for [`Source::Builder`]. The ONE
    /// reader every macro printer uses, so the map cannot be re-derived
    /// per-dialect (I-12).
    pub fn og_options(&self) -> &str {
        match self {
            Source::Og { og_options, .. }
            | Source::Tql { og_options, .. }
            | Source::Advanced { og_options, .. } => og_options,
            Source::Builder => "",
        }
    }

    /// The exact authored form slice, without the trailing options map.
    pub fn original(&self) -> Option<&str> {
        match self {
            Source::Og { original, .. }
            | Source::Tql { original, .. }
            | Source::Advanced { original, .. } => Some(original),
            Source::Builder => None,
        }
    }
}

/// Why a query is (partly) not understood. Every kind names an in-scope
/// scenario: unknown vocabulary, malformed input, or an I-22 refusal.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DiagnosticKind {
    UnknownHead,
    Syntax,
    UnknownIdent,
    NotApplicable,
    Depth,
    Size,
}

impl DiagnosticKind {
    /// The `raw_hex('<kind>', …)` spelling of this kind (§4.3.2). The mapping is
    /// exact and total in both directions: a capsule that survives a save must
    /// come back as the same kind, never as a nearest guess.
    pub fn capsule_name(self) -> &'static str {
        match self {
            DiagnosticKind::UnknownHead => "unknown_head",
            DiagnosticKind::Syntax => "syntax",
            DiagnosticKind::UnknownIdent => "unknown_ident",
            DiagnosticKind::NotApplicable => "not_applicable",
            DiagnosticKind::Depth => "depth",
            DiagnosticKind::Size => "size",
        }
    }

    /// The inverse of [`DiagnosticKind::capsule_name`]. An unrecognised name is
    /// `None`, and the caller degrades the capsule to `Syntax` rather than
    /// inventing a kind or executing the payload.
    pub fn from_capsule_name(name: &str) -> Option<DiagnosticKind> {
        Some(match name {
            "unknown_head" => DiagnosticKind::UnknownHead,
            "syntax" => DiagnosticKind::Syntax,
            "unknown_ident" => DiagnosticKind::UnknownIdent,
            "not_applicable" => DiagnosticKind::NotApplicable,
            "depth" => DiagnosticKind::Depth,
            "size" => DiagnosticKind::Size,
            _ => return None,
        })
    }
}

/// Encode a [`Filter::Raw`] payload as the lowercase hexadecimal UTF-8 bytes the
/// `raw_hex('<kind>', '<hex>')` capsule carries (§4.3.2).
///
/// Hex is an INTERNAL preservation form: it never appears in the vocabulary
/// picker and the error renderer shows the decoded original text, never this.
pub fn encode_raw_hex(payload: &str) -> String {
    let mut out = String::with_capacity(payload.len() * 2);
    for byte in payload.as_bytes() {
        out.push(char::from_digit((byte >> 4) as u32, 16).expect("nibble"));
        out.push(char::from_digit((byte & 0x0f) as u32, 16).expect("nibble"));
    }
    out
}

/// Why a `raw_hex` capsule could not be decoded. Every one of these produces a
/// `Syntax` diagnostic and a preserved payload — never an executable predicate.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CapsuleError {
    /// The `<kind>` argument is not one of the six names.
    UnknownKind,
    /// Odd length, or a character that is not a hexadecimal digit.
    NotHex,
    /// The decoded bytes are not valid UTF-8.
    NotUtf8,
    /// The payload would exceed the source-size limit. Checked BEFORE allocating.
    TooLarge,
}

/// Decode a `raw_hex` capsule **strictly** (§4.3.2).
///
/// Even-length valid hexadecimal, valid UTF-8, and the source-size limit applied
/// **before** the payload is allocated — a hostile or corrupt capsule may not
/// make the decoder allocate proportionally to what it claims (I-22). Decoded
/// source is never evaluated or reparsed during execution.
pub fn decode_raw_hex(
    kind: &str,
    hex: &str,
    max_bytes: usize,
) -> Result<(DiagnosticKind, String), CapsuleError> {
    let kind = DiagnosticKind::from_capsule_name(kind).ok_or(CapsuleError::UnknownKind)?;
    if hex.len() % 2 != 0 {
        return Err(CapsuleError::NotHex);
    }
    // Size first: the byte count is `hex.len() / 2` before a single byte exists.
    if hex.len() / 2 > max_bytes {
        return Err(CapsuleError::TooLarge);
    }
    let digits = hex.as_bytes();
    let mut bytes = Vec::with_capacity(hex.len() / 2);
    for pair in digits.chunks_exact(2) {
        let high = (pair[0] as char).to_digit(16).ok_or(CapsuleError::NotHex)?;
        let low = (pair[1] as char).to_digit(16).ok_or(CapsuleError::NotHex)?;
        bytes.push((high * 16 + low) as u8);
    }
    let text = String::from_utf8(bytes).map_err(|_| CapsuleError::NotUtf8)?;
    Ok((kind, text))
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Diagnostic {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub span: Option<Span>,
    pub message: String,
    #[serde(default)]
    pub suggestions: Vec<String>,
    /// Set for a diagnostic inside an `Off` subtree: the row renders greyed with
    /// its message but does NOT invalidate the query (§3.5).
    #[serde(default)]
    pub disabled: bool,
    pub kind: DiagnosticKind,
}

impl Diagnostic {
    pub fn new(kind: DiagnosticKind, message: impl Into<String>) -> Diagnostic {
        Diagnostic {
            span: None,
            message: message.into(),
            suggestions: Vec::new(),
            disabled: false,
            kind,
        }
    }

    pub fn with_span(mut self, span: Option<Span>) -> Diagnostic {
        self.span = span;
        self
    }
}

/// A sort/group/column/aggregate target: a property key or an OG-sortable field
/// name, kept as the user wrote it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Field(pub String);

impl Field {
    pub fn new(name: impl Into<String>) -> Field {
        Field(name.into())
    }
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SortDir {
    Asc,
    Desc,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ViewKind {
    /// The existing fourth view (`Macro.tsx:108-109`), which stays.
    Search,
    List,
    Table,
    Board,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AggFn {
    Count,
    Sum,
    Avg,
}

/// Presentation. NEVER part of the filter (Q15): `sort-by`, `sample`,
/// `aggregate` and `group-by` are lifted here on parse and re-emitted from here
/// by the printers until P2 moves them to block properties.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ViewSettings {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub view: Option<ViewKind>,
    #[serde(default)]
    pub sort: Vec<(Field, SortDir)>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub group_by: Option<Field>,
    #[serde(default)]
    pub columns: Vec<Field>,
    #[serde(default)]
    pub aggregates: Vec<(Field, AggFn)>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sample: Option<u32>,
}

/// The two construction limits every result bridge already enforces
/// (`RESULT_BRIDGE_MAX_ROWS` / `RESULT_BRIDGE_MAX_BYTES`) — and nothing else.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Bounds {
    pub max_rows: usize,
    pub max_bytes: usize,
}

impl Bounds {
    pub fn unbounded() -> Bounds {
        Bounds {
            max_rows: usize::MAX,
            max_bytes: usize::MAX,
        }
    }
}

/// One query. The TypeScript mirror of this JSON lives in
/// `src/editor/queryBuilder.ts` and is pinned by the golden fixtures under
/// `crates/tine-core/tests/fixtures/query-ir/`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Query {
    pub anchor: Anchor,
    pub filter: Filter,
    #[serde(default)]
    pub diagnostics: Vec<Diagnostic>,
    pub source: Source,
}

impl Query {
    pub fn new(anchor: Anchor, filter: Filter, source: Source) -> Query {
        Query {
            anchor,
            filter,
            diagnostics: Vec::new(),
            source,
        }
    }

    /// A query with at least one ENABLED diagnostic is invalid: it returns zero
    /// results plus its diagnostics (§3.5). A diagnostic inside an `Off` subtree
    /// carries `disabled: true` and does not invalidate.
    pub fn is_invalid(&self) -> bool {
        self.diagnostics.iter().any(|d| !d.disabled)
    }

    /// **Semantic equality** (§3.5). Drops `source`, spans and diagnostic text;
    /// flattens nested `And`/`Or` of the same kind; removes identity elements;
    /// keeps child order and `Off` nodes. Round-trip tests compare these.
    pub fn normalized(&self) -> Query {
        Query {
            anchor: self.anchor,
            filter: self.filter.normalize_tree(),
            diagnostics: self
                .diagnostics
                .iter()
                .map(|d| Diagnostic {
                    span: None,
                    message: String::new(),
                    suggestions: Vec::new(),
                    disabled: d.disabled,
                    kind: d.kind,
                })
                .collect(),
            source: Source::Builder,
        }
    }

    /// The filter as the walk and the lowering evaluate it: `Off` removed
    /// bottom-up, a fully removed root becoming `True` (§3.5).
    pub fn evaluable_filter(&self) -> Filter {
        self.filter.without_off().unwrap_or(Filter::True)
    }
}

/// One `@page` result row. Needs no document load (K16).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PageRow {
    pub name: String,
    pub kind: crate::model::PageKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub journal_day: Option<i64>,
}

/// The advanced-query report, preserved verbatim from `AdvancedResult` (M5):
/// OG/TQL sources report an empty `ignored` and `supported = true`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct QueryReport {
    #[serde(default)]
    pub ran: Vec<String>,
    #[serde(default)]
    pub ignored: Vec<String>,
    pub supported: bool,
}

/// The anchor-specific result rows (SPEC §7.1, K16).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "anchor", rename_all = "snake_case")]
pub enum QueryRows {
    Block { groups: Vec<crate::model::RefGroup> },
    Page { pages: Vec<PageRow> },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QueryResult {
    #[serde(flatten)]
    pub rows: QueryRows,
    #[serde(default)]
    pub diagnostics: Vec<Diagnostic>,
    pub report: QueryReport,
    pub total: usize,
    pub exceeded: bool,
}

// ---------------------------------------------------------------------------
// Registry wire types (SPEC §6.1). The PRODUCER is P0-rust Wave B; the shapes
// live here because the golden fixtures and the TypeScript mirror pin them
// alongside the rest of the IR.
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ObservedType {
    Text,
    Number,
    Date,
    Checkbox,
    Ref,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Cardinality {
    One,
    Many,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RegistryRow {
    pub normalized_name: String,
    pub cardinality: Cardinality,
    pub observed_type: ObservedType,
    pub count_blocks: u64,
    pub count_pages: u64,
    /// Atom counts per class, in the `ObservedType` order above.
    #[serde(default)]
    pub histogram: Vec<(ObservedType, u64)>,
    pub mismatch_count: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub declared: Option<(ObservedType, Cardinality)>,
    /// At most eight, by count.
    #[serde(default)]
    pub top_values: Vec<(String, u64)>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RegistrySnapshot {
    pub rows: Vec<RegistryRow>,
    pub generation: u64,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn leaf_a() -> Filter {
        Filter::page_ref("a")
    }
    fn leaf_b() -> Filter {
        Filter::page_ref("b")
    }

    #[test]
    fn span_offsets_are_utf16_code_units_not_bytes() {
        // "😀" is 4 UTF-8 bytes and 2 UTF-16 code units; "é" is 2 and 1.
        let source = "😀é ab";
        assert_eq!(
            Span::from_byte_range(source, 0, 4),
            Span { start: 0, end: 2 }
        );
        assert_eq!(
            Span::from_byte_range(source, 4, 6),
            Span { start: 2, end: 3 }
        );
        assert_eq!(
            Span::from_byte_range(source, 7, 9),
            Span { start: 4, end: 6 }
        );
    }

    /// **R2.** Flattening is structural and stays; identity removal is gone.
    /// Both constants below are operands the author can see and disable, so
    /// erasing them would change what a disabled sibling means (§3.5).
    #[test]
    fn normalized_flattens_same_kind_groups_and_keeps_active_constants() {
        let query = Query::new(
            Anchor::Block,
            Filter::and(vec![
                Filter::and(vec![leaf_a(), Filter::True]),
                Filter::or(vec![Filter::False, leaf_b()]),
            ]),
            Source::Builder,
        );
        assert_eq!(
            query.normalized().filter,
            Filter::and(vec![
                leaf_a(),
                Filter::True,
                Filter::or(vec![Filter::False, leaf_b()]),
            ])
        );
    }

    #[test]
    fn normalized_empty_and_is_true_empty_or_is_false() {
        let and = Query::new(Anchor::Block, Filter::and(vec![]), Source::Builder);
        assert_eq!(and.normalized().filter, Filter::True);
        let or = Query::new(Anchor::Block, Filter::or(vec![]), Source::Builder);
        assert_eq!(or.normalized().filter, Filter::False);
    }

    /// §3.5: an originally empty group is an ACTIVE CONSTANT, including inside
    /// a relation predicate — distinct from a group emptied by disabling its
    /// children, which `without_off` removes entirely.
    #[test]
    fn an_originally_empty_group_inside_a_relation_is_an_active_constant() {
        let query = Query::new(
            Anchor::Block,
            Filter::rel(Rel::Children, Quant::Any, Filter::or(vec![])),
            Source::Builder,
        );
        assert_eq!(
            query.normalized().filter,
            Filter::rel(Rel::Children, Quant::Any, Filter::False)
        );
        // The disabled-children case is the contrast: removed, not `Any(True)`.
        let disabled = Filter::rel(Rel::Children, Quant::Any, Filter::off(leaf_a()));
        assert_eq!(disabled.without_off(), None);
    }

    #[test]
    fn normalized_collapses_nested_off() {
        let query = Query::new(
            Anchor::Block,
            Filter::off(Filter::off(leaf_a())),
            Source::Builder,
        );
        assert_eq!(query.normalized().filter, Filter::off(leaf_a()));
    }

    #[test]
    fn normalized_drops_source_and_spans_but_keeps_diagnostic_kinds() {
        let mut query = Query::new(
            Anchor::Block,
            Filter::Raw {
                text: "(frobnicate x)".into(),
                kind: DiagnosticKind::UnknownHead,
                span: Some(Span { start: 1, end: 5 }),
            },
            Source::Og {
                original: "(frobnicate x)".into(),
                og_options: String::new(),
            },
        );
        query
            .diagnostics
            .push(Diagnostic::new(DiagnosticKind::UnknownHead, "unknown head"));
        let normalized = query.normalized();
        assert_eq!(normalized.source, Source::Builder);
        assert_eq!(
            normalized.filter,
            Filter::raw("(frobnicate x)", DiagnosticKind::UnknownHead)
        );
        assert_eq!(normalized.diagnostics[0].message, "");
        assert_eq!(normalized.diagnostics[0].kind, DiagnosticKind::UnknownHead);
    }

    // --- §3.5 `Off` structural-omission truth cases ------------------------

    #[test]
    fn off_or_off_is_removed_entirely() {
        let filter = Filter::or(vec![Filter::off(leaf_a()), Filter::off(leaf_b())]);
        assert_eq!(filter.without_off(), None);
    }

    #[test]
    fn and_with_one_off_child_keeps_the_other() {
        let filter = Filter::and(vec![leaf_a(), Filter::off(leaf_b())]);
        assert_eq!(
            filter.without_off(),
            Some(Filter::and(vec![leaf_a()])),
            "an `and` keeps its enabled children and drops the disabled ones"
        );
    }

    #[test]
    fn not_of_off_is_removed_never_negated_true() {
        let filter = Filter::not(Filter::off(leaf_a()));
        assert_eq!(filter.without_off(), None);
    }

    #[test]
    fn relation_whose_predicate_is_entirely_off_is_removed_not_any_true() {
        let filter = Filter::rel(Rel::Children, Quant::Any, Filter::off(leaf_a()));
        assert_eq!(
            filter.without_off(),
            None,
            "`any(children, off(a))` must not degrade to `any(children, true)`"
        );
    }

    #[test]
    fn relation_predicate_keeps_its_enabled_conjunct() {
        let filter = Filter::rel(
            Rel::Children,
            Quant::Any,
            Filter::and(vec![leaf_a(), Filter::off(leaf_b())]),
        );
        assert_eq!(
            filter.without_off(),
            Some(Filter::rel(
                Rel::Children,
                Quant::Any,
                Filter::and(vec![leaf_a()])
            ))
        );
    }

    #[test]
    fn off_of_raw_is_removed_so_a_disabled_broken_row_still_returns_results() {
        let filter = Filter::off(Filter::raw("(frobnicate x)", DiagnosticKind::UnknownHead));
        assert_eq!(filter.without_off(), None);
    }

    #[test]
    fn a_fully_disabled_root_evaluates_as_true() {
        let query = Query::new(Anchor::Block, Filter::off(leaf_a()), Source::Builder);
        assert_eq!(query.evaluable_filter(), Filter::True);
    }

    #[test]
    fn a_disabled_diagnostic_does_not_invalidate_the_query() {
        let mut query = Query::new(Anchor::Block, Filter::off(leaf_a()), Source::Builder);
        let mut diagnostic = Diagnostic::new(DiagnosticKind::Syntax, "broken");
        diagnostic.disabled = true;
        query.diagnostics.push(diagnostic);
        assert!(!query.is_invalid());
        query
            .diagnostics
            .push(Diagnostic::new(DiagnosticKind::Syntax, "broken"));
        assert!(query.is_invalid());
    }

    #[test]
    fn has_props_leaf_sees_through_off_and_relations() {
        let filter = Filter::off(Filter::rel(
            Rel::Children,
            Quant::Any,
            Filter::rel(
                Rel::Props,
                Quant::Any,
                Filter::attr(Attr::Key, CmpOp::Eq, Value::text("k")),
            ),
        ));
        assert!(filter.has_props_leaf());
        assert!(!Filter::off(leaf_a()).has_props_leaf());
    }
}
