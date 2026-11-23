//! The bounded EDN reader and writer shared by config, queries and PDF sidecars.
//! The reader produces effective values with authored UTF-8 spans. Unknown or
//! unsupported EDN refuses the form. Reading does no I/O or graph work.

use std::fmt::Write as _;
use std::hash::{BuildHasher, Hash, Hasher};

// A sidecar is user-controlled input opened by the native process. These limits
// are intentionally far above any realistic highlight file, but keep malformed
// or hostile EDN from consuming unbounded native memory or stack.
pub(crate) const MAX_INPUT_BYTES: usize = 64 * 1024 * 1024;
const MAX_DEPTH: usize = 128;
const MAX_NODES: usize = 1_000_000;
const MAX_COLLECTION_ITEMS: usize = 250_000;

#[derive(Debug, Clone, PartialEq)]
pub enum Edn {
    Nil,
    Bool(bool),
    Int(i64),
    Float(f64),
    Str(String),
    Keyword(String), // without the leading ':'
    /// A token this model does not interpret — a symbol (`foo/bar`), a
    /// character literal (`\a`), a symbolic value (`##NaN`) or a number form
    /// such as `1N`/`1.5M` — kept as its exact source text so a write-back
    /// neither changes its type nor its spelling.
    Symbol(String),
    Vec(Vec<Edn>),
    /// `#{...}`: kept in source order (EDN sets are unordered).
    Set(Vec<Edn>),
    List(Vec<Edn>),
    Map(Vec<(Edn, Edn)>),
    Tagged(String, Box<Edn>),
}

impl Edn {
    /// Look up a keyword key in a map.
    pub fn get(&self, key: &str) -> Option<&Edn> {
        if let Edn::Map(pairs) = self {
            for (k, v) in pairs {
                if matches!(k, Edn::Keyword(s) if s == key) {
                    return Some(v);
                }
            }
        }
        None
    }
    pub fn as_str(&self) -> Option<&str> {
        match self {
            // A bare symbol read as its text before symbols were modelled;
            // readers keep that answer.
            Edn::Str(s) | Edn::Symbol(s) => Some(s),
            Edn::Tagged(_, value) => value.as_str(),
            _ => None,
        }
    }
    pub fn as_i64(&self) -> Option<i64> {
        match self {
            Edn::Int(i) => Some(*i),
            Edn::Float(f) => Some(*f as i64),
            _ => None,
        }
    }
    pub fn as_f64(&self) -> Option<f64> {
        match self {
            Edn::Float(f) => Some(*f),
            Edn::Int(i) => Some(*i as f64),
            _ => None,
        }
    }
    pub fn as_vec(&self) -> Option<&[Edn]> {
        match self {
            Edn::Vec(v) | Edn::List(v) => Some(v),
            _ => None,
        }
    }
}

/// A value and its authored byte spans. The public serialization is the
/// existing query editor's span tree; scalar values stay inside the core.
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct Form {
    pub span: std::ops::Range<usize>,
    pub kind: Kind,
    pub children: Vec<Form>,
    #[serde(skip)]
    authored_start: usize,
    #[serde(skip)]
    pub(crate) leaf: Option<Edn>,
}

#[derive(Debug, Clone, Copy, PartialEq, serde::Serialize)]
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

impl Form {
    /// The source range of the form, including any attached metadata.
    pub(crate) fn authored_span(&self) -> std::ops::Range<usize> {
        self.authored_start..self.span.end
    }

    pub(crate) fn is_symbol(&self) -> bool {
        matches!(&self.leaf, Some(Edn::Symbol(raw))
            if self.kind == Kind::Atom
                && !starts_numeric(raw)
                && !raw.starts_with(['\\', '#', '[']))
    }

    pub(crate) fn keyword(&self) -> Option<&str> {
        match self.leaf.as_ref()? {
            Edn::Keyword(key) => Some(key),
            _ => None,
        }
    }

    fn tag(&self) -> Option<&str> {
        if self.kind == Kind::Tagged {
            self.leaf.as_ref()?.as_str()
        } else {
            None
        }
    }

    pub(crate) fn value(&self) -> Edn {
        let children = || self.children.iter().map(Form::value).collect();
        match self.kind {
            Kind::Map => Edn::Map(
                self.children
                    .chunks_exact(2)
                    .map(|pair| (pair[0].value(), pair[1].value()))
                    .collect(),
            ),
            Kind::Vector => Edn::Vec(children()),
            Kind::List => Edn::List(children()),
            Kind::Set => Edn::Set(children()),
            Kind::Tagged => Edn::Tagged(
                self.leaf.as_ref().unwrap().as_str().unwrap().to_owned(),
                Box::new(self.children[0].value()),
            ),
            Kind::String | Kind::Atom => self.leaf.clone().unwrap(),
        }
    }
}

#[derive(Clone, Copy, PartialEq)]
pub(crate) enum Mode {
    Edn,
    Query,
    // Refusal diagnostics can identify an explicitly authored option around
    // a nonsupported Clojure form without accepting it as a query value.
    QuerySyntax,
    // A simple-query macro can have Logseq [[page references]] and DSL tokens.
    // This mode locates the trailing map; the DSL parser validates the query.
    Macro,
    // Image braces admit unquoted CSS lengths and repeated top-level keys.
    ImageMetadata,
    // Existing PDF files and edits preserve CR-only comment lines and opaque
    // sidecar data. This dialect never supplies query or publication values.
    PdfSidecar,
    // Config settings recover only at a known next direct entry. Values whose
    // syntax was invalid are absent. A physically torn suffix keeps prior entries.
    Config,
}

pub(crate) const QUERY_BYTES: usize = 1024 * 1024;

pub(crate) fn js_whitespace(c: char) -> bool {
    c == '\u{feff}' || (c != '\u{0085}' && c.is_whitespace())
}

pub fn parse(input: &str) -> Option<Edn> {
    Reader::new(input, MAX_INPUT_BYTES, Mode::Edn)?
        .form(0)
        .map(|form| form.value())
}

pub fn parse_strict(input: &str) -> Option<Edn> {
    read(input, MAX_INPUT_BYTES, Mode::Edn).map(|form| form.value())
}

