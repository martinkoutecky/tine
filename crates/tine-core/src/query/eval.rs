//! The walk's evaluation of a [`Query`] (SPEC §3.2–§3.5).
//!
//! **Two-valued leaves (Q5).** Every leaf is exactly true or false: an absent
//! optional attribute makes any comparison on it false, an atom that does not
//! coerce to the compared type fails the comparison, and `not`/`and`/`or` are
//! classical — so `(not (task DONE))` includes non-task blocks exactly as OG
//! does. `Any` over an empty relation is false and `Every` over an empty
//! relation is true (OData §5.1.1.13).
//!
//! **Property leaves quantify over the ATOMS of ONE key** (§3.3, Wave B): the
//! key equality conjunct scopes the relation, every source row of that key is
//! flattened into one atom list by the shared atomizer (§5.8), and each atom is
//! compared by the key's **effective type** from the registry (§6.3). Wave A's
//! `value_matches` — comma-split, ref-stripped, ASCII-lowercased, untyped — is
//! deleted; the corpus-parity export (O8) is what accounts for the difference.

use std::collections::HashMap;

use crate::config::ParseConfig;
use crate::date::JournalDate;
use crate::doc::{property_key_norm, DocBlock};
use crate::model::PageKind;
use crate::query::atom::{atom_key, Atom, AtomFormat};
use crate::query::ir::{Attr, CmpOp, Filter, Leaf, ObservedType, Quant, Rel, Value};
use crate::query::registry::Registry;
use crate::refs;
use crate::search_query::{canonical_fold, Matcher};

/// Per-page evaluation context: the page row a block row belongs to, plus the
/// evaluation's `today` (relative date literals stay unresolved in the IR).
///
/// `Copy` so one leaf can evaluate under a narrowed [`CompareMode`] without
/// rebuilding the context — see `eval_props`'s page-identity keys.
#[derive(Clone, Copy)]
pub(crate) struct EvalCtx<'a> {
    /// The page's journal-day ordinal (`yyyymmdd`), or `None` for named pages.
    pub(crate) journal: Option<i64>,
    pub(crate) is_journal: bool,
    pub(crate) page_name: &'a str,
    pub(crate) page_props: &'a [(String, String)],
    pub(crate) today: JournalDate,
    pub(crate) compiled: &'a CompiledLeaves,
    /// The page's on-disk format: the atomizer parses a property value with the
    /// page's own inline grammar (§6.2 E4), never with a guess.
    pub(crate) format: AtomFormat,
    /// The graph config the atomizer reads (comma-split keys, unparsed keys,
    /// journal title format).
    pub(crate) config: &'a ParseConfig,
    /// ONE coherent registry snapshot for the whole query, so every property
    /// leaf coerces at the same generation (§6.2).
    pub(crate) registry: &'a Registry,
    /// Which of the §8.1 counterfactual modes this evaluation runs under.
    /// [`CompareMode::Both`] is Tine and is what every product path uses; the
    /// other four exist so gate 1 can attribute a walk/OG difference to the
    /// decision that caused it.
    pub(crate) mode: crate::query::atom::CompareMode,
}

/// Patterns that cost real work to build (`(search …)`'s friendly matcher, a
/// `(content-regex …)` regex) compiled ONCE per query rather than per block.
/// The old `Pred` carried the compiled value inside the variant; the IR carries
/// only the user's text, so the compile cache lives here.
#[derive(Default)]
pub(crate) struct CompiledLeaves {
    matchers: HashMap<String, Matcher>,
    regexes: HashMap<String, Option<regex::Regex>>,
    folded: HashMap<String, String>,
}

impl CompiledLeaves {
    pub(crate) fn for_query(filter: &Filter) -> CompiledLeaves {
        let mut compiled = CompiledLeaves::default();
        collect_compiled(filter, &mut compiled);
        compiled
    }

    fn matcher(&self, source: &str) -> Option<&Matcher> {
        self.matchers.get(source)
    }
    fn regex(&self, source: &str) -> Option<&regex::Regex> {
        self.regexes.get(source).and_then(Option::as_ref)
    }
    fn fold(&self, text: &str) -> String {
        self.folded
            .get(text)
            .cloned()
            .unwrap_or_else(|| canonical_fold(text))
    }
}

fn collect_compiled(filter: &Filter, out: &mut CompiledLeaves) {
    filter.any_leaf(&mut |leaf| {
        if let Leaf::Attr {
            attr: Attr::Content,
            op,
            value: Value::Text { text },
        } = leaf
        {
            match op {
                CmpOp::Match => {
                    out.matchers
                        .entry(text.clone())
                        .or_insert_with(|| Matcher::parse(text));
                }
                CmpOp::Regex => {
                    out.regexes
                        .entry(text.clone())
                        .or_insert_with(|| regex::Regex::new(text).ok());
                }
                CmpOp::Like => {
                    out.folded
                        .entry(text.clone())
                        .or_insert_with(|| canonical_fold(text));
                }
                _ => {}
            }
        }
        false
    });
}

