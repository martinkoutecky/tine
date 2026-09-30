//! The in-memory evaluation of a resolved query [`Filter`] (SPEC §3.2–§3.5),
//! promoted from master's walk evaluator (`tine-core/src/query/eval.rs`), which
//! master compiles only as the SQL lowering's test oracle. og has no projection
//! database, so this IS og's production evaluator (I-12: one answerer).
//!
//! **Two-valued leaves (Q5).** Every leaf is exactly true or false: an absent
//! optional attribute makes any comparison on it false, an atom that does not
//! coerce to the compared type fails the comparison, and `not`/`and`/`or` are
//! classical — so `(not (task DONE))` includes non-task blocks exactly as OG
//! does. `Any` over an empty relation is false and `Every` over an empty
//! relation is true (OData §5.1.1.13).
//!
//! **Property leaves quantify over the ATOMS of ONE key** (§3.3): every source
//! row of that key is flattened into one atom list by the shared atomizer
//! (§5.8), and each atom is compared by the key's **effective type** from the
//! registry (§6.3). Only master's production comparison mode (`Both`) exists
//! here; master's four counterfactual modes are an oracle-attribution device
//! with no product caller, so they were not ported.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use tine_core::date::JournalDate;
use tine_core::doc::{property_key_norm, DocBlock};
use tine_core::model::PageKind;
use tine_core::query::atom::{
    atom_key, format_number, property_atoms, Atom, AtomDeduper, AtomFormat, ParseConfig,
};
use tine_core::query::ir::{Attr, CmpOp, Filter, Leaf, ObservedType, Quant, Rel, Value};
use tine_core::query::path_refs::{closure_contains, closure_names, dfs_path_refs, PathRefCounts};
use tine_core::query::registry::Registry;
use tine_core::query::text::LikePattern;
use tine_core::refs;
use tine_core::search_query::{canonical_fold, literal_fold, Matcher};
use unicode_normalization::UnicodeNormalization;

/// A `(content-regex …)` pattern compiles into at most this much program. The
/// `regex` crate matches in time linear in the haystack for every pattern, so
/// the only hostile-input cost left is compilation, which this bounds: a
/// pattern whose program would exceed it (`(a{1000}){1000}`) is refused at
/// compile time and becomes the retained false leaf §4.3.2 already defines for
/// an invalid pattern (I-22).
pub(crate) const REGEX_PROGRAM_MAX_BYTES: usize = 1 << 20;

/// Patterns that cost real work to build (`(search …)`'s friendly matcher, a
/// `(content-regex …)` regex) compiled ONCE per query rather than per block.
#[derive(Default)]
pub(crate) struct CompiledLeaves {
    matchers: HashMap<String, Matcher>,
    regexes: HashMap<String, Option<regex::Regex>>,
}

impl CompiledLeaves {
    pub(crate) fn for_query(filter: &Filter, remove_accents: bool) -> CompiledLeaves {
        let mut out = CompiledLeaves::default();
        for source in filter.match_sources() {
            out.matchers
                .entry(source.to_string())
                .or_insert_with(|| Matcher::parse_with_policy(source, remove_accents));
        }
        filter.any_leaf(&mut |leaf| {
            if let Leaf::Attr {
                attr: Attr::Content,
                op: CmpOp::Regex,
                value: Value::Text { text },
            } = leaf
            {
                out.regexes
                    .entry(text.clone())
                    .or_insert_with(|| compile_regex(text));
            }
            false
        });
        out
    }

    fn match_program(&self, source: &str) -> Option<&Matcher> {
        self.matchers.get(source)
    }

    /// `None` when the pattern did not compile (or exceeded
    /// [`REGEX_PROGRAM_MAX_BYTES`]) — a retained leaf that matches false.
    fn regex(&self, source: &str) -> Option<&regex::Regex> {
        self.regexes.get(source).and_then(Option::as_ref)
    }
}

pub(crate) fn compile_regex(pattern: &str) -> Option<regex::Regex> {
    regex::RegexBuilder::new(pattern)
        .size_limit(REGEX_PROGRAM_MAX_BYTES)
        .dfa_size_limit(REGEX_PROGRAM_MAX_BYTES)
        .build()
        .ok()
}