pub(crate) fn read(input: &str, limit: usize, mode: Mode) -> Option<Form> {
    Reader::new(input, limit, mode)?.one()
}

/// Authored spans for Logseq image metadata, including its bare CSS lengths.
/// The renderer selects sizes from the returned pairs in authored order.
pub fn read_image_metadata(input: &str) -> Option<Form> {
    read(input, QUERY_BYTES, Mode::ImageMetadata)
}

/// A BEGIN_QUERY read retains the duplicate root key for its refusal message.
pub(crate) fn read_query_map(input: &str) -> Result<Form, Option<&str>> {
    let mut reader = Reader::new(input, QUERY_BYTES, Mode::Query).ok_or(None)?;
    reader.one().ok_or(reader.duplicate_root_key)
}

pub(crate) fn forms(input: &str, mode: Mode) -> Option<Vec<Form>> {
    #[cfg(test)]
    STREAM_READS.with(|count| count.set(count.get() + 1));
    let mut reader = Reader::new(input, QUERY_BYTES, mode)?;
    let mut forms = Vec::new();
    loop {
        reader.trivia(0)?;
        if reader.i == input.len() {
            return Some(forms);
        }
        forms.push(reader.form(0)?);
    }
}

#[cfg(test)]
thread_local! {
    pub(crate) static STREAM_READS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

pub(crate) fn config_entries(input: &str) -> Option<Vec<(Form, Option<Form>)>> {
    let mut reader = Reader::new(input, MAX_INPUT_BYTES, Mode::Config)?;
    let mut entries = Vec::new();
    let mut keys = std::collections::HashSet::new();
    reader.trivia(0)?;
    if reader.byte()? != b'{' {
        return None;
    }
    reader.i += 1;
    loop {
        reader.trivia(1)?;
        if reader.invalid {
            return None;
        }
        match reader.byte() {
            Some(b'}') => {
                reader.i += 1;
                reader.trivia(0)?;
                return (!reader.invalid && reader.i == input.len()).then_some(entries);
            }
            None => return Some(entries),
            _ => (),
        }
        let key = reader.form(1)?;
        if reader.invalid || !keys.insert(identity(&key, &reader.hash)) {
            return None;
        }
        let value = match reader.form(1) {
            Some(value) => (!std::mem::take(&mut reader.invalid)).then_some(value),
            None if reader.i == input.len() => {
                entries.push((key, None));
                return Some(entries);
            }
            None => return None,
        };
        entries.push((key, value));
    }
}

struct Reader<'a> {
    s: &'a [u8],
    i: usize,
    src: &'a str,
    nodes: usize,
    mode: Mode,
    invalid: bool,
    duplicate_root_key: Option<&'a str>,
    hash: std::collections::hash_map::RandomState,
}

impl<'a> Reader<'a> {
    fn new(src: &'a str, limit: usize, mode: Mode) -> Option<Self> {
        (src.len() <= limit).then(|| Self {
            s: src.as_bytes(),
            i: 0,
            src,
            nodes: 0,
            mode,
            invalid: false,
            duplicate_root_key: None,
            hash: Default::default(),
        })
    }

    fn byte(&self) -> Option<u8> {
        self.s.get(self.i).copied()
    }

    fn one(&mut self) -> Option<Form> {
        let form = self.form(0)?;
        self.trivia(0)?;
        (self.i == self.src.len()).then_some(form)
    }

    fn invalid(&mut self) -> Option<()> {
        self.invalid = true;
        (self.mode == Mode::Config).then_some(())
    }

    fn trivia(&mut self, depth: usize) -> Option<()> {
        loop {
            if let Some(end) = comment_end(self.src, self.i, self.mode == Mode::PdfSidecar) {
                self.i = end;
            } else if self.s[self.i..].starts_with(b"#_") {
                self.i += 2;
                self.form(depth + 1)?;
            } else {
                match self.src[self.i..].chars().next() {
                    Some(c) if c == ',' || js_whitespace(c) => self.i += c.len_utf8(),
                    _ => return Some(()),
                }
            }
        }
    }