type PathRefCounts = HashMap<String, usize>;

/// Whether this query reads `:block/path-refs`, i.e. whether the walk has to
/// maintain the ancestor-ref counters at all.
pub(crate) fn uses_path_refs(filter: &Filter) -> bool {
    filter.any_leaf(&mut |leaf| matches!(leaf, Leaf::Rel { rel: Rel::Refs, .. }))
}

/// Evaluate a filter against one BLOCK row.
pub(crate) fn eval_block(
    filter: &Filter,
    block: &DocBlock,
    ancestor_refs: &PathRefCounts,
    ctx: &EvalCtx,
) -> bool {
    match filter {
        Filter::And { items } => items
            .iter()
            .all(|item| eval_block(item, block, ancestor_refs, ctx)),
        Filter::Or { items } => items
            .iter()
            .any(|item| eval_block(item, block, ancestor_refs, ctx)),
        Filter::Not { inner } => !eval_block(inner, block, ancestor_refs, ctx),
        Filter::True => true,
        // A `Raw` span is never satisfiable: the query carrying it is invalid and
        // returns nothing, and this keeps `not(<raw>)` from inventing matches.
        Filter::False | Filter::Raw { .. } => false,
        // `Off` is removed by `Query::evaluable_filter()` before evaluation
        // (§3.5); reaching one means a caller skipped that step.
        Filter::Off { .. } => {
            debug_assert!(false, "Off must be removed before evaluation (§3.5)");
            true
        }
        Filter::Leaf { leaf } => eval_block_leaf(leaf, block, ancestor_refs, ctx),
    }
}

fn eval_block_leaf(
    leaf: &Leaf,
    block: &DocBlock,
    ancestor_refs: &PathRefCounts,
    ctx: &EvalCtx,
) -> bool {
    match leaf {
        Leaf::Attr { attr, op, value } => match attr {
            Attr::Content => eval_content(*op, value, block, ctx),
            Attr::Task => eval_optional_text(*op, value, block.marker()),
            Attr::Priority => eval_optional_text(*op, value, block.priority()),
            Attr::Scheduled => {
                eval_planning(*op, value, block.projection().scheduled.as_deref(), ctx)
            }
            Attr::Deadline => {
                eval_planning(*op, value, block.projection().deadline.as_deref(), ctx)
            }
            // Page attributes only ever appear under a `page` relation, and the
            // property-element attributes only under `props`.
            _ => false,
        },
        Leaf::Rel { rel, quant, pred } => match rel {
            Rel::Refs => eval_refs(*quant, pred, block, ancestor_refs, ctx),
            Rel::Tags => quantify(*quant, block.projection().tags.iter(), |tag| {
                eval_name_element(pred, tag)
            }),
            Rel::Props => eval_props(*quant, pred, &block.properties(), ctx),
            Rel::Children => quantify(*quant, block.children.iter(), |child| {
                // A child is a fresh row: it carries no ancestor-ref context of
                // its own here, matching the direct-children-only rule (A1).
                eval_block(pred, child, ancestor_refs, ctx)
            }),
            // To-one: the page row is exactly one element, so all three
            // quantifiers reduce to the predicate (or its negation).
            Rel::Page => {
                let hit = eval_page(pred, ctx);
                match quant {
                    Quant::Any | Quant::Every => hit,
                    Quant::None => !hit,
                }
            }
            // `blocks` is a page-row relation; a block-anchored walk never sees it.
            Rel::Blocks => false,
        },
    }
}

/// Evaluate a filter against one PAGE row.
pub(crate) fn eval_page(filter: &Filter, ctx: &EvalCtx) -> bool {
    match filter {
        Filter::And { items } => items.iter().all(|item| eval_page(item, ctx)),
        Filter::Or { items } => items.iter().any(|item| eval_page(item, ctx)),
        Filter::Not { inner } => !eval_page(inner, ctx),
        Filter::True => true,
        Filter::False | Filter::Raw { .. } => false,
        Filter::Off { .. } => {
            debug_assert!(false, "Off must be removed before evaluation (§3.5)");
            true
        }
        Filter::Leaf { leaf } => match leaf {
            Leaf::Attr { attr, op, value } => match attr {
                Attr::Name => eval_page_name(*op, value, ctx.page_name),
                Attr::Journal => {
                    matches!(
                        (op, value),
                        (CmpOp::Eq, Value::Bool { value: true }) if ctx.is_journal
                    ) || matches!(
                        (op, value),
                        (CmpOp::Eq, Value::Bool { value: false }) if !ctx.is_journal
                    )
                }
                Attr::Day => eval_day(*op, value, ctx.journal, ctx.today),
                Attr::Namespace => {
                    // The immediate parent segment (Tine-only, M20).
                    let key = refs::normalize(ctx.page_name);
                    let parent = key.rsplit_once('/').map(|(head, _)| head.to_string());
                    eval_optional_text(*op, value, parent.as_deref())
                }
                _ => false,
            },
            Leaf::Rel { rel, quant, pred } => match rel {
                Rel::Props => eval_props(*quant, pred, ctx.page_props, ctx),
                // A page's own refs, its blocks and its tag table are not walked
                // by this evaluator: the OG DSL cannot express them and the
                // page-anchored walk of Wave A reads only the page index.
                _ => false,
            },
        },
    }
}