/// A SCHEDULED/DEADLINE projection text's day ordinal (`yyyymmdd`), accepting
/// the bracketless projection text and the `<…>` form. Calendar-validated, so a
/// malformed `<2026-13-45 …>` has presence and no day (master `date::planning_day`).
pub(crate) fn planning_day(text: &str) -> Option<i64> {
    let text = text.trim();
    let text = text.strip_prefix('<').unwrap_or(text);
    let date = &text[..text.find([' ', '>']).unwrap_or(text.len())];
    if date.contains('_') {
        return None;
    }
    JournalDate::from_file_stem(date).map(|day| day.ordinal_key())
}

/// A block's planning timestamp: lsdoc's own-line one, else a `MARKER: <…>`
/// anywhere in the block — Martin's model since f5f514878 (`TODO SCHEDULED:
/// <…> text` on one line is an agenda item too), which master's walk lost.
fn planning_text<'a>(projected: Option<&'a str>, raw: &'a str, marker: &str) -> Option<&'a str> {
    projected.or_else(|| {
        raw.match_indices(marker)
            .map(|(at, _)| raw[at + marker.len()..].trim_start())
            .find(|rest| rest.starts_with('<'))
    })
}

/// One execution's memo of the work a query repeats per block: atomizing a
/// property value (a graph repeats few distinct values across many blocks,
/// and atomizing runs the inline parser), and compiling a `LIKE` pattern
/// (folded per call site, compiled once per distinct pattern). Dies with the
/// execution; at most [`EvalCache::MAX_ENTRIES`] values are retained (the rest
/// are recomputed).
#[derive(Default)]
pub(crate) struct EvalCache {
    atoms: RefCell<AtomSlots>,
    likes: RefCell<HashMap<String, Rc<LikePattern>>>,
}

/// Indexed by `format == Org`, then source key, then value; looked up by
/// `&str` so a hit allocates nothing.
type AtomSlots = ([HashMap<String, HashMap<String, Rc<[Atom]>>>; 2], usize);

impl EvalCache {
    const MAX_ENTRIES: usize = 65_536;

    fn get(&self, key: &str, value: &str, format: AtomFormat, config: &ParseConfig) -> Rc<[Atom]> {
        let org = usize::from(format == AtomFormat::Org);
        if let Some(atoms) = self.atoms.borrow().0[org]
            .get(key)
            .and_then(|by| by.get(value))
        {
            return Rc::clone(atoms);
        }
        let atoms: Rc<[Atom]> = property_atoms(key, value, format, config).into();
        let mut cache = self.atoms.borrow_mut();
        if cache.1 < Self::MAX_ENTRIES {
            cache.1 += 1;
            cache.0[org]
                .entry(key.to_owned())
                .or_default()
                .insert(value.to_owned(), Rc::clone(&atoms));
        }
        atoms
    }

    /// `haystack LIKE pattern` (both already folded), compiling each distinct
    /// pattern once per execution: O(pattern) to fold and look up, then
    /// [`LikePattern::matches`]' linear scan.
    fn like(&self, haystack: &str, pattern: &str) -> bool {
        let hit = self.likes.borrow().get(pattern).cloned();
        let compiled = hit.unwrap_or_else(|| {
            let compiled = Rc::new(LikePattern::compile(pattern));
            let mut likes = self.likes.borrow_mut();
            if likes.len() < Self::MAX_ENTRIES {
                likes.insert(pattern.to_owned(), Rc::clone(&compiled));
            }
            compiled
        });
        compiled.matches(haystack)
    }
}