    fn form(&mut self, depth: usize) -> Option<Form> {
        // Four enclosing forms are outside the advanced query's expression limit.
        let depth_limit = if matches!(self.mode, Mode::Query | Mode::QuerySyntax) {
            MAX_DEPTH + 4
        } else {
            MAX_DEPTH
        };
        if depth >= depth_limit || self.nodes >= MAX_NODES {
            return None;
        }
        self.trivia(depth)?;
        let start = self.i;
        self.nodes += 1;
        let mut children = Vec::new();
        let mut leaf = None;
        let kind = match self.byte()? {
            b'^' | b'#' if self.s[start] == b'^' || self.s[start..].starts_with(b"#^") => {
                if self.mode == Mode::PdfSidecar {
                    return None;
                }
                self.i += if self.s[start] == b'^' { 1 } else { 2 };
                let metadata = self.form(depth + 1)?;
                let mut target = self.form(depth + 1)?;
                if !(metadata.kind == Kind::Map
                    || matches!(metadata.leaf, Some(Edn::Keyword(_) | Edn::Str(_)))
                    || metadata.is_symbol())
                    || !(matches!(
                        target.kind,
                        Kind::List | Kind::Vector | Kind::Map | Kind::Set
                    ) || target.is_symbol()
                        || target.tag() == Some("queue"))
                {
                    self.invalid()?;
                }
                target.authored_start = start;
                return Some(target);
            }
            b'[' if self.mode == Mode::Macro && self.s[start..].starts_with(b"[[") => {
                let end = self.src[start + 2..].find("]]")?;
                self.i = start + end + 4;
                leaf = Some(Edn::Symbol(self.src[start..self.i].into()));
                Kind::Atom
            }
            b'{' | b'[' | b'(' => {
                let (kind, close) = match self.byte()? {
                    b'{' => (Kind::Map, b'}'),
                    b'[' => (Kind::Vector, b']'),
                    _ => (Kind::List, b')'),
                };
                self.i += 1;
                children = self.collection(depth, close, kind, None)?;
                kind
            }
            b'"' => {
                leaf = self.string();
                if leaf.is_none() {
                    let end = string_end(self.src, start);
                    self.i = end.unwrap_or(self.s.len());
                    end?;
                    self.invalid()?;
                }
                Kind::String
            }
            b'\\' => {
                self.i += 1;
                let ch = self.src[self.i..].chars().next()?;
                if macro_terminator(ch, false) || invalid_constituent(ch) {
                    self.i += ch.len_utf8();
                } else {
                    self.token(false);
                }
                let raw = &self.src[start..self.i];
                if character(&raw[1..]).is_none() {
                    self.invalid()?;
                }
                leaf = Some(Edn::Symbol(raw.into()));
                Kind::Atom
            }
            b'#' if self.s[start..].starts_with(b"#{") => {
                self.i += 2;
                children = self.collection(depth, b'}', Kind::Set, None)?;
                Kind::Set
            }
            b'#' if self.s[start..].starts_with(b"#:") => {
                self.i += 2;
                let namespace_start = self.i;
                self.token(false);
                let namespace = &self.src[namespace_start..self.i];
                if !valid_name(namespace) || (namespace.contains('/') && namespace != "/") {
                    return None;
                }
                while let Some(c) = self.src[self.i..].chars().next() {
                    if c != ',' && !js_whitespace(c) {
                        break;
                    }
                    self.i += c.len_utf8();
                }
                if self.byte()? != b'{' {
                    return None;
                }
                self.i += 1;
                children = self.collection(depth, b'}', Kind::Map, Some(namespace))?;
                Kind::Map
            }
            b'#' if matches!(self.mode, Mode::QuerySyntax | Mode::Macro)
                && matches!(self.s.get(start + 1), Some(b'(' | b'\'' | b'"')) =>
            {
                self.i += 1;
                match self.byte()? {
                    b'(' => {
                        self.form(depth + 1)?;
                    }
                    b'\'' => {
                        self.i += 1;
                        if !self.form(depth + 1)?.is_symbol() {
                            return None;
                        }
                    }
                    _ => self.i = string_end(self.src, self.i)?,
                }
                leaf = Some(Edn::Symbol(self.src[start..self.i].into()));
                Kind::Atom
            }
            b'#' if self.s[start..].starts_with(b"##") => {
                self.i += 2;
                let symbol = self.form(depth + 1)?;
                let raw = match symbol.leaf.as_ref().and_then(Edn::as_str) {
                    Some("NaN") if symbol.is_symbol() => "##NaN",
                    Some("Inf") if symbol.is_symbol() => "##Inf",
                    Some("-Inf") if symbol.is_symbol() => "##-Inf",
                    _ => return None,
                };
                leaf = Some(Edn::Symbol(raw.into()));
                Kind::Atom
            }
            b'#' if self.s[start..].starts_with(b"#<") => return None,
            b'#' => {
                self.i += 1;
                let tag = self.form(depth + 1)?;
                if !tag.is_symbol() {
                    return None;
                }
                let name = tag.leaf?.as_str()?.to_owned();
                let value = self.form(depth + 1)?;
                let supported = match name.as_str() {
                    "uuid" => value.kind == Kind::String,
                    "inst" => matches!(&value.leaf, Some(Edn::Str(text)) if instant(text).is_some()),
                    "queue" => value.kind == Kind::Vector,
                    "js" => match value.kind {
                        Kind::Vector => true,
                        Kind::Map => value.children.iter().step_by(2).all(|key| {
                            matches!(&key.leaf, Some(Edn::Str(_) | Edn::Keyword(_)))
                                || key.is_symbol()
                                || matches!(&key.leaf, Some(Edn::Symbol(raw)) if raw.starts_with('\\'))
                        }),
                        _ => false,
                    },
                    _ => false,
                };
                if !supported {
                    self.invalid()?;
                }
                children.push(value);
                leaf = Some(Edn::Symbol(name));
                Kind::Tagged
            }
            b')' | b']' | b'}' => return None,
            _ => {
                let keyword = self.byte() == Some(b':');
                self.i += usize::from(keyword);
                let token_start = self.i;
                let numeric = !keyword && starts_numeric(&self.src[start..]);
                self.token(numeric);
                if self.i == start {
                    return None;
                }
                let token = &self.src[token_start..self.i];
                leaf = match token {
                    _ if keyword
                        && !token.starts_with(':')
                        && valid_name(token)
                        && !token.contains("::") =>
                    {
                        Some(Edn::Keyword(token.into()))
                    }
                    _ if keyword => None,
                    "nil" => Some(Edn::Nil),
                    "true" => Some(Edn::Bool(true)),
                    "false" => Some(Edn::Bool(false)),
                    _ if numeric => number(token)
                        .map(|value| {
                            // Keep the established write-back spelling and
                            // typed settings behavior for suffix literals.
                            if token.ends_with(['N', 'M']) {
                                Edn::Symbol(token.into())
                            } else {
                                value
                            }
                        })
                        .or_else(|| {
                            (self.mode == Mode::ImageMetadata
                                || (matches!(
                                    self.mode,
                                    Mode::Query | Mode::QuerySyntax | Mode::Macro
                                ) && crate::query::date_token::DateToken::parse(token)
                                    .is_some()))
                            .then(|| Edn::Symbol(token.into()))
                        }),
                    _ if valid_name(token) || self.mode == Mode::Macro => {
                        Some(Edn::Symbol(token.into()))
                    }
                    _ => None,
                };
                if leaf.is_none() {
                    self.invalid()?;
                }
                Kind::Atom
            }
        };
        Some(Form {
            span: start..self.i,
            kind,
            children,
            authored_start: start,
            leaf,
        })
    }

    fn token(&mut self, numeric: bool) {
        while let Some(c) = self.src[self.i..].chars().next() {
            if macro_terminator(c, numeric) {
                break;
            }
            self.i += c.len_utf8();
        }
    }

