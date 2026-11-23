//! Authored query EDN spans: the native and wasm option-editing door (I-4/I-12).
//!
//! `read` returns each form's UTF-8 byte range and direct children, excluding
//! `#_` discarded forms from collection membership. `options` reads only direct
//! keyword entries; `edit_title` splices only the direct title value/pair. Every
//! other byte (comments, whitespace, nested forms, discards) stays authored.
//! Malformed, ambiguous duplicate-key, oversized or excessive-depth EDN refuses
//! with `None`; callers retain the original source and surface a typed refusal.
//! Cost O(source bytes), bounded to 1 MiB and a fixed depth. No I/O or graph state.
//! `inspect_begin_query` supplies the live/export BEGIN_QUERY payload decision.
//! The shared EDN reader validates forms before the query layer selects fields.

use crate::edn::{self, Edn, Mode};
use serde::Serialize;

pub(crate) use crate::edn::js_whitespace;
pub use crate::edn::{string_end, Form, Kind};

/// Exactly one form, allowing trivia and discards before and after it.
pub fn read(source: &str) -> Option<Form> {
    edn::read(source, edn::QUERY_BYTES, Mode::Edn)
}

fn map(source: &str) -> Option<Form> {
    read(source).filter(|form| form.kind == Kind::Map)
}

#[derive(Default, Serialize)]
pub struct Options {
    pub title: Option<String>,
    pub collapsed: bool,
    pub table: bool,
}
/// Empty source means no options; unreadable maps yield None.
pub fn options(source: &str) -> Option<Options> {
    if source.is_empty() {
        return Some(Options::default());
    }
    let form = map(source)?;
    let mut options = Options::default();
    for pair in form.children.chunks_exact(2) {
        let value = pair[1].leaf.as_ref();
        match pair[0].keyword() {
            Some("title") => {
                if let Some(Edn::Str(title)) = value {
                    options.title = Some(title.clone());
                }
            }
            Some("collapsed?") => options.collapsed = value == Some(&Edn::Bool(true)),
            Some("table-view?") => options.table = value == Some(&Edn::Bool(true)),
            _ => (),
        }
    }
    Some(options)
}

/// Set/remove only the direct title. Empty title removes it. No normalization
/// of the remaining map; callers decide whether a title fits a macro wrapper.
pub fn edit_title(source: &str, title: &str) -> Option<String> {
    if source.is_empty() {
        return Some(if title.is_empty() {
            String::new()
        } else {
            format!("{{:title {}}}", edn::to_string(&Edn::Str(title.into())))
        });
    }
    let form = map(source)?;
    for pair in form.children.chunks_exact(2) {
        if pair[0].keyword() == Some("title") {
            let range = pair[1].authored_span();
            if title.is_empty() {
                let key = pair[0].authored_span();
                return Some(format!(
                    "{}{}{}",
                    &source[..key.start],
                    &source[key.end..range.start],
                    &source[range.end..]
                ));
            }
            let replacement = edn::to_string(&Edn::Str(title.into()));
            return Some(format!(
                "{}{}{}",
                &source[..range.start],
                replacement,
                &source[range.end..]
            ));
        }
    }
    if title.is_empty() {
        return Some(source.into());
    }
    if source.as_bytes()[form.span.start] == b'#' {
        let at = form.span.end - 1;
        return Some(format!(
            "{} :_/title {}{}",
            &source[..at],
            edn::to_string(&Edn::Str(title.into())),
            &source[at..]
        ));
    }
    let at = form.span.start + 1;
    let separator = if at == form.span.end - 1 { "" } else { " " };
    Some(format!(
        "{}:title {}{}{}",
        &source[..at],
        edn::to_string(&Edn::Str(title.into())),
        separator,
        &source[at..]
    ))
}

/// One direct option value in an advanced map or vector-plus-options stream.
/// Duplicate declarations refuse rather than choosing one arbitrarily.
pub(crate) fn option_value<'a>(source: &'a str, key: &str) -> Option<&'a str> {
    read_option(source, key, |form| &source[form.span.clone()])
}

pub(crate) fn read_option<T>(
    source: &str,
    key: &str,
    select: impl FnOnce(&Form) -> T,
) -> Option<T> {
    let forms = edn::forms(source, Mode::Query)?;
    option_form(option_pairs(&forms), key.strip_prefix(':')?).map(select)
}

fn option_pairs(forms: &[Form]) -> &[Form] {
    match forms {
        [form] if form.kind == Kind::Map => &form.children,
        [form, options @ ..] if form.kind == Kind::Vector => options,
        _ => &[],
    }
}