/// Per-page evaluation context: the page row a block row belongs to, plus the
/// evaluation's one `today` (relative date literals stay unresolved in the IR).
pub(crate) struct EvalCtx<'a> {
    /// The page's journal-day ordinal (`yyyymmdd`), or `None` for named pages.
    pub(crate) journal: Option<i64>,
    pub(crate) is_journal: bool,
    pub(crate) page_name: &'a str,
    /// `refs::normalize(page_name)`, hoisted out of every `refs` leaf.
    pub(crate) page_key: String,
    pub(crate) page_props: &'a [(String, String)],
    /// Every ordinary block physically owned by this page; the preamble is
    /// represented by `page_props` and is never a block-row element.
    pub(crate) page_roots: &'a [DocBlock],
    pub(crate) today: JournalDate,
    pub(crate) remove_accents: bool,
    pub(crate) compiled: &'a CompiledLeaves,
    /// The page's on-disk format: the atomizer parses a property value with the
    /// page's own inline grammar (§6.2 E4).
    pub(crate) format: AtomFormat,
    pub(crate) config: &'a ParseConfig,
    /// ONE coherent registry snapshot for the whole query (§6.2).
    pub(crate) registry: &'a Registry,
    pub(crate) cache: &'a EvalCache,
}

impl<'a> EvalCtx<'a> {
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn new(
        name: &'a str,
        kind: PageKind,
        journal: Option<i64>,
        page_props: &'a [(String, String)],
        page_roots: &'a [DocBlock],
        format: AtomFormat,
        today: JournalDate,
        remove_accents: bool,
        compiled: &'a CompiledLeaves,
        config: &'a ParseConfig,
        registry: &'a Registry,
        cache: &'a EvalCache,
    ) -> Self {
        EvalCtx {
            journal,
            is_journal: kind == PageKind::Journal,
            page_name: name,
            page_key: refs::normalize(name),
            page_props,
            page_roots,
            today,
            remove_accents,
            compiled,
            format,
            config,
            registry,
            cache,
        }
    }
}

/// Whether this query reads `:block/path-refs`, i.e. whether the walk has to
/// maintain the ancestor-ref counters at all.
pub(crate) fn uses_path_refs(filter: &Filter) -> bool {
    filter.any_leaf(&mut |leaf| matches!(leaf, Leaf::Rel { rel: Rel::Refs, .. }))
}

fn doc_children(block: &DocBlock) -> &[DocBlock] {
    &block.children
}

fn doc_refs(block: &DocBlock) -> &[String] {
    &block.projection().refs_norm
}

/// The ONE path-refs traversal over a document forest (tine-core
/// `path_refs::dfs_path_refs`), with the ancestor multiset in hand per block.
pub(crate) fn walk_path_refs<'a, V>(blocks: &'a [DocBlock], track: bool, visitor: &mut V)
where
    V: tine_core::query::path_refs::PathRefVisitor<'a, DocBlock>,
{
    let mut counts = PathRefCounts::new();
    dfs_path_refs(
        blocks,
        &doc_children,
        &doc_refs,
        &mut counts,
        track,
        visitor,
    );
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
        // `Off` is removed by `Query::evaluable_filter()` before evaluation.
        Filter::Off { .. } => true,
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
            Attr::Task => eval_optional_text(*op, value, block.marker(), ctx),
            Attr::Priority => eval_optional_text(*op, value, block.priority(), ctx),
            Attr::Scheduled => eval_planning(
                *op,
                value,
                planning_text(
                    block.projection().scheduled.as_deref(),
                    block.raw(),
                    "SCHEDULED:",
                ),
                ctx,
            ),
            Attr::Deadline => eval_planning(
                *op,
                value,
                planning_text(
                    block.projection().deadline.as_deref(),
                    block.raw(),
                    "DEADLINE:",
                ),
                ctx,
            ),
            // Page attributes only ever appear under a `page` relation, and the
            // property-element attributes only under `props`.
            _ => false,
        },
        Leaf::Rel { rel, quant, pred } => match rel {
            Rel::Refs => eval_refs(*quant, pred, block, ancestor_refs, ctx),
            Rel::Tags => quantify(*quant, block.projection().tags.iter(), |tag| {
                eval_name_element(pred, tag)
            }),
            Rel::Props => eval_props(*quant, pred, &block.projection().properties, ctx),
            Rel::Children => quantify(*quant, block.children.iter(), |child| {
                // A child is a fresh row: it keeps the anchor's ancestor context,
                // matching the direct-children-only rule (A1).
                eval_block(pred, child, ancestor_refs, ctx)
            }),
            // To-one: the page row is exactly one element.
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
        Filter::Off { .. } => true,
        Filter::Leaf { leaf } => match leaf {
            Leaf::Attr { attr, op, value } => match attr {
                Attr::Name => eval_page_name(*op, value, ctx.page_name, ctx),
                Attr::Journal => {
                    let wanted = value.as_bool();
                    match op {
                        CmpOp::Eq => wanted.is_some_and(|wanted| ctx.is_journal == wanted),
                        CmpOp::NotEq => wanted.is_some_and(|wanted| ctx.is_journal != wanted),
                        _ => false,
                    }
                }
                Attr::Day => eval_day(*op, value, ctx.journal, ctx.today),
                Attr::Namespace => {
                    // The immediate parent segment (Tine-only, M20).
                    let parent = ctx
                        .page_key
                        .rsplit_once('/')
                        .map(|(head, _)| head.to_string());
                    eval_optional_text(*op, value, parent.as_deref(), ctx)
                }
                _ => false,
            },
            Leaf::Rel { rel, quant, pred } => match rel {
                Rel::Props => eval_props(*quant, pred, ctx.page_props, ctx),
                Rel::Blocks => eval_page_blocks(*quant, pred, ctx),
                // No accepted page-row syntax reads a page's own refs or tags.
                _ => false,
            },
        },
    }
}