    fn collection(
        &mut self,
        depth: usize,
        close: u8,
        kind: Kind,
        namespace: Option<&str>,
    ) -> Option<Vec<Form>> {
        let mut children = Vec::new();
        loop {
            self.trivia(depth + 1)?;
            if self.byte()? == close {
                self.i += 1;
                break;
            }
            if children.len() >= MAX_COLLECTION_ITEMS {
                return None;
            }
            children.push(self.form(depth + 1)?);
        }
        let stride = match kind {
            Kind::Map => {
                if children.len() % 2 != 0 {
                    self.invalid()?;
                }
                if let Some(namespace) = namespace {
                    for key in children.iter_mut().step_by(2) {
                        let symbol = key.is_symbol();
                        if let Some(Edn::Keyword(name)) = &mut key.leaf {
                            qualify(name, namespace);
                        } else if symbol {
                            if let Some(Edn::Symbol(name)) = &mut key.leaf {
                                qualify(name, namespace);
                            }
                        }
                    }
                } else if self.mode == Mode::ImageMetadata && depth == 0 {
                    return Some(children);
                }
                2
            }
            Kind::Set => 1,
            _ => return Some(children),
        };
        let mut seen = std::collections::HashSet::new();
        if let Some(duplicate) = children
            .iter()
            .step_by(stride)
            .find(|child| !seen.insert(identity(child, &self.hash)))
        {
            if kind == Kind::Map && depth == 0 {
                self.duplicate_root_key = self.src.get(duplicate.span.clone());
            }
            self.invalid()?;
        }
        Some(children)
    }

    /// Decode a string with the escapes Clojure's `EdnReader.readString`
    /// accepts (`\t \r \n \\ \" \b \f`, `\uXXXX`, octal `\NNN`).
    /// Any other escape is not EDN and fails the parse: a byte-wise guess
    /// used to slice mid-character and panic on `"\é"`.
    fn string(&mut self) -> Option<Edn> {
        self.i += 1; // opening quote
        let mut out = String::new();
        while self.i < self.s.len() {
            let c = self.s[self.i];
            if c == b'\\' {
                self.i += 1;
                let n = *self.s.get(self.i)?;
                self.i += 1;
                out.push(match n {
                    b'n' => '\n',
                    b't' => '\t',
                    b'r' => '\r',
                    b'b' => '\u{8}',
                    b'f' => '\u{c}',
                    b'"' => '"',
                    b'\\' => '\\',
                    b'u' => {
                        let unit = self.hex4()?;
                        match unit {
                            0xD800..=0xDBFF => {
                                // A high surrogate must pair with `\uDC00`–`\uDFFF`.
                                if self.s.get(self.i..self.i + 2) != Some(b"\\u") {
                                    return None;
                                }
                                self.i += 2;
                                let low = self.hex4()?;
                                if !(0xDC00..=0xDFFF).contains(&low) {
                                    return None;
                                }
                                char::from_u32(0x10000 + ((unit - 0xD800) << 10) + (low - 0xDC00))?
                            }
                            _ => char::from_u32(unit)?,
                        }
                    }
                    b'0'..=b'7' => {
                        let mut value = u32::from(n - b'0');
                        for _ in 0..2 {
                            let c = self.src.get(self.i..)?.chars().next()?;
                            if macro_terminator(c, true) {
                                break;
                            }
                            value = value * 8 + c.to_digit(8)?;
                            self.i += 1;
                        }
                        if value > 0o377 {
                            return None;
                        }
                        char::from_u32(value)?
                    }
                    _ => return None,
                });
            } else if c == b'"' {
                self.i += 1;
                return Some(Edn::Str(out));
            } else {
                // copy a full UTF-8 char; `self.i` is always on a char boundary
                let ch = self.src.get(self.i..)?.chars().next()?;
                out.push(ch);
                self.i += ch.len_utf8();
            }
        }
        None
    }

    fn hex4(&mut self) -> Option<u32> {
        let digits = self.s.get(self.i..self.i + 4)?;
        let text = std::str::from_utf8(digits).ok()?;
        if !digits.iter().all(u8::is_ascii_hexdigit) {
            return None;
        }
        self.i += 4;
        u32::from_str_radix(text, 16).ok()
    }
}

fn valid_name(token: &str) -> bool {
    if token.is_empty()
        || token.ends_with(':')
        || token.starts_with("::")
        || token.chars().any(invalid_constituent)
    {
        return false;
    }
    if token == "/" {
        return true;
    }
    token.split_once('/').is_none_or(|(namespace, name)| {
        !namespace.is_empty()
            && !namespace.ends_with(':')
            && !name.is_empty()
            && !name.as_bytes()[0].is_ascii_digit()
            && (name == "/" || !name.contains('/'))
    })
}

fn invalid_constituent(c: char) -> bool {
    matches!(c, '@' | '~' | '\x60')
}

fn qualify(name: &mut String, namespace: &str) {
    match name.split_once('/') {
        Some(("_", local)) => *name = local.into(),
        Some((prefix, _)) if !prefix.is_empty() => (),
        _ => *name = format!("{namespace}/{name}"),
    }
}

fn macro_terminator(c: char, numeric: bool) -> bool {
    js_whitespace(c)
        || matches!(
            c,
            ',' | ';' | '(' | ')' | '[' | ']' | '{' | '}' | '"' | '^' | '\\'
        )
        || (numeric && matches!(c, ':' | '#'))
}

fn character(token: &str) -> Option<char> {
    Some(match token {
        "newline" => '\n',
        "return" => '\r',
        "space" => ' ',
        "tab" => '\t',
        "backspace" => '\u{8}',
        "formfeed" => '\u{c}',
        _ if token.encode_utf16().count() == 1 => token.chars().next()?,
        _ if token.starts_with('u')
            && token.len() == 5
            && token.as_bytes()[1..].iter().all(u8::is_ascii_hexdigit) =>
        {
            char::from_u32(u32::from_str_radix(&token[1..], 16).ok()?)?
        }
        _ if token.starts_with('o')
            && token.len() <= 4
            && token.as_bytes()[1..]
                .iter()
                .all(|digit| matches!(digit, b'0'..=b'7')) =>
        {
            char::from(u8::from_str_radix(&token[1..], 8).ok()?)
        }
        _ => return None,
    })
}

