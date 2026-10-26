//! Authored query EDN spans: the native and wasm option-editing door (I-4/I-12).
//!
//! `read` returns each form's UTF-8 byte range and direct children, excluding
//! `#_` discarded forms from collection membership. `options` reads only direct
//! keyword entries; `edit_title` splices only the direct title value/pair. Every
//! other byte (comments, whitespace, nested forms, discards) stays authored.
//! Malformed, ambiguous duplicate-key, oversized or excessive-depth EDN refuses
//! with `None`; callers retain the original source and surface a typed refusal.
//! Cost O(source bytes), bounded to 1 MiB / 128 levels. No I/O or graph state.
//! `inspect_begin_query` supplies the live/export BEGIN_QUERY payload decision.
//! Scalar decoding and string writing use `edn`, the existing EDN value owner.

use crate::edn::{self, Edn};
use serde::Serialize;
use std::ops::Range;

const MAX_BYTES: usize = 1024 * 1024;
const MAX_DEPTH: usize = 128;

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Form {
    pub span: Range<usize>,
    pub kind: Kind,
    pub children: Vec<Form>,
}
#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Kind {
    Map,
    Vector,
    List,
    Set,
    String,
    Atom,
    Tagged,
}

struct Reader<'a> {
    source: &'a str,
    at: usize,
    page_refs: bool,
}
impl Reader<'_> {
    fn byte(&self) -> Option<u8> {
        self.source.as_bytes().get(self.at).copied()
    }
    fn trivia(&mut self, depth: usize) -> Option<()> {
        loop {
            match self.byte() {
                Some(b',') => self.at += 1,
                Some(c) if c.is_ascii_whitespace() => self.at += 1,
                Some(b';') => {
                    while !matches!(self.byte(), None | Some(b'\n' | b'\r')) {
                        self.at += 1;
                    }
                }
                Some(b'#') if self.source[self.at..].starts_with("#_") => {
                    self.at += 2;
                    self.form(depth + 1)?;
                }
                _ => return Some(()),
            }
        }
    }
    fn form(&mut self, depth: usize) -> Option<Form> {
        if depth >= MAX_DEPTH {
            return None;
        }
        self.trivia(depth)?;
        let start = self.at;
        let mut children = Vec::new();
        let kind = match self.byte()? {
            b'[' if self.page_refs && self.source[start..].starts_with("[[") => {
                let end = self.source[start + 2..].find("]]")?;
                self.at = start + 2 + end + 2;
                Kind::Atom
            }
            b'{' | b'[' | b'(' => {
                let (kind, close) = match self.byte()? {
                    b'{' => (Kind::Map, b'}'),
                    b'[' => (Kind::Vector, b']'),
                    _ => (Kind::List, b')'),
                };
                self.at += 1;
                loop {
                    self.trivia(depth)?;
                    if self.byte()? == close {
                        self.at += 1;
                        break;
                    }
                    children.push(self.form(depth + 1)?);
                }
                if kind == Kind::Map && children.len() % 2 != 0 {
                    return None;
                }
                kind
            }
            b'"' => {
                self.at = string_end(self.source, start)?;
                if !matches!(
                    edn::parse_strict(&self.source[start..self.at]),
                    Some(Edn::Str(_))
                ) {
                    return None;
                }
                Kind::String
            }
            b'#' if self.source[start..].starts_with("#{") => {
                self.at += 1;
                self.at += 1;
                loop {
                    self.trivia(depth)?;
                    if self.byte()? == b'}' {
                        self.at += 1;
                        break;
                    }
                    children.push(self.form(depth + 1)?);
                }
                Kind::Set
            }
            // Clojure reader macros an authored datalog query may carry: the
            // anonymous-fn literal `#( .. )` reads as a list, a regex literal
            // `#".."` as a string. Neither changes any key or value boundary.
            b'#' if self.source[start..].starts_with("#(") => {
                self.at += 1;
                return self.form(depth).map(|mut form| {
                    form.span.start = start;
                    form
                });
            }
            b'#' if self.source[start..].starts_with("#\"") => {
                self.at = string_end(self.source, start + 1)?;
                Kind::String
            }
            b'#' if !self.source[start..].starts_with("##")
                && !self.source[start..].starts_with("#'") =>
            {
                self.token()?;
                if self.at == start + 1 {
                    return None;
                }
                children.push(self.form(depth + 1)?);
                Kind::Tagged
            }
            b')' | b']' | b'}' => return None,
            _ => {
                self.token()?;
                let token = &self.source[start..self.at];
                if token == ":" {
                    return None;
                }
                Kind::Atom
            }
        };
        Some(Form {
            span: start..self.at,
            kind,
            children,
        })
    }
    fn token(&mut self) -> Option<()> {
        let start = self.at;
        if self.byte() == Some(b'\\') {
            self.at += 1;
            self.at += self.source.get(self.at..)?.chars().next()?.len_utf8();
        }
        while let Some(c) = self.byte() {
            if c.is_ascii_whitespace()
                || matches!(
                    c,
                    b',' | b';' | b'(' | b')' | b'[' | b']' | b'{' | b'}' | b'"'
                )
            {
                break;
            }
            self.at += 1;
        }
        (self.at > start).then_some(())
    }
}