/// Quantify over every ordinary block physically owned by this page. Each block
/// is a fresh anchor for its `refs` closure; result-parent suppression does not
/// participate.
fn eval_page_blocks(quant: Quant, pred: &Filter, ctx: &EvalCtx) -> bool {
    let mut answer = !matches!(quant, Quant::Any);
    let mut visit = |block: &DocBlock, ancestors: &PathRefCounts| {
        let hit = eval_block(pred, block, ancestors, ctx);
        match quant {
            Quant::Any => answer |= hit,
            Quant::Every => answer &= hit,
            Quant::None => answer &= !hit,
        }
    };
    walk_path_refs(ctx.page_roots, uses_path_refs(pred), &mut visit);
    answer
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

/// `refs` is OG's `:block/path-refs`: this block's refs, every ancestor's, and
/// the page it lives on. The `name = 'x'` predicate is answered by membership.
fn eval_refs(
    quant: Quant,
    pred: &Filter,
    block: &DocBlock,
    ancestor_refs: &PathRefCounts,
    ctx: &EvalCtx,
) -> bool {
    let own = &block.projection().refs_norm;
    if let Some(name) = single_ref_name(pred) {
        let hit = closure_contains(&ctx.page_key, own, ancestor_refs, &refs::normalize(name));
        return match quant {
            Quant::Any | Quant::Every => hit,
            Quant::None => !hit,
        };
    }
    let names = closure_names(&ctx.page_key, own, ancestor_refs);
    quantify(quant, names.iter(), |name| eval_name_element(pred, name))
}

/// The `name = 'x'` shape both `[[x]]` and `#x` produce.
pub(crate) fn single_ref_name(pred: &Filter) -> Option<&str> {
    match pred {
        Filter::Leaf {
            leaf:
                Leaf::Attr {
                    attr: Attr::Name,
                    op: CmpOp::Eq,
                    value: Value::Text { text },
                },
        } => Some(text),
        _ => None,
    }
}

/// The five forms of §3.3, evaluated over the owner's property rows. The
/// quantifier ranges over the atoms of one key, never over `(key, value)` pairs.
fn eval_props(quant: Quant, pred: &Filter, properties: &[(String, String)], ctx: &EvalCtx) -> bool {
    // An owner with no property rows has no atoms under any key: only `none`
    // holds (bare presence, atom tests and atom counts are all scoped by
    // presence). Most blocks take this exit, before any allocation.
    if properties.is_empty() {
        return matches!(quant, Quant::None) && pred.props_key().is_some();
    }
    let Some(key) = pred.props_key() else {
        return false;
    };
    let key_norm = property_key_norm(&key);
    let mut rows: Vec<&str> = Vec::new();
    let mut source_key: &str = key.as_str();
    for (name, value) in properties {
        if key_norm_eq(name, &key_norm) {
            source_key = name.as_str();
            rows.push(value.as_str());
        }
    }
    let present = !rows.is_empty();
    let Some(test) = pred.props_atom_test() else {
        return match quant {
            Quant::Any | Quant::Every => present,
            Quant::None => !present,
        };
    };
    let atoms = flatten_atoms(source_key, &rows, ctx);
    if let Some(hit) = eval_atom_count_test(&test, present, atoms.len()) {
        return match quant {
            Quant::Any | Quant::Every => hit,
            Quant::None => !hit,
        };
    }
    // OG compares `tags`/`alias`/`aliases` values by lower-cased page identity;
    // the production comparison already folds case for every text atom, so
    // master's per-key case switch is the identity here.
    let effective = ctx
        .registry
        .effective_type(&key_norm)
        .unwrap_or(ObservedType::Text);
    let matches = |atom: &Atom| eval_atom_test(&test, atom, effective, ctx);
    match quant {
        Quant::Any => atoms.iter().any(matches),
        Quant::None => !atoms.iter().any(matches),
        // Present, and no atom violates; an uncoercible atom IS a violator.
        Quant::Every => present && atoms.iter().all(matches),
    }
}

/// `property_key_norm(name) == key_norm` without allocating.
fn key_norm_eq(name: &str, key_norm: &str) -> bool {
    let name = name.trim();
    name.len() == key_norm.len()
        && name.bytes().zip(key_norm.bytes()).all(|(a, b)| {
            let a = match a.to_ascii_lowercase() {
                b' ' | b'_' => b'-',
                other => other,
            };
            a == b
        })
}

/// §5.8's flattening: atomize each source row of the key in source order,
/// concatenate, de-duplicate by atom key with first occurrence winning.
fn flatten_atoms(source_key: &str, rows: &[&str], ctx: &EvalCtx) -> Vec<Atom> {
    let mut out: Vec<Atom> = Vec::new();
    let mut seen = AtomDeduper::default();
    for value in rows {
        for atom in ctx
            .cache
            .get(source_key, value, ctx.format, ctx.config)
            .iter()
        {
            if !seen.admit(&atom.key) {
                continue;
            }
            let ordinal = out.len() as u32;
            out.push(Atom {
                ordinal,
                ..atom.clone()
            });
        }
    }
    out
}

/// `Some(truth)` when the test reads only `atom_count`.
fn eval_atom_count_test(test: &Filter, present: bool, count: usize) -> Option<bool> {
    let Filter::Leaf {
        leaf:
            Leaf::Attr {
                attr: Attr::AtomCount,
                op,
                value: Value::Number { number },
            },
    } = test
    else {
        return None;
    };
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
    // Cardinality is scoped by presence: an absent key has no blank value.
    Some(present && hit)
}

fn eval_atom_test(test: &Filter, atom: &Atom, effective: ObservedType, ctx: &EvalCtx) -> bool {
    match test {
        Filter::True | Filter::Off { .. } => true,
        Filter::False | Filter::Raw { .. } => false,
        Filter::And { items } => items
            .iter()
            .all(|item| eval_atom_test(item, atom, effective, ctx)),
        Filter::Or { items } => items
            .iter()
            .any(|item| eval_atom_test(item, atom, effective, ctx)),
        Filter::Not { inner } => !eval_atom_test(inner, atom, effective, ctx),
        Filter::Leaf {
            leaf: Leaf::Attr { attr, op, value },
        } => match attr {
            Attr::Value => eval_atom_value(*op, value, atom, effective, ctx),
            // The leaf's own key-scoping conjunct, already applied by `eval_props`.
            Attr::Key => true,
            _ => false,
        },
        Filter::Leaf { .. } => false,
    }
}

/// One `atom op value` comparison, coerced by the key's effective type (§6.3).
/// An atom whose typed value is absent fails EVERY comparison, including `!=`.
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
    let typed = match effective {
        ObservedType::Number => atom.num.is_some_and(|num| compare_number(op, value, num)),
        ObservedType::Date => atom
            .day
            .is_some_and(|day| compare_day(op, value, day, ctx.today)),
        _ => return compare_atom_text(op, value, &atom.key, ctx),
    };
    // An OG `(property key value)` literal is text, and OG compares it as a
    // string: a date- or number-typed key still equals its exact text (og
    // parity; master's typed-only comparison missed `done_at:: 2026-07-19`).
    typed
        || (op == CmpOp::Eq
            && matches!(value, Value::Text { .. })
            && compare_atom_text(op, value, &atom.key, ctx))
}