// tools.reader tries int-pattern first, then float-pattern, then ratio-pattern.
// ClojureScript decodes all three into JS Numbers, including N/M suffixes.
fn number(token: &str) -> Option<Edn> {
    let digits = |s: &str| !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit());
    let signed = token.strip_suffix('N').unwrap_or(token);
    let plain = signed.strip_prefix(['+', '-']).unwrap_or(signed);
    let sign = if signed.starts_with('-') { -1.0 } else { 1.0 };
    let integer = if digits(plain) {
        if plain.len() > 1 && plain.starts_with('0') {
            radix_number(plain, 8, true)
        } else {
            plain.parse().ok()
        }
    } else if let Some(hex) = plain
        .strip_prefix("0x")
        .or_else(|| plain.strip_prefix("0X"))
    {
        radix_number(hex, 16, true)
    } else if let Some((base, value)) = token
        .strip_prefix(['+', '-'])
        .unwrap_or(token)
        .split_once(['r', 'R'])
    {
        if base.len() > 2
            || base.starts_with('0')
            || !digits(base)
            || value.is_empty()
            || !value.bytes().all(|b| b.is_ascii_alphanumeric())
        {
            return None;
        }
        let base = base.parse::<u32>().ok()?;
        if !(2..=36).contains(&base) {
            return None;
        }
        let value = if base == 16 {
            value
                .strip_prefix("0x")
                .or_else(|| value.strip_prefix("0X"))
                .unwrap_or(value)
        } else {
            value
        };
        radix_number(value, base, false)
    } else {
        if signed.len() != token.len() {
            return None;
        }
        if let Some((numerator, denominator)) = plain.split_once('/') {
            return (digits(numerator) && digits(denominator)).then(|| {
                Edn::Float(
                    sign * numerator.parse::<f64>().unwrap() / denominator.parse::<f64>().unwrap(),
                )
            });
        }
        let decimal = signed.strip_suffix('M').unwrap_or(signed);
        return decimal.parse::<f64>().ok().map(Edn::Float);
    }? * sign;
    if integer.abs() < 9_007_199_254_740_992.0 {
        Some(Edn::Int(integer as i64))
    } else {
        Some(Edn::Float(integer))
    }
}

fn radix_number(source: &str, base: u32, exact: bool) -> Option<f64> {
    let end = source
        .bytes()
        .take_while(|b| char::from(*b).is_digit(base))
        .count();
    if end == 0 || (exact && end != source.len()) {
        return None;
    }
    let digits = &source[..end];
    if base == 10 {
        return digits.parse().ok();
    }
    let value = |byte| char::from(byte).to_digit(base).unwrap();
    if base.is_power_of_two() {
        // Retain enough leading bits for f64 to round once. A nonzero
        // discarded tail breaks an exact tie in the low bits of the prefix.
        let mut prefix = 0u64;
        for (index, byte) in digits.bytes().enumerate() {
            let digit = u64::from(value(byte));
            if let Some(next) = prefix.checked_mul(u64::from(base)) {
                prefix = next + digit;
            } else {
                let tail = &digits[index..];
                let sticky = u64::from(tail.bytes().any(|byte| byte != b'0'));
                let shift = tail.len() as i32 * base.trailing_zeros() as i32;
                return Some((prefix | sticky) as f64 * 2f64.powi(shift));
            }
        }
        return Some(prefix as f64);
    }
    // V8's parseInt approximates other radices using exact u32 chunks.
    let (mut total, mut part, mut scale) = (0.0, 0, 1);
    for byte in digits.bytes() {
        if scale * base > u32::MAX / 36 {
            total = total * f64::from(scale) + f64::from(part);
            (part, scale) = (0, 1);
        }
        part = part * base + value(byte);
        scale *= base;
    }
    Some(total * f64::from(scale) + f64::from(part))
}

fn instant(text: &str) -> Option<i64> {
    use chrono::Datelike as _;
    let decimal = |digits: &[u8]| {
        digits.iter().try_fold(0u32, |n, &byte| {
            byte.is_ascii_digit()
                .then(|| n * 10 + u32::from(byte - b'0'))
        })
    };
    let text = text.as_bytes();
    let (mut stamp, offset) = match text.split_last_chunk() {
        Some((stamp, [sign, h1, h2, b':', m1, m2])) if matches!(sign, b'+' | b'-') => {
            let minutes = decimal(&[*h1, *h2])? * 60 + decimal(&[*m1, *m2])?;
            (
                stamp,
                i64::from(minutes) * if *sign == b'-' { -1 } else { 1 },
            )
        }
        _ => (text.strip_suffix(b"Z").unwrap_or(text), 0),
    };
    let year = decimal(stamp.get(..4)?)? as i32;
    stamp = &stamp[4..];
    let mut fields = [1, 1, 0, 0, 0];
    let mut precision = 0;
    for (separator, field) in [b'-', b'-', b'T', b':', b':'].into_iter().zip(&mut fields) {
        let Some(rest) = stamp.strip_prefix(&[separator]) else {
            break;
        };
        *field = decimal(rest.get(..2)?)?;
        stamp = &rest[2..];
        precision += 1;
    }
    let [month, day, hour, minute, second] = fields;
    let date = chrono::NaiveDate::from_ymd_opt(year, month, day)?;
    if hour > 23 || minute > 59 || second > if minute == 59 { 60 } else { 59 } {
        return None;
    }
    let millis = if stamp.is_empty() {
        0
    } else {
        let fraction = stamp.strip_prefix(b".")?;
        if precision != fields.len()
            || fraction.is_empty()
            || !fraction.iter().all(u8::is_ascii_digit)
        {
            return None;
        }
        fraction
            .iter()
            .zip([100, 10, 1])
            .map(|(&digit, place)| u32::from(digit - b'0') * place)
            .sum()
    };
    // Date.UTC interprets years 0–99 as 1900–1999 after the reader validates
    // the authored date. It also rolls a leap second into the next minute.
    let days = if year < 100 {
        chrono::NaiveDate::from_ymd_opt(year + 1900, month, 1)?.num_days_from_ce() + day as i32 - 1
    } else {
        date.num_days_from_ce()
    };
    let minutes = (i64::from(days) * 24 + i64::from(hour)) * 60 + i64::from(minute) - offset;
    Some(minutes * 60_000 + i64::from(second * 1000 + millis))
}