fn option_form<'a>(options: &'a [Form], key: &str) -> Option<&'a Form> {
    let mut values = options
        .chunks_exact(2)
        .filter(|pair| pair[0].keyword() == Some(key));
    let first = &values.next()?[1];
    values.next().is_none().then_some(first)
}

#[derive(PartialEq)]
pub(crate) enum InputItem<'a> {
    CurrentPage,
    Other(&'a str),
}

pub(crate) struct Advanced<'a> {
    pub vector: Vec<&'a str>,
    pub inputs: Vec<InputItem<'a>>,
    pub has_rules: bool,
    pub has_result_transform: bool,
    pub result_transform: Option<Form>,
}

/// Effective vector and positional inputs from one bounded EDN read.
/// The map is Logseq syntax. The bare vector plus options is Tine syntax.
pub(crate) fn advanced(source: &str) -> Option<Advanced<'_>> {
    let mut options = edn::forms(source, Mode::Query)?;
    let head = options.first()?.kind;
    let form = options.remove(0);
    let vector = match head {
        Kind::Vector => &form,
        Kind::Map if options.is_empty() => {
            options = form.children;
            option_form(&options, "query")?
        }
        _ => return None,
    };
    if vector.kind != Kind::Vector || options.len() % 2 != 0 {
        return None;
    }
    let input_items = option_form(&options, "inputs")
        .filter(|form| matches!(form.kind, Kind::List | Kind::Vector))
        .map_or(&[][..], |form| &form.children);
    let vector = vector
        .children
        .iter()
        .map(|item| &source[item.span.clone()])
        .collect();
    let inputs = input_items
        .iter()
        .map(|item| match item.keyword() {
            Some("current-page") => InputItem::CurrentPage,
            _ => InputItem::Other(&source[item.span.clone()]),
        })
        .collect();
    let mut has_rules = false;
    let mut transform = None;
    let mut duplicate_transform = false;
    for (index, pair) in options.chunks_exact(2).enumerate() {
        match pair[0].keyword() {
            Some("rules") => has_rules = true,
            Some("result-transform") => {
                duplicate_transform |= transform.replace(index * 2 + 1).is_some()
            }
            _ => (),
        }
    }
    Some(Advanced {
        vector,
        inputs,
        has_rules,
        has_result_transform: transform.is_some(),
        result_transform: transform
            .filter(|_| !duplicate_transform)
            .map(|index| options.swap_remove(index)),
    })
}

/// Whether OG's top-level-result rule applies to an advanced query's block rows:
/// `components/query/result.cljs` `get-query-result` runs
/// `tree/filter-top-level-blocks` only `(get query-m :remove-block-children? true)`.
/// `get` returns a stored value as written and `cond->` tests its truthiness, so
/// exactly the literals `false` and `nil` turn the rule off; any other value, an
/// absent key and a duplicated key (unreadable as a map) keep it on. O(source bytes).
pub fn removes_block_children(source: &str) -> bool {
    !matches!(
        option_value(source, ":remove-block-children?"),
        Some("false" | "nil")
    )
}

/// Whether an authored advanced-query source *declares* the option `key`
/// (for example `:result-transform`): either as a direct entry of a map form
/// (`{:query [..] :result-transform (fn ..)}`) or as a top-level keyword
/// followed by its value (`[:find ..] :inputs [..] :result-transform (fn ..)`).
/// A mention inside a string, a comment, a `#_` discard or the query vector is
/// not a declaration (I-12: the EDN owner decides, not a substring). Unreadable
/// EDN declares nothing. O(source bytes).
pub fn declares_option(source: &str, key: &str) -> bool {
    let Some(forms) = edn::forms(source, Mode::QuerySyntax) else {
        return false;
    };
    let key = key.strip_prefix(':');
    option_pairs(&forms)
        .chunks(2)
        .any(|pair| key.is_some() && pair[0].keyword() == key)
}