fn compare_number(op: CmpOp, value: &Value, num: f64) -> bool {
    let operand = |value: &Value| match value {
        Value::Number { number } => Some(*number),
        Value::Text { text } => text.trim().parse::<f64>().ok().filter(|n| n.is_finite()),
        Value::Date { literal } => literal.trim().parse::<f64>().ok().filter(|n| n.is_finite()),
        _ => None,
    };
    let listed = |items: &[Value]| items.iter().any(|item| operand(item) == Some(num));
    match op {
        CmpOp::Between => match value {
            Value::List { items } if items.len() == 2 => {
                match (operand(&items[0]), operand(&items[1])) {
                    (Some(low), Some(high)) => {
                        let (low, high) = if low > high { (high, low) } else { (low, high) };
                        num >= low && num <= high
                    }
                    _ => false,
                }
            }
            _ => false,
        },
        CmpOp::In => value.as_list().is_some_and(listed),
        CmpOp::NotIn => value.as_list().is_some_and(|items| !listed(items)),
        CmpOp::Eq => operand(value).is_some_and(|bound| num == bound),
        CmpOp::NotEq => operand(value).is_some_and(|bound| num != bound),
        CmpOp::Lt => operand(value).is_some_and(|bound| num < bound),
        CmpOp::Le => operand(value).is_some_and(|bound| num <= bound),
        CmpOp::Gt => operand(value).is_some_and(|bound| num > bound),
        CmpOp::Ge => operand(value).is_some_and(|bound| num >= bound),
        _ => false,
    }
}