// Equality for duplicate checks follows CLJS: lists and vectors compare as
// sequences, map and set order is immaterial, and reader spelling is ignored.
// A fingerprint collision refuses the form, so it cannot admit a duplicate.
fn identity(form: &Form, hash: &impl BuildHasher) -> u64 {
    let mut state = hash.build_hasher();
    match (form.kind, form.leaf.as_ref()) {
        (Kind::Map, _) => {
            "map".hash(&mut state);
            let sum = form.children.chunks_exact(2).fold(0u64, |sum, pair| {
                sum.wrapping_add(
                    hash.hash_one((identity(&pair[0], hash), identity(&pair[1], hash))),
                )
            });
            sum.hash(&mut state);
        }
        (Kind::Set, _) => {
            "set".hash(&mut state);
            let sum = form
                .children
                .iter()
                .fold(0u64, |sum, child| sum.wrapping_add(identity(child, hash)));
            sum.hash(&mut state);
        }
        (Kind::Vector | Kind::List, _) => return sequence_identity(&form.children, hash),
        (Kind::Tagged, _) if form.tag() == Some("queue") => {
            return sequence_identity(&form.children[0].children, hash);
        }
        (Kind::Tagged, _) if form.tag() == Some("uuid") => {
            "uuid".hash(&mut state);
            form.children[0]
                .leaf
                .as_ref()
                .and_then(Edn::as_str)
                .map(str::to_lowercase)
                .hash(&mut state);
        }
        (Kind::Tagged, _) if form.tag() == Some("inst") => {
            (
                "inst",
                form.children[0]
                    .leaf
                    .as_ref()
                    .and_then(Edn::as_str)
                    .and_then(instant),
            )
                .hash(&mut state);
        }
        (Kind::Tagged, _) if form.tag() == Some("js") => {
            // Each #js form constructs a distinct JavaScript object.
            ("js", form.span.start).hash(&mut state);
        }
        (Kind::Tagged, _) => {
            form.tag().hash(&mut state);
            identity(&form.children[0], hash).hash(&mut state);
        }
        (_, Some(Edn::Int(n))) => return numeric_identity(*n as f64, hash),
        (_, Some(Edn::Float(n))) if n.is_nan() => ("NaN", form.span.start).hash(&mut state),
        (_, Some(Edn::Float(n))) => return numeric_identity(*n, hash),
        (_, Some(Edn::Symbol(raw))) if raw.starts_with('\\') => {
            "string".hash(&mut state);
            let mut buf = [0; 4];
            if let Some(c) = character(&raw[1..]) {
                c.encode_utf8(&mut buf).hash(&mut state);
            }
        }
        (_, Some(Edn::Symbol(raw))) if starts_numeric(raw) && raw.ends_with(['M', 'N']) => {
            match number(raw) {
                Some(Edn::Int(n)) => return numeric_identity(n as f64, hash),
                Some(Edn::Float(n)) => return numeric_identity(n, hash),
                _ => ("symbol", raw).hash(&mut state),
            }
        }
        (_, Some(Edn::Symbol(raw))) if raw == "##Inf" => {
            return numeric_identity(f64::INFINITY, hash)
        }
        (_, Some(Edn::Symbol(raw))) if raw == "##-Inf" => {
            return numeric_identity(f64::NEG_INFINITY, hash)
        }
        (_, Some(Edn::Symbol(raw))) if raw == "##NaN" => ("NaN", form.span.start).hash(&mut state),
        (_, Some(Edn::Symbol(raw))) => ("symbol", raw).hash(&mut state),
        (_, Some(Edn::Keyword(raw))) => ("keyword", raw).hash(&mut state),
        (_, Some(Edn::Str(text))) => ("string", text).hash(&mut state),
        (_, Some(Edn::Bool(value))) => ("bool", value).hash(&mut state),
        _ => "nil".hash(&mut state),
    }
    state.finish()
}

fn numeric_identity(n: f64, hash: &impl BuildHasher) -> u64 {
    hash.hash_one(("number", if n == 0.0 { 0 } else { n.to_bits() }))
}

fn sequence_identity(items: &[Form], hash: &impl BuildHasher) -> u64 {
    let mut state = hash.build_hasher();
    "sequence".hash(&mut state);
    for item in items {
        identity(item, hash).hash(&mut state);
    }
    state.finish()
}

/// End of a line comment in the EDN string reader. CR terminates a comment
/// only for the legacy PDF sidecar format.
pub(crate) fn comment_end(source: &str, at: usize, pdf_sidecar: bool) -> Option<usize> {
    let tail = source.get(at..)?;
    (tail.starts_with(';') || tail.starts_with("#!")).then(|| {
        tail.find(|c| c == '\n' || (pdf_sidecar && c == '\r'))
            .map_or(source.len(), |end| at + end)
    })
}

/// End of an authored string for consumers that only protect delimiters.
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

fn starts_numeric(tok: &str) -> bool {
    let digits = tok.strip_prefix(['+', '-']).unwrap_or(tok);
    digits.as_bytes().first().is_some_and(u8::is_ascii_digit)
}

pub(crate) fn is_delim(c: u8) -> bool {
    c.is_ascii_whitespace()
        || matches!(
            c,
            b'{' | b'}' | b'[' | b']' | b'(' | b')' | b'"' | b',' | b';'
        )
}

/// Pretty-ish EDN writer. Logseq reads regardless of formatting; we emit a
/// compact, stable form.
pub fn to_string(e: &Edn) -> String {
    let mut out = String::new();
    write_edn(e, &mut out);
    out
}

fn write_seq(open: &str, items: &[Edn], close: char, out: &mut String) {
    out.push_str(open);
    for (i, it) in items.iter().enumerate() {
        if i > 0 {
            out.push(' ');
        }
        write_edn(it, out);
    }
    out.push(close);
}

