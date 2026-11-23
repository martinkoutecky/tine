//! Authored query EDN spans: the native and wasm option-editing door (I-4/I-12).
//!
//! `read` returns each form's UTF-8 byte range and direct children, excluding
//! `#_` discarded forms from collection membership. `options` reads only direct
//! keyword entries; `edit_title` splices only the direct title value/pair. Every
//! other byte (comments, whitespace, nested forms, discards) stays authored.
//! A splice that would cut comments, discards or metadata is refused.
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
    edn::read(source, edn::QUERY_BYTES, Mode::SpanCodec)
}

fn map(source: &str) -> Option<Form> {
    edn::read(source, edn::QUERY_BYTES, Mode::Edn).filter(|form| form.kind == Kind::Map)
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
            format!("{{:title {}}}", edn::quote(title))
        });
    }
    let form = map(source)?;
    for pair in form.children.chunks_exact(2) {
        if pair[0].keyword() == Some("title") {
            if pair[1].protected_trivia {
                return None;
            }
            let range = pair[1].span.clone();
            if title.is_empty() {
                let key = pair[0].span.clone();
                return Some(format!(
                    "{}{}{}",
                    &source[..key.start],
                    &source[key.end..range.start],
                    &source[range.end..]
                ));
            }
            let replacement = edn::quote(title);
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
            edn::quote(title),
            &source[at..]
        ));
    }
    let at = form.span.start + 1;
    let separator = if at == form.span.end - 1 { "" } else { " " };
    Some(format!(
        "{}:title {}{}{}",
        &source[..at],
        edn::quote(title),
        separator,
        &source[at..]
    ))
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
pub(crate) enum InputItem {
    CurrentPage,
    Other(Edn),
}

pub(crate) struct Advanced {
    pub vector: Vec<Edn>,
    pub inputs: Vec<InputItem>,
    pub has_rules: bool,
    pub has_result_transform: bool,
    pub result_transform: Option<Form>,
    pub keeps_block_children: bool,
}

/// Effective vector and positional inputs from one bounded EDN read.
/// The map is Logseq syntax. The bare vector plus options is Tine syntax.
pub(crate) fn advanced(source: &str) -> Option<Advanced> {
    advanced_forms(edn::forms(source, Mode::Query)?)
}

pub(crate) fn advanced_forms(mut options: Vec<Form>) -> Option<Advanced> {
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
    let vector = vector.children.iter().map(Form::value).collect();
    let inputs = input_items
        .iter()
        .map(|item| match item.keyword() {
            Some("current-page") => InputItem::CurrentPage,
            _ => InputItem::Other(item.value()),
        })
        .collect();
    let mut has_rules = false;
    let mut transform = None;
    let mut duplicate_transform = false;
    let keeps_block_children = keeps_children(&options);
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
        keeps_block_children,
        result_transform: transform
            .filter(|_| !duplicate_transform)
            .map(|index| options.swap_remove(index)),
    })
}

fn keeps_children(options: &[Form]) -> bool {
    option_form(options, "remove-block-children?")
        .is_some_and(|form| matches!(form.leaf, Some(Edn::Bool(false) | Edn::Nil)))
}

/// Whether OG's top-level-result rule applies to an advanced query's block rows:
/// `components/query/result.cljs` `get-query-result` runs
/// `tree/filter-top-level-blocks` only `(get query-m :remove-block-children? true)`.
/// `get` returns a stored value as written and `cond->` tests its truthiness, so
/// exactly the literals `false` and `nil` turn the rule off; any other value, an
/// absent key and a duplicated key (unreadable as a map) keep it on. O(source bytes).
pub fn removes_block_children(source: &str) -> bool {
    source.is_empty()
        || edn::forms(source, Mode::Query).is_none_or(|forms| !keeps_children(option_pairs(&forms)))
}

pub(crate) fn display_keeps_children(source: &str) -> bool {
    !source.is_empty() && map(source).is_some_and(|form| keeps_children(&form.children))
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
    let Some(forms) = edn::forms(source, Mode::Macro) else {
        return (source.trim().into(), String::new());
    };
    let (body, options) = macro_spans(source, &forms);
    (body.into(), options.into())
}

fn macro_spans<'a>(source: &'a str, forms: &[Form]) -> (&'a str, &'a str) {
    let unchanged = || (source.trim(), "");
    if let [vector, stream @ .., key, _] = forms {
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
        return (source[..span.start].trim(), &source[span]);
    }
    unchanged()
}

/// Read the macro once for both its wrapper options and its declaration.
pub(crate) fn read_query_macro(source: &str) -> (&str, &str, Option<Advanced>) {
    let Some(mut forms) = edn::query_macro_forms(source) else {
        return (source.trim(), "", None);
    };
    let (body, options) = macro_spans(source, &forms);
    if !options.is_empty() {
        forms.pop();
    }
    (body, options, advanced_forms(forms))
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
    let mut execution_options = false;
    for pair in form.children.chunks_exact(2) {
        match pair[0].keyword() {
            Some("query") => {
                query = Some(&pair[1]);
            }
            Some("inputs") => {
                if !matches!(pair[1].kind, Kind::Vector | Kind::List) {
                    return Unsupported {
                        reason: "expected :inputs to be a vector or list",
                    };
                }
                execution_options = true;
            }
            // A Clojure function Tine never runs (ADR 0042). Carry it so the
            // advanced lowerer sees it and refuses the whole query visibly,
            // instead of running the bare vector as if the transform did not exist.
            Some("result-transform" | "rules") => execution_options = true,
            Some("title") => match pair[1].leaf.as_ref() {
                Some(Edn::Str(text)) => title = Some(text.clone()),
                Some(Edn::Utf16(_)) => (),
                _ => {
                    return Unsupported {
                        reason: "expected :title to be a string",
                    }
                }
            },
            _ => (),
        }
    }
    // Docs example 17 embeds the existing simple DSL in the same EDN map.
    // Route it to that parser; inputs/rules/transforms have no simple meaning.
    if query.is_some_and(|q| q.kind == Kind::List) && execution_options {
        return Unsupported {
            reason: "simple query maps with inputs, rules or result-transform are unsupported",
        };
    }
    let Some(query) = query.filter(|q| {
        q.kind == Kind::List
            || (q.kind == Kind::Vector
                && ["find", "where"]
                    .into_iter()
                    .all(|key| q.children.iter().any(|item| item.keyword() == Some(key))))
    }) else {
        return Unsupported {
            reason: "expected an advanced :query vector",
        };
    };
    let query = if query.kind == Kind::Vector {
        source
    } else {
        &source[query.span.clone()]
    };
    Supported {
        query: query.into(),
        title,
        keeps_block_children: keeps_children(&form.children),
    }
}