fn compare_atom_text(op: CmpOp, value: &Value, key: &str, ctx: &EvalCtx) -> bool {
    let operand = |value: &Value| match value {
        Value::Text { text } => Some(atom_key(text)),
        Value::Number { number } => Some(atom_key(&format_number(*number))),
        Value::Date { literal } => Some(atom_key(literal)),
        Value::Bool { value } => Some(value.to_string()),
        _ => None,
    };
    let listed = |items: &[Value]| {
        items
            .iter()
            .any(|item| operand(item).is_some_and(|item| item == key))
    };
    match op {
        CmpOp::In => value.as_list().is_some_and(listed),
        CmpOp::NotIn => value.as_list().is_some_and(|items| !listed(items)),
        CmpOp::Like => operand(value).is_some_and(|pattern| ctx.cache.like(key, &pattern)),
        CmpOp::StartsWith => operand(value).is_some_and(|prefix| key.starts_with(&prefix)),
        CmpOp::Eq => operand(value).is_some_and(|operand| key == operand),
        // K3: `!=` is "coercible AND unequal"; a text atom always coerces.
        CmpOp::NotEq => operand(value).is_some_and(|operand| key != operand),
        _ => false,
    }
}

fn eval_content(op: CmpOp, value: &Value, block: &DocBlock, ctx: &EvalCtx) -> bool {
    let projection = block.projection();
    let body = projection.visible_folded(ctx.remove_accents);
    let fold = |text: &str| {
        if ctx.remove_accents {
            canonical_fold(text)
        } else {
            literal_fold(text)
        }
    };
    let folded = || value.as_text().map(fold);
    // `content in (…)` is `=` against any one of the listed texts (§4.2.3).
    let listed = |items: &[Value]| {
        items
            .iter()
            .any(|item| item.as_text().is_some_and(|text| body == fold(text)))
    };
    match op {
        CmpOp::Like => folded().is_some_and(|pattern| ctx.cache.like(body, &pattern)),
        CmpOp::StartsWith => folded().is_some_and(|prefix| body.starts_with(&prefix)),
        CmpOp::Eq => folded().is_some_and(|text| body == text),
        CmpOp::NotEq => folded().is_some_and(|text| body != text),
        CmpOp::In => value.as_list().is_some_and(listed),
        CmpOp::NotIn => value.as_list().is_some_and(|items| !listed(items)),
        // §5.10: an empty or invalid Match is a FALSE leaf.
        CmpOp::Match => value.as_text().is_some_and(|text| {
            ctx.compiled
                .match_program(text)
                .is_some_and(|m| m.matches(body, &projection.visible))
        }),
        // An invalid (or over-limit) regex is retained but matches nothing.
        CmpOp::Regex => value.as_text().is_some_and(|text| {
            ctx.compiled
                .regex(text)
                .is_some_and(|r| r.is_match(&projection.visible))
        }),
        _ => false,
    }
}