/// End of one EDN string; delimiter protection shares this with macro_text.
/// Escape validity belongs to the scalar decoder, not this extent operation.
pub fn string_end(source: &str, at: usize) -> Option<usize> {
    let bytes = source.as_bytes();
    let mut i = at + 1;
    while i < bytes.len() {
        match bytes[i] {
            b'\\' => i += 2,
            b'"' => return Some(i + 1),
            _ => i += 1,
        }
    }
    None
}

/// Exactly one form, allowing trivia/discards before and after it.
pub fn read(source: &str) -> Option<Form> {
    if source.len() > MAX_BYTES {
        return None;
    }
    let mut reader = Reader {
        source,
        at: 0,
        page_refs: false,
    };
    let form = reader.form(0)?;
    reader.trivia(0)?;
    (reader.at == source.len()).then_some(form)
}

fn map(source: &str) -> Option<Form> {
    let form = read(source)?;
    if form.kind != Kind::Map {
        return None;
    }
    // Duplicate keys are ambiguous for mutation, even if EDN readers accept them.
    let mut keys = std::collections::HashSet::new();
    for pair in form.children.chunks_exact(2) {
        if !keys.insert(source[pair[0].span.clone()].to_string()) {
            return None;
        }
    }
    Some(form)
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
        let value = &source[pair[1].span.clone()];
        match &source[pair[0].span.clone()] {
            ":title" => {
                if let Some(Edn::Str(title)) = edn::parse_strict(value) {
                    options.title = Some(title);
                }
            }
            ":collapsed?" => options.collapsed = value == "true",
            ":table-view?" => options.table = value == "true",
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
        if &source[pair[0].span.clone()] == ":title" {
            if title.is_empty() {
                return Some(format!(
                    "{}{}{}",
                    &source[..pair[0].span.start],
                    &source[pair[0].span.end..pair[1].span.start],
                    &source[pair[1].span.end..]
                ));
            }
            let range = pair[1].span.clone();
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

/// Every top-level form of an authored query source, in order (the query
/// vector, then any trailing `:inputs [..]`-style option pairs). Trivia and
/// `#_` discards are skipped. Malformed, oversized or too-deep EDN yields
/// `None`. O(source bytes).
fn top_level_forms(source: &str) -> Option<Vec<Form>> {
    if source.len() > MAX_BYTES {
        return None;
    }
    let mut reader = Reader {
        source,
        at: 0,
        page_refs: false,
    };
    let mut forms = Vec::new();
    loop {
        reader.trivia(0)?;
        if reader.at == source.len() {
            return Some(forms);
        }
        forms.push(reader.form(0)?);
    }
}

/// One direct option value in an advanced map or vector-plus-options stream.
/// Duplicate declarations refuse rather than choosing one arbitrarily.
pub(crate) fn option_value<'a>(source: &'a str, key: &str) -> Option<&'a str> {
    let forms = top_level_forms(source)?;
    let mut found = None;
    for (i, form) in forms.iter().enumerate() {
        let pairs = if form.kind == Kind::Map {
            form.children.chunks_exact(2).collect::<Vec<_>>()
        } else if form.kind == Kind::Atom && &source[form.span.clone()] == key {
            vec![forms.get(i..i + 2)?]
        } else {
            Vec::new()
        };
        for pair in pairs {
            if &source[pair[0].span.clone()] == key {
                if found.is_some() {
                    return None;
                }
                found = Some(&source[pair[1].span.clone()]);
            }
        }
    }
    found
}

/// Whether an authored advanced-query source *declares* the option `key`
/// (for example `:result-transform`): either as a direct entry of a map form
/// (`{:query [..] :result-transform (fn ..)}`) or as a top-level keyword
/// followed by its value (`[:find ..] :inputs [..] :result-transform (fn ..)`).
/// A mention inside a string, a comment, a `#_` discard or the query vector is
/// not a declaration (I-12: the EDN owner decides, not a substring). Unreadable
/// EDN declares nothing. O(source bytes).
pub fn declares_option(source: &str, key: &str) -> bool {
    let Some(forms) = top_level_forms(source) else {
        return false;
    };
    forms.iter().any(|form| match form.kind {
        Kind::Atom => &source[form.span.clone()] == key,
        Kind::Map => form
            .children
            .chunks_exact(2)
            .any(|pair| &source[pair[0].span.clone()] == key),
        _ => false,
    })
}

/// An EDN query stream's final map, only when another form precedes it.
/// Logseq page references in the query form remain opaque authored atoms.
pub fn split_trailing_map(source: &str) -> (String, String) {
    let unchanged = || (source.trim().into(), String::new());
    if source.len() > MAX_BYTES {
        return unchanged();
    }
    let mut reader = Reader {
        source,
        at: 0,
        page_refs: true,
    };
    let mut forms = Vec::new();
    loop {
        if reader.trivia(0).is_none() {
            return unchanged();
        }
        if reader.at == source.len() {
            break;
        }
        // Page refs are a query DSL extension, not EDN option-map syntax.
        if source[reader.at..].starts_with("[[") {
            let start = reader.at;
            let Some(end) = source[start + 2..].find("]]") else {
                return unchanged();
            };
            reader.at = start + 2 + end + 2;
            forms.push(Form {
                span: start..reader.at,
                kind: Kind::Atom,
                children: Vec::new(),
            });
        } else {
            let Some(form) = reader.form(0) else {
                return unchanged();
            };
            forms.push(form);
        }
    }
    if let Some(last) = forms.last().filter(|last| {
        forms.len() > 1 && last.kind == Kind::Map && last.span.end == source.trim_end().len()
    }) {
        return (
            source[..last.span.start].trim().into(),
            source[last.span.clone()].into(),
        );
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
    },
    Unsupported {
        reason: &'static str,
    },
}

pub fn inspect_begin_query(payload: &str) -> BeginQueryMatch {
    use BeginQueryMatch::{Supported, Unsupported};
    let source = payload.trim();
    let Some(form) = read(source).filter(|form| form.kind == Kind::Map) else {
        return Unsupported {
            reason: "malformed EDN query map",
        };
    };
    let mut title = None;
    let mut query = None;
    let mut inputs = None;
    let mut result_transform = None;
    let mut rules = None;
    for pair in form.children.chunks_exact(2) {
        let value = &source[pair[1].span.clone()];
        match &source[pair[0].span.clone()] {
            ":query" => {
                if query.is_some() {
                    return Unsupported {
                        reason: "duplicate :query entry",
                    };
                }
                query = Some(value);
            }
            ":inputs" => {
                if inputs.is_some() || pair[1].kind != Kind::Vector {
                    return Unsupported {
                        reason: "expected :inputs to be a vector",
                    };
                }
                inputs = Some(value);
            }
            // A Clojure function Tine never runs (ADR 0042). Carry it so the
            // advanced lowerer sees it and refuses the whole query visibly,
            // instead of running the bare vector as if the transform did not exist.
            ":result-transform" => result_transform = Some(value),
            ":rules" => rules = Some(value),
            ":title" => {
                if title.is_some() || pair[1].kind != Kind::String {
                    return Unsupported {
                        reason: "expected :title to be a string",
                    };
                }
                if let Some(Edn::Str(text)) = edn::parse_strict(value) {
                    title = Some(text);
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
    Supported { query, title }
}
