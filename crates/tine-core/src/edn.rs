//! A small EDN value model + parser/writer — just enough for the PDF highlight
//! files Logseq writes (`assets/<key>.edn`): nil/bool/int/float/string/keyword,
//! vectors, and maps. Not a full EDN implementation.

use std::fmt::Write as _;

#[derive(Debug, Clone, PartialEq)]
pub enum Edn {
    Nil,
    Bool(bool),
    Int(i64),
    Float(f64),
    Str(String),
    Keyword(String), // without the leading ':'
    Vec(Vec<Edn>),
    Map(Vec<(Edn, Edn)>),
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
            Edn::Str(s) => Some(s),
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
            Edn::Vec(v) => Some(v),
            _ => None,
        }
    }
}

pub fn parse(input: &str) -> Option<Edn> {
    let mut p = Parser {
        s: input.as_bytes(),
        i: 0,
        src: input,
    };
    p.skip_ws();
    let v = p.value()?;
    p.skip_ws();
    Some(v)
}

/// Parse exactly one EDN value, rejecting any non-whitespace/comment suffix.
/// Mutation baselines use this stricter form so recoverable trailing bytes are
/// never silently discarded by a parse-and-reserialize cycle.
pub fn parse_strict(input: &str) -> Option<Edn> {
    let mut p = Parser {
        s: input.as_bytes(),
        i: 0,
        src: input,
    };
    p.skip_ws();
    let value = p.value()?;
    p.skip_ws();
    (p.i == p.s.len()).then_some(value)
}

struct Parser<'a> {
    s: &'a [u8],
    i: usize,
    src: &'a str,
}

impl<'a> Parser<'a> {
    fn skip_ws(&mut self) {
        while self.i < self.s.len() {
            let c = self.s[self.i];
            if c == b',' || c.is_ascii_whitespace() {
                self.i += 1;
            } else if c == b';' {
                while self.i < self.s.len() && self.s[self.i] != b'\n' {
                    self.i += 1;
                }
            } else {
                break;
            }
        }
    }

    fn value(&mut self) -> Option<Edn> {
        self.skip_ws();
        if self.i >= self.s.len() {
            return None;
        }
        match self.s[self.i] {
            b'{' => self.map(),
            b'[' => self.vector(),
            b'"' => self.string(),
            b':' => self.keyword(),
            _ => self.atom(),
        }
    }

    fn map(&mut self) -> Option<Edn> {
        self.i += 1; // {
        let mut pairs = Vec::new();
        loop {
            self.skip_ws();
            if self.i >= self.s.len() {
                return None;
            }
            if self.s[self.i] == b'}' {
                self.i += 1;
                break;
            }
            let k = self.value()?;
            let v = self.value()?;
            pairs.push((k, v));
        }
        Some(Edn::Map(pairs))
    }

    fn vector(&mut self) -> Option<Edn> {
        self.i += 1; // [
        let mut items = Vec::new();
        loop {
            self.skip_ws();
            if self.i >= self.s.len() {
                return None;
            }
            if self.s[self.i] == b']' {
                self.i += 1;
                break;
            }
            items.push(self.value()?);
        }
        Some(Edn::Vec(items))
    }

    fn string(&mut self) -> Option<Edn> {
        self.i += 1; // opening quote
        let mut out = String::new();
        while self.i < self.s.len() {
            let c = self.s[self.i];
            if c == b'\\' && self.i + 1 < self.s.len() {
                let n = self.s[self.i + 1];
                out.push(match n {
                    b'n' => '\n',
                    b't' => '\t',
                    b'r' => '\r',
                    b'"' => '"',
                    b'\\' => '\\',
                    other => other as char,
                });
                self.i += 2;
            } else if c == b'"' {
                self.i += 1;
                return Some(Edn::Str(out));
            } else {
                // copy a full UTF-8 char
                let ch = self.src[self.i..].chars().next()?;
                out.push(ch);
                self.i += ch.len_utf8();
            }
        }
        None
    }

    fn keyword(&mut self) -> Option<Edn> {
        self.i += 1; // ':'
        let start = self.i;
        while self.i < self.s.len() && !is_delim(self.s[self.i]) {
            self.i += 1;
        }
        Some(Edn::Keyword(self.src[start..self.i].to_string()))
    }

    fn atom(&mut self) -> Option<Edn> {
        let start = self.i;
        while self.i < self.s.len() && !is_delim(self.s[self.i]) {
            self.i += 1;
        }
        let tok = &self.src[start..self.i];
        Some(match tok {
            "nil" => Edn::Nil,
            "true" => Edn::Bool(true),
            "false" => Edn::Bool(false),
            _ => {
                if let Ok(n) = tok.parse::<i64>() {
                    Edn::Int(n)
                } else if let Ok(f) = tok.parse::<f64>() {
                    Edn::Float(f)
                } else {
                    // Unknown symbol — keep as a string so round-trips don't lose it.
                    Edn::Str(tok.to_string())
                }
            }
        })
    }
}

fn is_delim(c: u8) -> bool {
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
                    _ => out.push(ch),
                }
            }
            out.push('"');
        }
        Edn::Keyword(k) => {
            out.push(':');
            out.push_str(k);
        }
        Edn::Vec(items) => {
            out.push('[');
            for (i, it) in items.iter().enumerate() {
                if i > 0 {
                    out.push(' ');
                }
                write_edn(it, out);
            }
            out.push(']');
        }
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
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
}