fn write_edn(e: &Edn, out: &mut String) {
    match e {
        Edn::Nil => out.push_str("nil"),
        Edn::Bool(b) => out.push_str(if *b { "true" } else { "false" }),
        Edn::Int(i) => {
            let _ = write!(out, "{i}");
        }
        Edn::Float(f) if f.is_nan() => out.push_str("##NaN"),
        Edn::Float(f) if *f == f64::INFINITY => out.push_str("##Inf"),
        Edn::Float(f) if *f == f64::NEG_INFINITY => out.push_str("##-Inf"),
        Edn::Float(f) => {
            let _ = write!(out, "{f}");
        }
        Edn::Str(s) => {
            out.push('"');
            for ch in s.chars() {
                match ch {
                    '"' => out.push_str("\\\""),
                    '\\' => out.push_str("\\\\"),
                    '\n' => out.push_str("\\n"),
                    '\t' => out.push_str("\\t"),
                    '\r' => out.push_str("\\r"),
                    '\u{8}' => out.push_str("\\b"),
                    '\u{c}' => out.push_str("\\f"),
                    _ => out.push(ch),
                }
            }
            out.push('"');
        }
        Edn::Keyword(k) => {
            out.push(':');
            out.push_str(k);
        }
        Edn::Symbol(raw) => out.push_str(raw),
        Edn::Set(items) => write_seq("#{", items, '}', out),
        Edn::Vec(items) => write_seq("[", items, ']', out),
        Edn::List(items) => write_seq("(", items, ')', out),
        Edn::Map(pairs) => {
            out.push('{');
            for (i, (k, v)) in pairs.iter().enumerate() {
                if i > 0 {
                    out.push(' ');
                }
                write_edn(k, out);
                out.push(' ');
                write_edn(v, out);
            }
            out.push('}');
        }
        Edn::Tagged(tag, value) => {
            out.push('#');
            out.push_str(tag);
            out.push(' ');
            write_edn(value, out);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn duplicate_detection_uses_values_and_ignores_authored_spelling() {
        for source in [
            r#"#{:k :k}"#,
            r#"{[1 2] :a (1 2) :b}"#,
            r#"#{^:private [1] (1.0)}"#,
            r#"#{#queue [1] [1.0]}"#,
            r#"#{#{1 2} #{2 1}}"#,
            r#"{{:x 1 :y 2} :a {:y 2 :x 1} :b}"#,
            r#"#{1N 1.0M}"#,
            r#"#{0x12 18}"#,
            r#"#{0x10000000000000801 18446744073709555712}"#,
            r#"#{022 18.0M}"#,
            r#"{2r10 0 4/2 1}"#,
            r#"#{\o101 \u0041}"#,
            r#"#{#inst "2020-02-29T00:30+01:00" #inst "2020-02-28T23:30Z"}"#,
            r#"#{#uuid "ABC" #uuid "abc"}"#,
            r#"#:ns{:a 1 :ns/a 2}"#,
            r#"#{#:ns{:_/a 1 b 2} {:a 1 ns/b 2}}"#,
            r#"#{\a "\u0061"}"#,
            r#"#{#_ {:x 1 :x 2} :public}"#,
        ] {
            assert_eq!(parse_strict(source), None, "{source}");
        }
        assert_eq!(parse_strict("#{1 #_ 1 2}"), parse_strict("#{1 2}"));
        let distinct_js = parse_strict("#{#js [1] #js [1] [1]}");
        assert!(matches!(distinct_js, Some(Edn::Set(items)) if items.len() == 3));
        assert!(parse_strict("#{0x10000000000000801 18446744073709551616}").is_some());
    }

    #[test]
    fn reader_validates_restored_literals_and_decodes_their_values() {
        for source in [
            r"\u+000",
            r"\u+041",
            r"\u+FFF",
            r"\uD800",
            r"\u00000",
            r"\o8",
            r"\o400",
            r"\o0000",
            r"\o+01",
            "08",
            "0x+",
            "2r2",
            "2r",
            "37r1",
            "1.1N",
            "1/-2",
            r#"#inst "2025-02-29""#,
            r#"#inst "2024-12-31T23:58:60Z""#,
            r#"#inst "2024-01T12Z""#,
            r#"#inst "2024-01-02T12:34.5Z""#,
            r#"#inst "2024-01-02T12:34:56.""#,
            "#inst :value",
            "#js (:a)",
            "#js {1 :a}",
            "#uuid :bad",
            "#:ns #{:a}",
            "#:ns/name{:a 1}",
            "#:ns{:a}",
        ] {
            assert_eq!(parse_strict(source), None, "{source}");
        }
        for (source, expected) in [
            ("0x12N", 18.0),
            ("-0X12", -18.0),
            ("+022", 18.0),
            ("00N", 0.0),
            ("2R10z", 2.0),
            ("16r0x12", 18.0),
            ("36rN", 23.0),
            ("+1/2", 0.5),
            ("01M", 1.0),
            ("08.5", 8.5),
            ("01e2", 100.0),
            ("2.e-1", 0.2),
            ("1/0", f64::INFINITY),
        ] {
            assert_eq!(
                number(source).and_then(|value| value.as_f64()),
                Some(expected),
                "{source}"
            );
        }
        assert_eq!(
            parse_strict("#:ns {:a 1 b 2 :_/c 3 :other/d 4}"),
            parse_strict("{:ns/a 1 ns/b 2 :c 3 :other/d 4}")
        );
        for (left, right) in [
            ("2024Z", "2024-01-01T00:00:00.000+00:00"),
            ("2024-01:00", "2024-01-01T01Z"),
            ("2024-12-31T23:59:60Z", "2025Z"),
            ("2024-02-29T23:59:59.123456Z", "2024-02-29T23:59:59.123Z"),
        ] {
            let value = instant(left).expect(left);
            assert_eq!(instant(right), Some(value));
        }
    }

    #[test]
    fn unsupported_dispatch_and_invalid_metadata_refuse_the_whole_form() {
        for source in [
            "[#unknown :value :public]",
            "[#_ #unknown :value :public]",
            "[#?(:cljs :private) :public]",
            "[^:meta :keyword]",
            "[^true []]",
            "[bad/ns/name]",
            "[#uuid :not-a-string]",
        ] {
            assert_eq!(parse_strict(source), None, "{source}");
        }
        assert_eq!(
            parse_strict("#^:private ^{:doc :private} [1 #_ ^:private [] 2]"),
            parse_strict("[1 2]")
        );
    }

    #[test]
    fn reader_discards_preserve_collection_membership_and_limits() {
        assert_eq!(parse_strict("{#_ :ignored :a 1}"), parse_strict("{:a 1}"));
        assert_eq!(
            parse_strict("#_ [unused] {:a #_ 0 1}"),
            parse_strict("{:a 1}")
        );
        assert_eq!(parse_strict("[#_ 0 1 #_ 2]"), parse_strict("[1]"));
        assert_eq!(parse_strict("(1 #_ 2)"), parse_strict("(1)"));
        assert_eq!(parse_strict("{:a #_}"), None);
        assert_eq!(
            parse_strict(&format!("{}nil", "#_ ".repeat(MAX_DEPTH + 1))),
            None
        );
    }

    #[test]
    fn roundtrip_basic() {
        let src = r#"{:highlights [{:id "abc" :page 42 :properties {:color "yellow"}}] :extra {}}"#;
        let e = parse(src).unwrap();
        let hl = e.get("highlights").unwrap().as_vec().unwrap();
        assert_eq!(hl.len(), 1);
        assert_eq!(hl[0].get("id").unwrap().as_str(), Some("abc"));
        assert_eq!(hl[0].get("page").unwrap().as_i64(), Some(42));
        // reparse our own output
        let again = parse(&to_string(&e)).unwrap();
        assert_eq!(again, e);
    }

    #[test]
    fn parses_floats_and_nil() {
        let e = parse(r#"{:top 100.5 :image nil}"#).unwrap();
        assert_eq!(e.get("top").unwrap().as_f64(), Some(100.5));
        assert_eq!(e.get("image"), Some(&Edn::Nil));
    }

    #[test]
    fn parses_and_preserves_logseq_uuid_tags_and_rect_lists() {
        let src = r#"{:id #uuid "6a5604f8-a337-4336-a711-2ba6bc14fbfd"
            :rects ({:x1 10 :y1 20 :x2 30 :y2 40})}"#;
        let parsed = parse_strict(src).unwrap();
        assert_eq!(
            parsed.get("id").and_then(Edn::as_str),
            Some("6a5604f8-a337-4336-a711-2ba6bc14fbfd")
        );
        assert_eq!(parsed.get("rects").and_then(Edn::as_vec).unwrap().len(), 1);
        assert_eq!(parse_strict(&to_string(&parsed)), Some(parsed));
    }

    #[test]
    fn malformed_or_excessively_nested_collections_fail_closed() {
        for src in ["(", "[1 2", "{:a 1", "(#uuid", "]", ")", "}", ":"] {
            assert_eq!(parse_strict(src), None, "accepted malformed EDN: {src}");
        }

        let too_deep = format!(
            "{}nil{}",
            "[".repeat(MAX_DEPTH + 1),
            "]".repeat(MAX_DEPTH + 1)
        );
        assert_eq!(parse_strict(&too_deep), None);
    }

    #[test]
    fn every_escape_decodes_or_fails_closed_without_panicking() {
        // `\` before every byte class, including multi-byte chars, at every
        // position of a short string and at end of input (C3U / L01 H3).
        let tails = [
            "é",
            "\u{1F600}",
            "日本",
            "a",
            "z",
            "0",
            "7",
            "8",
            "u",
            "u12",
            "uD83D",
            "uD83Dx",
            "uD83D\\uDE00",
            "uDE00",
            "u00e9",
            "400",
            "377",
            "\"",
            "\\",
            "b",
            "f",
            "t",
            "r",
            "n",
            "",
            " ",
            "\n",
        ];
        for tail in tails {
            for prefix in ["", "x", "é", "ab"] {
                for close in ["\"", ""] {
                    let src = format!("\"{prefix}\\{tail}{close}");
                    let parsed = std::panic::catch_unwind(|| parse_strict(&src));
                    let parsed = parsed.unwrap_or_else(|_| panic!("parser panicked on {src:?}"));
                    if let Some(value) = parsed {
                        assert_eq!(
                            parse_strict(&to_string(&value)).as_ref(),
                            Some(&value),
                            "write-back changed {src:?}"
                        );
                    }
                }
            }
        }
        assert_eq!(parse_strict("\"\\é\""), None, "unknown escape is not EDN");
        assert_eq!(parse_strict("\"\\a\""), None, "unknown escape is not EDN");
    }

    #[test]
    fn every_prefix_of_a_mixed_document_parses_or_fails_without_panicking() {
        let src = "{:a \"caf\\u00e9 \\t\\b\\f\\101 日本\" :b \\é :c #{1 2} :d ##NaN :e foo/bar :f 1N :g [\\( \\space]}";
        for end in (0..=src.len()).filter(|end| src.is_char_boundary(*end)) {
            let _ = std::panic::catch_unwind(|| parse_strict(&src[..end]))
                .unwrap_or_else(|_| panic!("parser panicked on prefix {end}"));
        }
        // Byte-level truncations that are not char boundaries cannot reach
        // the parser (it takes &str), so boundaries are the whole input space.
        let parsed = parse_strict(src).expect("valid EDN");
        assert_eq!(
            parsed.get("a").and_then(Edn::as_str),
            Some("café \t\u{8}\u{c}A 日本")
        );
    }

    #[test]
    fn escapes_decode_like_the_edn_reader() {
        let cases = [
            (r#""\b\f""#, "\u{8}\u{c}"),
            (r#""\u00e9""#, "é"),
            (r#""\uD83D\uDE00""#, "\u{1F600}"),
            (r#""\101\0""#, "A\u{0}"),
        ];
        for (src, want) in cases {
            assert_eq!(parse_strict(src), Some(Edn::Str(want.into())), "{src}");
        }
        for bad in [r#""\uD83D""#, r#""\uDE00""#, r#""\u12""#, r#""\400""#] {
            assert_eq!(parse_strict(bad), None, "{bad}");
        }
    }

    #[test]
    fn uninterpreted_values_keep_their_type_and_spelling_on_write_back() {
        let src = "{:sym foo.bar/baz :ch \\a :paren \\( :nan ##NaN :inf ##-Inf :big 1N :dec 1.5M :word inf :set #{1 :a} :t \"a\\tb\"}";
        let parsed = parse_strict(src).unwrap();
        let written = to_string(&parsed);
        for token in [
            ":sym foo.bar/baz",
            ":ch \\a",
            ":paren \\(",
            ":nan ##NaN",
            ":inf ##-Inf",
            ":big 1N",
            ":dec 1.5M",
            ":word inf",
            ":set #{1 :a}",
            r#":t "a\tb""#,
        ] {
            assert!(written.contains(token), "{token} changed: {written}");
        }
        assert_eq!(parse_strict(&written), Some(parsed));
    }
}