/// A comparison on an OPTIONAL text attribute (`task`, `priority`, page
/// `namespace`). Absent makes every comparison false (§3.4).
fn eval_optional_text(op: CmpOp, value: &Value, actual: Option<&str>, ctx: &EvalCtx) -> bool {
    let text = value.as_text();
    let listed = |actual: &str| {
        value.as_list().map(|items| {
            items.iter().any(|item| {
                item.as_text()
                    .is_some_and(|text| actual.eq_ignore_ascii_case(text))
            })
        })
    };
    match op {
        CmpOp::IsSet => actual.is_some(),
        CmpOp::IsNotSet => actual.is_none(),
        CmpOp::Eq => actual
            .zip(text)
            .is_some_and(|(actual, text)| actual.eq_ignore_ascii_case(text)),
        CmpOp::NotEq => actual
            .zip(text)
            .is_some_and(|(actual, text)| !actual.eq_ignore_ascii_case(text)),
        CmpOp::In => actual.and_then(listed).unwrap_or(false),
        CmpOp::NotIn => actual.and_then(listed).is_some_and(|hit| !hit),
        CmpOp::Like => actual.zip(text).is_some_and(|(actual, text)| {
            ctx.cache
                .like(&actual.to_ascii_lowercase(), &text.to_ascii_lowercase())
        }),
        CmpOp::StartsWith => actual.zip(text).is_some_and(|(actual, text)| {
            actual
                .to_ascii_lowercase()
                .starts_with(&text.to_ascii_lowercase())
        }),
        _ => false,
    }
}

fn eval_page_name(op: CmpOp, value: &Value, page_name: &str, ctx: &EvalCtx) -> bool {
    let key = refs::page_key(page_name);
    let text = value.as_text();
    let listed = || {
        value.as_list().map(|items| {
            items.iter().any(|item| {
                item.as_text()
                    .is_some_and(|text| key == refs::page_key(text))
            })
        })
    };
    match op {
        CmpOp::Eq => text.is_some_and(|text| key == refs::page_key(text)),
        CmpOp::NotEq => text.is_some_and(|text| key != refs::page_key(text)),
        CmpOp::StartsWith => text.is_some_and(|text| key.starts_with(&page_prefix_key(text))),
        // A LIKE pattern keeps its boundary slashes: they are pattern text.
        CmpOp::Like => text.is_some_and(|text| {
            ctx.cache
                .like(&key, &text.to_lowercase().nfc().collect::<String>())
        }),
        CmpOp::In => listed().unwrap_or(false),
        CmpOp::NotIn => listed().is_some_and(|hit| !hit),
        _ => false,
    }
}

/// The page-identity fold applied to a PREFIX: a trailing `/` is the namespace
/// boundary and survives the fold, so `(namespace Proj)` does not match `Proj`.
fn page_prefix_key(text: &str) -> String {
    match text.strip_suffix('/') {
        Some(head) => format!("{}/", refs::page_key(head)),
        None => refs::page_key(text),
    }
}