/// `Any` false / `Every` true on an empty collection (OData §5.1.1.13, Q5).
fn quantify<T>(
    quant: Quant,
    mut items: impl Iterator<Item = T>,
    mut test: impl FnMut(T) -> bool,
) -> bool {
    match quant {
        Quant::Any => items.any(&mut test),
        Quant::None => !items.any(&mut test),
        Quant::Every => items.all(&mut test),
    }
}

/// The predicate over a ref or tag element, whose only attribute is `name`.
fn eval_name_element(pred: &Filter, name: &str) -> bool {
    match pred {
        Filter::True => true,
        Filter::False => false,
        Filter::And { items } => items.iter().all(|item| eval_name_element(item, name)),
        Filter::Or { items } => items.iter().any(|item| eval_name_element(item, name)),
        Filter::Not { inner } => !eval_name_element(inner, name),
        Filter::Leaf {
            leaf:
                Leaf::Attr {
                    attr: Attr::Name,
                    op: CmpOp::Eq,
                    value: Value::Text { text },
                },
        } => refs::page_key(name) == refs::page_key(text),
        _ => false,
    }
}

/// `refs` is OG's `:block/path-refs`: this block's explicit and property refs,
/// every ancestor's, and the page it lives on. The `name = 'x'` predicate — the
/// only one v1 accepts — is answered by membership so the walk does not
/// materialize the closure per block.
fn eval_refs(
    quant: Quant,
    pred: &Filter,
    block: &DocBlock,
    ancestor_refs: &PathRefCounts,
    ctx: &EvalCtx,
) -> bool {
    if let Some(name) = single_ref_name(pred) {
        let normalized = refs::normalize(&name);
        let hit = block.projection().refs_contains_norm(&normalized)
            || ancestor_refs.contains_key(&normalized)
            || refs::normalize(ctx.page_name) == normalized;
        return match quant {
            Quant::Any | Quant::Every => hit,
            Quant::None => !hit,
        };
    }
    let names = block
        .projection()
        .refs_norm
        .iter()
        .cloned()
        .chain(ancestor_refs.keys().cloned())
        .chain(std::iter::once(refs::normalize(ctx.page_name)))
        .collect::<Vec<_>>();
    quantify(quant, names.iter(), |name| eval_name_element(pred, name))
}