/// An EDN query stream's final map, only when another form precedes it.
/// Logseq page references in the query form remain opaque authored atoms.
pub fn split_trailing_map(source: &str) -> (String, String) {
    let unchanged = || (source.trim().into(), String::new());
    let Some(forms) = edn::forms(source, Mode::Macro) else {
        return unchanged();
    };
    if let [vector, stream @ .., key, _] = forms.as_slice() {
        // A vector followed by option pairs can have a map as its last
        // value. Only a standalone final map is the macro's display options.
        if vector.kind == Kind::Vector && stream.len() % 2 == 0 && key.keyword().is_some() {
            return unchanged();
        }
    }
    if let Some(last) = forms.last().filter(|last| {
        forms.len() > 1 && last.kind == Kind::Map && last.span.end == source.trim_end().len()
    }) {
        let span = last.authored_span();
        return (source[..span.start].trim().into(), source[span].into());
    }
    unchanged()
}

/// BEGIN_QUERY payload inspection shared by live wasm and native export.
/// Reads only accepted direct EDN map entries; preserves query/input spelling,
/// decodes titles with the EDN owner and returns the live refusal reason.
/// O(payload bytes), with `read`'s size/depth bounds; no graph access or I/O.
#[derive(Debug, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum BeginQueryMatch {
    Supported {
        query: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        title: Option<String>,
        /// The map said `:remove-block-children? false` (or `nil`): OG then keeps every matching
        /// block, children of matching parents included ([`removes_block_children`]). The query
        /// text above cannot carry it for the simple DSL, so the caller forwards it in the macro's
        /// trailing options map.
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        keeps_block_children: bool,
    },
    Unsupported {
        reason: &'static str,
    },
}

pub fn inspect_begin_query(payload: &str) -> BeginQueryMatch {
    use BeginQueryMatch::{Supported, Unsupported};
    let source = payload.trim_matches(js_whitespace);
    let form = match edn::read_query_map(source) {
        Ok(form) => form,
        Err(key) => {
            return Unsupported {
                reason: match key {
                    Some(":query") => "duplicate :query entry",
                    Some(":inputs") => "expected :inputs to be a vector or list",
                    Some(":title") => "expected :title to be a string",
                    _ => "malformed EDN query map",
                },
            };
        }
    };
    if form.kind != Kind::Map {
        return Unsupported {
            reason: "malformed EDN query map",
        };
    }
    let mut title = None;
    let mut query = None;
    let mut inputs = None;
    let mut result_transform = None;
    let mut rules = None;
    for pair in form.children.chunks_exact(2) {
        let value = &source[pair[1].span.clone()];
        match pair[0].keyword() {
            Some("query") => {
                query = Some(value);
            }
            Some("inputs") => {
                if !matches!(pair[1].kind, Kind::Vector | Kind::List) {
                    return Unsupported {
                        reason: "expected :inputs to be a vector or list",
                    };
                }
                inputs = Some(value);
            }
            // A Clojure function Tine never runs (ADR 0042). Carry it so the
            // advanced lowerer sees it and refuses the whole query visibly,
            // instead of running the bare vector as if the transform did not exist.
            Some("result-transform") => result_transform = Some(value),
            Some("rules") => rules = Some(value),
            Some("title") => {
                if pair[1].kind != Kind::String {
                    return Unsupported {
                        reason: "expected :title to be a string",
                    };
                }
                if let Some(Edn::Str(text)) = pair[1].leaf.as_ref() {
                    title = Some(text.clone());
                }
            }
            _ => (),
        }
    }
    // Retain the live inspector's advanced-vector admission (JS word boundary).
    let keyword = |text: &str, key: &str| {
        text.match_indices(key).any(|(at, _)| {
            text.as_bytes()
                .get(at + key.len())
                .is_none_or(|c| !c.is_ascii_alphanumeric() && *c != b'_')
        })
    };
    // Docs example 17 embeds the existing simple DSL in the same EDN map.
    // Route it to that parser; inputs/rules/transforms have no simple meaning.
    if query.is_some_and(|q| q.starts_with('('))
        && (inputs.is_some() || rules.is_some() || result_transform.is_some())
    {
        return Unsupported {
            reason: "simple query maps with inputs, rules or result-transform are unsupported",
        };
    }
    let Some(query) = query.filter(|q| {
        q.starts_with('(') || (q.starts_with('[') && keyword(q, ":find") && keyword(q, ":where"))
    }) else {
        return Unsupported {
            reason: "expected an advanced :query vector",
        };
    };
    let mut query = match inputs {
        Some(inputs) => format!("{query} :inputs {inputs}"),
        None => query.into(),
    };
    if let Some(transform) = result_transform {
        query = format!("{query} :result-transform {transform}");
    }
    if let Some(rules) = rules {
        query = format!("{query} :rules {rules}");
    }
    Supported {
        query,
        title,
        keeps_block_children: !removes_block_children(source),
    }
}