fn eval_day(op: CmpOp, value: &Value, day: Option<i64>, today: JournalDate) -> bool {
    match op {
        CmpOp::IsSet => day.is_some(),
        CmpOp::IsNotSet => day.is_none(),
        _ => day.is_some_and(|day| compare_day(op, value, day, today)),
    }
}

/// A planning-date comparison. Presence IS a projected timestamp (G2); a
/// malformed timestamp has presence but no day (E1).
fn eval_planning(op: CmpOp, value: &Value, text: Option<&str>, ctx: &EvalCtx) -> bool {
    match op {
        CmpOp::IsSet => text.is_some(),
        CmpOp::IsNotSet => text.is_none(),
        _ => text
            .and_then(planning_day)
            .is_some_and(|day| compare_day(op, value, day, ctx.today)),
    }
}

fn compare_day(op: CmpOp, value: &Value, day: i64, today: JournalDate) -> bool {
    let resolve = |value: &Value| match value {
        Value::Date { literal } => tine_core::query::resolve_date_token(literal, today),
        Value::Number { number } => Some(*number as i64),
        _ => None,
    };
    match op {
        CmpOp::Between => match value {
            Value::List { items } if items.len() == 2 => {
                // OG's `build-between-two-arg` sorts its two resolved bounds.
                let (low, high) = match (resolve(&items[0]), resolve(&items[1])) {
                    (Some(low), Some(high)) if low > high => (Some(high), Some(low)),
                    pair => pair,
                };
                low.is_none_or(|low| day >= low) && high.is_none_or(|high| day <= high)
            }
            _ => false,
        },
        CmpOp::Ge => resolve(value).is_none_or(|bound| day >= bound),
        CmpOp::Le => resolve(value).is_none_or(|bound| day <= bound),
        CmpOp::Gt => resolve(value).is_some_and(|bound| day > bound),
        CmpOp::Lt => resolve(value).is_some_and(|bound| day < bound),
        CmpOp::Eq => resolve(value).is_some_and(|bound| day == bound),
        CmpOp::NotEq => resolve(value).is_some_and(|bound| day != bound),
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn planning_day_reads_both_forms_and_validates_the_calendar() {
        assert_eq!(planning_day("2026-07-29 Wed"), Some(20260729));
        assert_eq!(planning_day("<2026-07-29 Wed>"), Some(20260729));
        assert_eq!(planning_day("2026-07-29"), Some(20260729));
        assert_eq!(planning_day("2026-13-45"), None);
        assert_eq!(planning_day("2026-02-30"), None);
        assert_eq!(planning_day("2023-02-29"), None);
        assert_eq!(planning_day("2024-02-29"), Some(20240229));
        assert_eq!(
            planning_day("2026_07_29"),
            None,
            "planning text uses dashes only"
        );
    }

    /// I-22: a regex whose compiled program would be huge is refused at
    /// compile time (a false leaf), while a benign pattern at the extreme end of
    /// ordinary use still compiles and matches in linear time.
    #[test]
    fn regex_compilation_is_bounded_and_matching_is_linear() {
        let started = std::time::Instant::now();
        assert!(
            compile_regex("(a{1000}){1000}").is_none(),
            "hostile program refused"
        );
        assert!(compile_regex("((a+)+)+$").is_some());
        let hostile_input = "a".repeat(64 * 1024) + "!";
        assert!(!compile_regex("^((a+)+)+$")
            .unwrap()
            .is_match(&hostile_input));
        // Benign extreme: a 64-way literal alternation of long words.
        let words = (0..64)
            .map(|at| format!("word{at:03}{}", "x".repeat(200)))
            .collect::<Vec<_>>();
        let benign = compile_regex(&words.join("|")).expect("benign pattern compiles");
        assert!(benign.is_match(&format!("prefix {} suffix", words[63])));
        assert!(
            started.elapsed() < std::time::Duration::from_secs(5),
            "bounded compile + linear match"
        );
    }
}
