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
    fn is_symbol(&self) -> bool {
        matches!(&self.leaf, Some(Edn::Symbol(raw))
            if self.kind == Kind::Atom
                && !starts_numeric(raw)
                && !raw.starts_with(['\\', '#', '[']))
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
                let target = self.form(depth + 1)?;
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
                // The returned span starts at the effective value. Authored
                // metadata remains available in the preceding source bytes.
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
                children = self.collection(depth, close, kind)?;
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
                children = self.collection(depth, b'}', Kind::Set)?;
                Kind::Set
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
                    "uuid" => matches!(&value.leaf, Some(Edn::Str(id)) if id.is_ascii()),
                    "queue" => value.kind == Kind::Vector,
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
                    _ if keyword && valid_name(token) && !token.contains("::") => {
                        Some(Edn::Keyword(token.into()))
                    }
                    _ if keyword => None,
                    "nil" => Some(Edn::Nil),
                    "true" => Some(Edn::Bool(true)),
                    "false" => Some(Edn::Bool(false)),
                    _ if numeric => number(token).or_else(|| {
                        (self.mode == Mode::ImageMetadata
                            || (matches!(self.mode, Mode::Query | Mode::QuerySyntax | Mode::Macro)
                                && crate::query::date_token::DateToken::parse(token).is_some()))
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

    fn collection(&mut self, depth: usize, close: u8, kind: Kind) -> Option<Vec<Form>> {
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
                if self.mode == Mode::ImageMetadata && depth == 0 {
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
        || token.starts_with(':')
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
        _ if token.starts_with('u') && token.len() == 5 => {
            char::from_u32(u32::from_str_radix(&token[1..], 16).ok()?)?
        }
        _ => return None,
    })
}

fn number(token: &str) -> Option<Edn> {
    let plain = token.trim_start_matches(['+', '-']);
    if plain.starts_with('0') && plain.as_bytes().get(1).is_some_and(u8::is_ascii_digit) {
        return None; // This spelling may denote octal.
    }
    let decimal = token.strip_suffix(['M', 'N']).unwrap_or(token);
    let int = decimal.parse::<i64>().ok();
    let float = decimal.parse::<f64>().ok().filter(|n| n.is_finite());
    if token.ends_with('N') && int.is_none() {
        return None;
    }
    let n = float?;
    if decimal.len() != token.len() {
        Some(Edn::Symbol(token.into()))
    } else {
        Some(
            int.filter(|_| n.abs() < 9_007_199_254_740_992.0)
                .map_or(Edn::Float(n), Edn::Int),
        )
    }
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
                .map(str::to_ascii_lowercase)
                .hash(&mut state);
        }
        (Kind::Tagged, _) => {
            form.tag().hash(&mut state);
            identity(&form.children[0], hash).hash(&mut state);
        }
        (_, Some(Edn::Int(n))) => return numeric_identity(*n as f64, hash),
        (_, Some(Edn::Float(n))) => return numeric_identity(*n, hash),
        (_, Some(Edn::Symbol(raw))) if raw.starts_with('\\') => {
            "string".hash(&mut state);
            let mut buf = [0; 4];
            if let Some(c) = character(&raw[1..]) {
                c.encode_utf8(&mut buf).hash(&mut state);
            }
        }
        (_, Some(Edn::Symbol(raw))) if starts_numeric(raw) => {
            if let Ok(n) = raw.trim_end_matches(['N', 'M']).parse() {
                return numeric_identity(n, hash);
            }
            ("symbol", raw).hash(&mut state);
        }
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
            r#"#{\a "\u0061"}"#,
            r#"#{#_ {:x 1 :x 2} :public}"#,
        ] {
            assert_eq!(parse_strict(source), None, "{source}");
        }
        assert_eq!(parse_strict("#{1 #_ 1 2}"), parse_strict("#{1 2}"));
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
