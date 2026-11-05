//! Native recognition of order-only advanced result transforms (GH #852).
//! No Clojure executes. Every key must be understood before any sort runs.
//! OG 0.10.9 c67b8b5f: graph-parser/block.cljs `construct-block` preserves raw
//! content and priority; `timestamps->scheduled-and-deadline` stores yyyymmdd
//! numbers. db/query_react.cljs `custom-query-result-transform` passes those
//! pulled values unchanged to the transform (apart from enriching the page).

use super::ir::{Field, SortDir};
use super::sort::SortDecor;
use crate::edn::{self, Edn};
use crate::query_edn::{self, Form, Kind};

#[derive(Clone, Copy, Debug)]
enum Key {
    Priority,
    Scheduled,
    Deadline,
    Content,
}

impl Key {
    fn name(self) -> &'static str {
        match self {
            Self::Priority => "priority",
            Self::Scheduled => "scheduled",
            Self::Deadline => "deadline",
            Self::Content => "content",
        }
    }

    fn numeric(self) -> bool {
        matches!(self, Self::Scheduled | Self::Deadline)
    }

    fn read(self, row: &AdvancedSortRow<'_>) -> Option<SortDecor> {
        match self {
            Self::Priority => row.priority.map(utf16),
            Self::Scheduled => row.scheduled.map(SortDecor::Num),
            Self::Deadline => row.deadline.map(SortDecor::Num),
            Self::Content => row.content.map(utf16),
        }
    }
}

fn utf16(text: &str) -> SortDecor {
    SortDecor::Utf16(text.encode_utf16().collect())
}

#[derive(Debug)]
enum DefaultValue {
    Literal(SortDecor),
    Get(Box<Get>),
}

#[derive(Debug)]
struct Get {
    key: Key,
    default: DefaultValue,
}

impl Get {
    fn read(&self, row: &AdvancedSortRow<'_>) -> SortDecor {
        self.key.read(row).unwrap_or_else(|| match &self.default {
            DefaultValue::Literal(value) => value.clone(),
            DefaultValue::Get(get) => get.read(row),
        })
    }
}

/// Pulled OG attribute values, before presentation normalization. Absent
/// attributes are `None`; an empty content string is present.
pub struct AdvancedSortRow<'a> {
    /// Exact parsed priority, without case folding.
    pub priority: Option<&'a str>,
    /// Scheduled day as a yyyymmdd integer.
    pub scheduled: Option<i64>,
    /// Deadline day as a yyyymmdd integer.
    pub deadline: Option<i64>,
    /// Complete raw block body, including properties and continuation lines.
    pub content: Option<&'a str>,
}

/// Recognised single-key or `juxt` sort. It changes only row order.
#[derive(Debug)]
pub struct AdvancedResultSort {
    keys: Vec<Get>,
}

impl AdvancedResultSort {
    /// Ordinary field names exposed by the existing Display settings. A caller
    /// overriding these fields uses its normal display sort semantics.
    pub fn view_sort(&self) -> Vec<(Field, SortDir)> {
        self.keys
            .iter()
            .map(|get| (Field::new(get.key.name()), SortDir::Asc))
            .collect()
    }

    /// Precompute keys once per row. Nil precedes present values; text compares
    /// by UTF-16 code units, as ClojureScript strings do. The executor preserves
    /// its input order among equal keys.
    pub fn decorate(&self, row: &AdvancedSortRow<'_>) -> Vec<SortDecor> {
        self.keys.iter().map(|get| get.read(row)).collect()
    }
}

struct Reader<'a>(&'a str);

impl<'a> Reader<'a> {
    fn text(&self, form: &Form) -> &'a str {
        &self.0[form.span.clone()]
    }

    fn atom(&self, form: &Form, want: &str) -> bool {
        form.kind == Kind::Atom && self.text(form) == want
    }

    fn lambda<'f>(&self, form: &'f Form) -> Option<(&'a str, &'f Form)> {
        let [head, params, body] = &form.children[..] else {
            return None;
        };
        let [param] = &params.children[..] else {
            return None;
        };
        let name = self.text(param);
        let symbol = param.kind == Kind::Atom
            && matches!(edn::parse_strict(name), Some(Edn::Symbol(_)))
            && !name.contains(['/', '\\', '#'])
            && name != "&"
            && !name.starts_with(|c: char| c.is_ascii_digit());
        (form.kind == Kind::List && self.atom(head, "fn") && params.kind == Kind::Vector && symbol)
            .then_some((name, body))
    }

    fn get(&self, form: &Form, row: &str, numeric: Option<bool>) -> Option<Get> {
        let [head, of, attr, rest @ ..] = &form.children[..] else {
            return None;
        };
        if form.kind != Kind::List
            || !self.atom(head, "get")
            || !self.atom(of, row)
            || rest.len() > 1
        {
            return None;
        }
        let key = match self.text(attr) {
            ":block/priority" if attr.kind == Kind::Atom => Key::Priority,
            ":block/scheduled" if attr.kind == Kind::Atom => Key::Scheduled,
            ":block/deadline" if attr.kind == Kind::Atom => Key::Deadline,
            ":block/content" if attr.kind == Kind::Atom => Key::Content,
            _ => return None,
        };
        if numeric.is_some_and(|numeric| numeric != key.numeric()) {
            return None;
        }
        let default = match rest.first() {
            None => DefaultValue::Literal(SortDecor::Nil),
            Some(form) if form.kind == Kind::List => {
                DefaultValue::Get(Box::new(self.get(form, row, Some(key.numeric()))?))
            }
            Some(form) => DefaultValue::Literal(match edn::parse_strict(self.text(form))? {
                Edn::Nil => SortDecor::Nil,
                Edn::Str(value) if !key.numeric() => utf16(&value),
                // JS represents integers exactly only within this range.
                Edn::Int(value)
                    if key.numeric() && value.unsigned_abs() <= 9_007_199_254_740_991 =>
                {
                    SortDecor::Num(value)
                }
                _ => return None,
            }),
        };
        Some(Get { key, default })
    }
}

/// Recognise `(fn [result] (sort-by K result))`, where K is a one-argument
/// `fn` containing a `get`, or a nonempty `juxt` of those functions. Supported
/// attributes: priority, scheduled, deadline, content. Defaults may be nil,
/// a same-type scalar (string / safe integer), or another same-type supported
/// `get` on that row. Unknown keys, code or comparators refuse the entire form.
/// Cost O(source bytes), using the bounded, comment-aware EDN reader.
pub fn advanced_result_sort(src: &str) -> Option<AdvancedResultSort> {
    let value = query_edn::option_value(src, ":result-transform")?;
    let form = query_edn::read(value)?;
    let reader = Reader(value);
    let (result, body) = reader.lambda(&form)?;
    // These names would capture a function used inside the transform.
    if matches!(result, "sort-by" | "juxt" | "get") {
        return None;
    }
    let [head, key, over] = &body.children[..] else {
        return None;
    };
    if body.kind != Kind::List || !reader.atom(head, "sort-by") || !reader.atom(over, result) {
        return None;
    }
    let keys = if key.kind == Kind::List
        && key
            .children
            .first()
            .is_some_and(|head| reader.atom(head, "juxt"))
    {
        if key.children.len() < 2 {
            return None;
        }
        &key.children[1..]
    } else {
        std::slice::from_ref(key)
    };
    let keys = keys
        .iter()
        .map(|key| {
            let (row, body) = reader.lambda(key)?;
            if row == "get" {
                return None;
            }
            reader.get(body, row, None)
        })
        .collect::<Option<Vec<_>>>()?;
    Some(AdvancedResultSort { keys })
}