/// The `name = 'x'` shape both `[[x]]` and `#x` produce.
fn single_ref_name(pred: &Filter) -> Option<String> {
    match pred {
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

// ---------------------------------------------------------------------------
// Property elements — atoms of ONE key, compared by the key's effective type
// ---------------------------------------------------------------------------

/// The property keys OG resolves to page references rather than to text, and
/// therefore compares by lower-cased page name in every mode.
const OG_PAGE_NAME_KEYS: [&str; 3] = ["tags", "alias", "aliases"];

/// The five forms of §3.3, evaluated over the owner's property rows.
///
/// The quantifier ranges over the **atoms of one key**, never over the owner's
/// `(key, value)` pairs: `k:: a` written twice plus `K:: b` is one atom list
/// `[a, b]` (§5.8's flattening rule), so `every(prop('k'), …)` sees the union a
/// user sees and not one row at a time.
fn eval_props(quant: Quant, pred: &Filter, properties: &[(String, String)], ctx: &EvalCtx) -> bool {
    // Every property leaf the parsers build carries the `key = 'k'` conjunct
    // (§3.3); without it the leaf names no relation to quantify over.
    let Some(key) = pred.props_key() else {
        return false;
    };
    let key_norm = property_key_norm(&key);
    let mut rows: Vec<&str> = Vec::new();
    let mut source_key: &str = key.as_str();
    for (name, value) in properties {
        if property_key_norm(name) == key_norm {
            source_key = name.as_str();
            rows.push(value.as_str());
        }
    }
    let present = !rows.is_empty();

    let Some(test) = pred.props_atom_test() else {
        // `prop('k') is not null` = Any(key='k'); `prop('k') is null` =
        // None(key='k'); property `Every` carries the presence conjunct (K2).
        return match quant {
            Quant::Any | Quant::Every => present,
            Quant::None => !present,
        };
    };

    let atoms = flatten_atoms(source_key, &rows, ctx);

    // `= ''` (IsBlank) and `all-page-tags`'s `atom_count > 0` are properties of
    // the whole atom list, not of one atom.
    if let Some(hit) = eval_atom_count_test(&test, present, atoms.len()) {
        return match quant {
            Quant::Any | Quant::Every => hit,
            Quant::None => !hit,
        };
    }

    // OG resolves `tags`, `alias` and `aliases` to PAGES, and a page's identity
    // in OG is its lower-cased `:block/name` — which is why `(page-tags genre)`
    // finds a page whose `tags:: Genre` (measured, `case.cljs`). That is page
    // identity, not property equality, so it folds case in EVERY mode; only
    // property equality is the case-SENSITIVE thing Q20 changes.
    let ctx = &if OG_PAGE_NAME_KEYS.contains(&key_norm.as_str()) {
        EvalCtx {
            mode: ctx.mode.folding_case(),
            ..*ctx
        }
    } else {
        *ctx
    };

    let effective = if ctx.mode.coerces_by_effective_type() {
        ctx.registry
            .effective_type(&key_norm)
            .unwrap_or(ObservedType::Text)
    } else {
        // §8.1: the OG-ward modes do not coerce. Every atom compares as OG
        // compares it, which `eval_atom_value` reads off the mode.
        ObservedType::Text
    };
    let matches = |atom: &Atom| eval_atom_test(&test, atom, effective, ctx);
    match quant {
        Quant::Any => atoms.iter().any(matches),
        Quant::None => !atoms.iter().any(matches),
        // Property `Every`: present, and no atom violates. An uncoercible atom
        // IS a violator, because every comparison on it is false (§3.3, §3.4).
        Quant::Every => present && atoms.iter().all(matches),
    }
}

/// §5.8's flattening: atomize each source row of the key in source order,
/// concatenate, de-duplicate by [`atom_key`] with first occurrence winning, and
/// renumber. The walk and the registry producer run the SAME rule (D-14) — this
/// is the walk's streaming form over an owner it already holds in memory.
fn flatten_atoms(source_key: &str, rows: &[&str], ctx: &EvalCtx) -> Vec<Atom> {
    let mut out: Vec<Atom> = Vec::new();
    for value in rows {
        for atom in crate::query::atom::property_atoms_in(
            source_key, value, ctx.format, ctx.config, ctx.mode,
        ) {
            if out.iter().any(|existing| existing.key == atom.key) {
                continue;
            }
            let ordinal = out.len() as u32;
            out.push(Atom { ordinal, ..atom });
        }
    }
    out
}

/// `Some(truth)` when the test reads only `atom_count`; `None` when it is an
/// atom-level test the quantifier has to range over.
fn eval_atom_count_test(test: &Filter, present: bool, count: usize) -> Option<bool> {
    match test {
        Filter::Leaf {
            leaf:
                Leaf::Attr {
                    attr: Attr::AtomCount,
                    op,
                    value: Value::Number { number },
                },
        } => {
            let count = count as f64;
            let hit = match op {
                CmpOp::Eq => count == *number,
                CmpOp::NotEq => count != *number,
                CmpOp::Gt => count > *number,
                CmpOp::Ge => count >= *number,
                CmpOp::Lt => count < *number,
                CmpOp::Le => count <= *number,
                _ => return None,
            };
            // Both `= ''` and `(all-page-tags)` are scoped by presence: a key
            // that is absent has no blank value and no tags.
            Some(present && hit)
        }
        _ => None,
    }
}

/// The predicate over ONE atom. `Attr::Value` is the atom; the boolean skeleton
/// is classical (§3.4).
fn eval_atom_test(test: &Filter, atom: &Atom, effective: ObservedType, ctx: &EvalCtx) -> bool {
    match test {
        Filter::True => true,
        Filter::False | Filter::Raw { .. } => false,
        Filter::And { items } => items
            .iter()
            .all(|item| eval_atom_test(item, atom, effective, ctx)),
        Filter::Or { items } => items
            .iter()
            .any(|item| eval_atom_test(item, atom, effective, ctx)),
        Filter::Not { inner } => !eval_atom_test(inner, atom, effective, ctx),
        Filter::Off { .. } => {
            debug_assert!(false, "Off must be removed before evaluation (§3.5)");
            true
        }
        Filter::Leaf {
            leaf: Leaf::Attr { attr, op, value },
        } => match attr {
            Attr::Value => eval_atom_value(*op, value, atom, effective, ctx),
            // A key equality reaching here is the leaf's own scoping conjunct,
            // already applied by `eval_props`.
            Attr::Key => true,
            _ => false,
        },
        Filter::Leaf { .. } => false,
    }
}

/// One `atom op value` comparison, coerced by the key's **effective type**
/// (§3.3, §6.3): a number key compares `num`, a date key compares `day`, and
/// text/ref/checkbox keys compare the NFC-lowercased [`atom_key`]. An atom whose
/// typed value is absent fails EVERY comparison, including `!=` (K3).
fn eval_atom_value(
    op: CmpOp,
    value: &Value,
    atom: &Atom,
    effective: ObservedType,
    ctx: &EvalCtx,
) -> bool {
    if op == CmpOp::IsSet {
        return true;
    }
    if !ctx.mode.coerces_by_effective_type() {
        return compare_atom_as_og(op, value, atom, ctx.mode);
    }
    match effective {
        ObservedType::Number => match atom.num {
            Some(num) => compare_number(op, value, num),
            None => false,
        },
        ObservedType::Date => match atom.day {
            Some(day) => compare_day(op, value, day, ctx.today),
            None => false,
        },
        // Text, ref and checkbox atoms all compare their NFC-lowercased text
        // (Q20). A ref atom's key IS the page-name key for every name without a
        // boundary slash, which is what makes `(property type [[Book]])` match
        // `type:: book`.
        _ => compare_atom_text(op, value, &atom.key),
    }
}

/// OG's own property comparison (SPEC §8.1, v13 Y3), for the four non-`both`
/// modes.
///
/// OG's rule, transcribed from what it actually does rather than from what it
/// looks like it does:
///
/// * both the stored value and the query's value pass through
///   `text/parse-non-string-property-value`, so a value matching `^\d+$` — and
///   ONLY that — becomes an integer on both sides (`01` is the integer 1, which
///   is why `k:: 01` answers `(property k 1)`);
/// * everything else compares as text, **case-sensitively**, refs included:
///   OG stores a ref as the page name AS WRITTEN, and `[[Book]]` and `[[book]]`
///   are two different strings in that set even though they are one page
///   (measured, `case.cljs`). Case-insensitive page identity belongs to the
///   `[[x]]` page-ref leaf, not to `(property k v)`.
///
/// Q20 and Q21 are already applied — or not — by the atomizer, so this reads
/// the mode only to decide whether the text comparison folds case.
fn compare_atom_as_og(
    op: CmpOp,
    value: &Value,
    atom: &Atom,
    mode: crate::query::atom::CompareMode,
) -> bool {
    /// OG `text/parse-non-string-property-value`'s integer branch: an unsigned
    /// run of ASCII digits, and nothing else. `1.5`, `-1` and `1,5` are text.
    fn og_integer(text: &str) -> Option<u64> {
        let trimmed = text.trim();
        (!trimmed.is_empty() && trimmed.chars().all(|c| c.is_ascii_digit()))
            .then(|| trimmed.parse::<u64>().ok())
            .flatten()
    }

    let literal = |value: &Value| -> Option<String> {
        match value {
            Value::Text { text } => Some(text.clone()),
            Value::Date { literal } => Some(literal.clone()),
            Value::Number { number } => Some(format_number(*number)),
            Value::Bool { value } => Some(value.to_string()),
            _ => None,
        }
    };
    let equal = |needle: &str| -> bool {
        match (og_integer(&atom.text), og_integer(needle)) {
            (Some(left), Some(right)) => left == right,
            _ => {
                crate::query::atom::atom_key_in(&atom.text, mode)
                    == crate::query::atom::atom_key_in(needle, mode)
            }
        }
    };
    match (op, value) {
        (CmpOp::In, Value::List { items }) => items
            .iter()
            .filter_map(|item| literal(item))
            .any(|needle| equal(&needle)),
        (CmpOp::NotIn, Value::List { items }) => !items
            .iter()
            .filter_map(|item| literal(item))
            .any(|needle| equal(&needle)),
        (op, value) => match literal(value) {
            None => false,
            Some(needle) => match op {
                CmpOp::Eq => equal(&needle),
                CmpOp::NotEq => !equal(&needle),
                _ => false,
            },
        },
    }
}

fn compare_number(op: CmpOp, value: &Value, num: f64) -> bool {
    let operand = |value: &Value| match value {
        Value::Number { number } => Some(*number),
        Value::Text { text } => text.trim().parse::<f64>().ok().filter(|n| n.is_finite()),
        Value::Date { literal } => literal.trim().parse::<f64>().ok().filter(|n| n.is_finite()),
        _ => None,
    };
    match (op, value) {
        (CmpOp::Between, Value::List { items }) if items.len() == 2 => {
            match (operand(&items[0]), operand(&items[1])) {
                (Some(low), Some(high)) => {
                    let (low, high) = if low > high { (high, low) } else { (low, high) };
                    num >= low && num <= high
                }
                _ => false,
            }
        }
        (CmpOp::In, Value::List { items }) => items.iter().any(|item| operand(item) == Some(num)),
        (CmpOp::NotIn, Value::List { items }) => {
            !items.iter().any(|item| operand(item) == Some(num))
        }
        (op, value) => match operand(value) {
            None => false,
            Some(bound) => match op {
                CmpOp::Eq => num == bound,
                CmpOp::NotEq => num != bound,
                CmpOp::Lt => num < bound,
                CmpOp::Le => num <= bound,
                CmpOp::Gt => num > bound,
                CmpOp::Ge => num >= bound,
                _ => false,
            },
        },
    }
}

fn compare_atom_text(op: CmpOp, value: &Value, key: &str) -> bool {
    let operand = |value: &Value| match value {
        Value::Text { text } => Some(atom_key(text)),
        Value::Number { number } => Some(atom_key(&format_number(*number))),
        Value::Date { literal } => Some(atom_key(literal)),
        Value::Bool { value } => Some(if *value {
            "true".into()
        } else {
            "false".into()
        }),
        _ => None,
    };
    match (op, value) {
        (CmpOp::In, Value::List { items }) => items
            .iter()
            .any(|item| operand(item).is_some_and(|item| item == key)),
        (CmpOp::NotIn, Value::List { items }) => !items
            .iter()
            .any(|item| operand(item).is_some_and(|item| item == key)),
        (CmpOp::Like, value) => operand(value).is_some_and(|pattern| like_matches(key, &pattern)),
        (CmpOp::StartsWith, value) => operand(value).is_some_and(|prefix| key.starts_with(&prefix)),
        (op, value) => match operand(value) {
            None => false,
            Some(operand) => match op {
                CmpOp::Eq => key == operand,
                // K3: `!=` is "coercible AND unequal"; a text atom always
                // coerces, so this is plain inequality on the comparison key.
                CmpOp::NotEq => key != operand,
                _ => false,
            },
        },
    }
}

/// A number written back as a comparison operand: integers without a `.0` tail,
/// so `prop('k') = 12` compares against the atom text `12`.
fn format_number(number: f64) -> String {
    if number.fract() == 0.0 && number.abs() < 1e15 {
        format!("{}", number as i64)
    } else {
        format!("{number}")
    }
}

// ---------------------------------------------------------------------------
// Attribute comparisons
// ---------------------------------------------------------------------------

fn eval_content(op: CmpOp, value: &Value, block: &DocBlock, ctx: &EvalCtx) -> bool {
    let Value::Text { text } = value else {
        return false;
    };
    let projection = block.projection();
    match op {
        CmpOp::Like => like_matches(&projection.visible_lower, &ctx.compiled.fold(text)),
        CmpOp::StartsWith => projection
            .visible_lower
            .starts_with(&ctx.compiled.fold(text)),
        CmpOp::Eq => projection.visible_lower == ctx.compiled.fold(text),
        CmpOp::NotEq => projection.visible_lower != ctx.compiled.fold(text),
        CmpOp::Match => ctx
            .compiled
            .matcher(text)
            .is_some_and(|m| m.matches(&projection.visible_lower, &projection.visible)),
        // An invalid regex is retained but deliberately matches nothing.
        CmpOp::Regex => ctx
            .compiled
            .regex(text)
            .is_some_and(|r| r.is_match(&projection.visible)),
        _ => false,
    }
}

/// A comparison on an OPTIONAL text attribute (`task`, `priority`, page
/// `namespace`). Absent makes every comparison false (§3.4).
fn eval_optional_text(op: CmpOp, value: &Value, actual: Option<&str>) -> bool {
    match (op, actual) {
        (CmpOp::IsSet, _) => actual.is_some(),
        (CmpOp::IsNotSet, _) => actual.is_none(),
        (_, None) => false,
        (CmpOp::Eq, Some(actual)) => match value {
            Value::Text { text } => actual.eq_ignore_ascii_case(text),
            _ => false,
        },
        (CmpOp::NotEq, Some(actual)) => match value {
            Value::Text { text } => !actual.eq_ignore_ascii_case(text),
            _ => false,
        },
        (CmpOp::In, Some(actual)) => match value {
            Value::List { items } => items.iter().any(|item| match item {
                Value::Text { text } => actual.eq_ignore_ascii_case(text),
                _ => false,
            }),
            _ => false,
        },
        (CmpOp::NotIn, Some(actual)) => match value {
            Value::List { items } => !items.iter().any(|item| match item {
                Value::Text { text } => actual.eq_ignore_ascii_case(text),
                _ => false,
            }),
            _ => false,
        },
        _ => false,
    }
}

fn eval_page_name(op: CmpOp, value: &Value, page_name: &str) -> bool {
    let key = refs::page_key(page_name);
    match (op, value) {
        (CmpOp::Eq, Value::Text { text }) => key == refs::page_key(text),
        (CmpOp::NotEq, Value::Text { text }) => key != refs::page_key(text),
        (CmpOp::StartsWith, Value::Text { text }) => key.starts_with(&page_prefix_key(text)),
        (CmpOp::Like, Value::Text { text }) => like_matches(&key, &canonical_fold(text)),
        (CmpOp::In, Value::List { items }) => items.iter().any(|item| match item {
            Value::Text { text } => key == refs::page_key(text),
            _ => false,
        }),
        _ => false,
    }
}

/// The page-identity fold applied to a PREFIX rather than to a whole name.
///
/// `refs::page_key` removes one slash at each boundary, because a page's
/// identity does not depend on them. A namespace prefix is precisely "this
/// name, then a boundary", so the trailing `/` carries the whole meaning and
/// has to survive the fold: without this, `(namespace Proj)` lowers to
/// `name starts-with "Proj/"`, folds to `"proj"`, and matches the page `Proj`
/// itself along with every page merely beginning with those letters.
fn page_prefix_key(text: &str) -> String {
    match text.strip_suffix('/') {
        Some(head) => format!("{}/", refs::page_key(head)),
        None => refs::page_key(text),
    }
}

/// A date comparison on a page's journal day ordinal.
fn eval_day(op: CmpOp, value: &Value, day: Option<i64>, today: JournalDate) -> bool {
    match op {
        CmpOp::IsSet => return day.is_some(),
        CmpOp::IsNotSet => return day.is_none(),
        _ => {}
    }
    let Some(day) = day else { return false };
    compare_day(op, value, day, today)
}

/// A planning-date comparison. **Presence IS a projected timestamp (G2):** the
/// walk reads `BlockProjection.scheduled`/`deadline`, which lsdoc fills only
/// from a `Timestamp` inline that starts a source line — so a bare `SCHEDULED:`
/// with no date, or one inside inline code or a fenced block, is no match, and a
/// malformed `<2026-13-45 …>` has presence but no day (E1). This replaces the
/// old raw-text `raw.contains("SCHEDULED:")` scan, which saw both.
fn eval_planning(op: CmpOp, value: &Value, text: Option<&str>, ctx: &EvalCtx) -> bool {
    match op {
        CmpOp::IsSet => return text.is_some(),
        CmpOp::IsNotSet => return text.is_none(),
        _ => {}
    }
    let Some(day) = text.and_then(planning_day) else {
        return false;
    };
    compare_day(op, value, day, ctx.today)
}

fn compare_day(op: CmpOp, value: &Value, day: i64, today: JournalDate) -> bool {
    let resolve = |value: &Value| match value {
        Value::Date { literal } => super::resolve_date_token(literal, today),
        Value::Number { number } => Some(*number as i64),
        _ => None,
    };
    match (op, value) {
        (CmpOp::Between, Value::List { items }) if items.len() == 2 => {
            // OG's `build-between-two-arg` sorts its two resolved bounds, so
            // `(between END START)` is the same inclusive interval.
            let (low, high) = (resolve(&items[0]), resolve(&items[1]));
            let (low, high) = match (low, high) {
                (Some(low), Some(high)) if low > high => (Some(high), Some(low)),
                pair => pair,
            };
            low.is_none_or(|low| day >= low) && high.is_none_or(|high| day <= high)
        }
        (CmpOp::Ge, value) => resolve(value).is_none_or(|bound| day >= bound),
        (CmpOp::Le, value) => resolve(value).is_none_or(|bound| day <= bound),
        (CmpOp::Gt, value) => resolve(value).is_some_and(|bound| day > bound),
        (CmpOp::Lt, value) => resolve(value).is_some_and(|bound| day < bound),
        (CmpOp::Eq, value) => resolve(value).is_some_and(|bound| day == bound),
        (CmpOp::NotEq, value) => resolve(value).is_some_and(|bound| day != bound),
        _ => false,
    }
}

/// The ONE timestamp-text → `yyyymmdd` primitive (D-14, J10), grown from the
/// walk's old `parse_angle_date`: it consumes the BRACKETLESS facet text exactly
/// as `doc::planning_dates` stores it on `BlockProjection::scheduled` and
/// `BlockProjection::deadline`, and an angle-bracketed caller strips the `<`
/// first.
///
/// **Calendar-validated (C5).** The old parser accepted `2026-13-45` because it
/// only read three integers. The month/day are now checked against the existing
/// `date.rs` `is_leap`/`days_in_month` (reused, never re-derived), so a
/// malformed timestamp has presence and no day.
pub(crate) fn planning_day(text: &str) -> Option<i64> {
    let text = text.trim();
    let text = text.strip_prefix('<').unwrap_or(text);
    let end = text.find([' ', '>']).unwrap_or(text.len());
    let mut parts = text[..end].split('-');
    let year: i64 = parts.next()?.parse().ok()?;
    let month: i64 = parts.next()?.parse().ok()?;
    let day: i64 = parts.next()?.parse().ok()?;
    if parts.next().is_some() {
        return None;
    }
    if !(1..=12).contains(&month) {
        return None;
    }
    let year_i32 = i32::try_from(year).ok()?;
    let month_u32 = u32::try_from(month).ok()?;
    if day < 1 || day > i64::from(crate::date::days_in_month(year_i32, month_u32)) {
        return None;
    }
    Some(year * 10000 + month * 100 + day)
}

/// SQL `LIKE` over an already-folded haystack: `%` matches any run, `_` any one
/// character, and `\` escapes either (the lowering emits `LIKE ? ESCAPE '\'`).
pub(crate) fn like_matches(haystack: &str, pattern: &str) -> bool {
    #[derive(Debug)]
    enum Part {
        Literal(String),
        Any,
        One,
    }
    let mut parts: Vec<Part> = Vec::new();
    let mut literal = String::new();
    let mut chars = pattern.chars();
    while let Some(ch) = chars.next() {
        match ch {
            '\\' => {
                if let Some(next) = chars.next() {
                    literal.push(next);
                }
            }
            '%' | '_' => {
                if !literal.is_empty() {
                    parts.push(Part::Literal(std::mem::take(&mut literal)));
                }
                parts.push(if ch == '%' { Part::Any } else { Part::One });
            }
            other => literal.push(other),
        }
    }
    if !literal.is_empty() {
        parts.push(Part::Literal(literal));
    }
    let haystack: Vec<char> = haystack.chars().collect();
    fn matches(parts: &[Part], haystack: &[char], at: usize) -> bool {
        match parts.first() {
            None => at == haystack.len(),
            Some(Part::One) => at < haystack.len() && matches(&parts[1..], haystack, at + 1),
            Some(Part::Any) => {
                (at..=haystack.len()).any(|next| matches(&parts[1..], haystack, next))
            }
            Some(Part::Literal(text)) => {
                let literal: Vec<char> = text.chars().collect();
                at + literal.len() <= haystack.len()
                    && haystack[at..at + literal.len()] == literal[..]
                    && matches(&parts[1..], haystack, at + literal.len())
            }
        }
    }
    matches(&parts, &haystack, 0)
}

/// Does this page row satisfy a `@page`-anchored query? Used by the page-anchored
/// walk, which reads the page index and loads no document (K16).
#[allow(clippy::too_many_arguments)]
pub(crate) fn page_row_matches(
    query: &Filter,
    name: &str,
    kind: PageKind,
    journal: Option<i64>,
    page_props: &[(String, String)],
    format: AtomFormat,
    today: JournalDate,
    compiled: &CompiledLeaves,
    config: &ParseConfig,
    registry: &Registry,
) -> bool {
    let ctx = EvalCtx {
        journal,
        is_journal: kind == PageKind::Journal,
        page_name: name,
        page_props,
        today,
        compiled,
        format,
        config,
        registry,
        mode: crate::query::atom::CompareMode::Both,
    };
    eval_page(query, &ctx)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn planning_day_accepts_the_bracketless_projection_text_and_the_angle_form() {
        assert_eq!(planning_day("2026-07-29 Wed"), Some(20260729));
        assert_eq!(planning_day("<2026-07-29 Wed>"), Some(20260729));
        assert_eq!(planning_day("2026-07-29"), Some(20260729));
    }

    #[test]
    fn planning_day_validates_the_calendar_so_a_malformed_date_has_no_day() {
        // C5: the old `parse_angle_date` answered 20261345 for the first of these.
        assert_eq!(planning_day("2026-13-45"), None);
        assert_eq!(planning_day("2026-02-30"), None);
        assert_eq!(planning_day("2023-02-29"), None);
        assert_eq!(planning_day("2026-04-31"), None);
        assert_eq!(planning_day("2024-02-29"), Some(20240229));
    }

    #[test]
    fn like_matches_wildcards_escapes_and_anchors() {
        assert!(like_matches("hello world", "%world%"));
        assert!(like_matches("hello world", "hello%"));
        assert!(!like_matches("hello world", "world%"));
        assert!(like_matches("a_b", "a\\_b"));
        assert!(!like_matches("axb", "a\\_b"));
        assert!(like_matches("axb", "a_b"));
        assert!(like_matches("100%", "%\\%"));
        assert!(like_matches("abc", "abc"));
        assert!(!like_matches("abcd", "abc"));
    }
}
