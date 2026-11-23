//! Equality used by the CLJS reader to reject duplicate keys and members.

use super::{character, instant, number, starts_numeric, Edn, Form, Kind};
use std::collections::{HashMap, HashSet};
use std::hash::{BuildHasher, Hash, Hasher, RandomState};

// Duplicate checks use the fingerprint to select candidates and compare the
// decoded forms on a match. This never materializes a second tree.
pub(super) struct FormKey<'a> {
    form: &'a Form,
    hash: &'a RandomState,
    fingerprint: u64,
}

impl<'a> FormKey<'a> {
    pub(super) fn new(form: &'a Form, hash: &'a RandomState) -> Self {
        Self {
            form,
            hash,
            fingerprint: identity(form, hash),
        }
    }
}

impl Hash for FormKey<'_> {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.fingerprint.hash(state);
    }
}

impl PartialEq for FormKey<'_> {
    fn eq(&self, other: &Self) -> bool {
        self.fingerprint == other.fingerprint && equivalent(self.form, other.form, self.hash)
    }
}

impl Eq for FormKey<'_> {}

fn equivalent(a: &Form, b: &Form, hash: &RandomState) -> bool {
    if std::ptr::eq(a, b) {
        return true;
    }
    if let (Some(a), Some(b)) = (a.sequence(), b.sequence()) {
        return a.len() == b.len() && a.iter().zip(b).all(|(a, b)| equivalent(a, b, hash));
    }
    match (a.kind, b.kind) {
        (Kind::Map, Kind::Map) if a.children.len() == b.children.len() => {
            let pairs: HashMap<_, _> = a
                .children
                .chunks_exact(2)
                .map(|pair| (FormKey::new(&pair[0], hash), &pair[1]))
                .collect();
            b.children.chunks_exact(2).all(|pair| {
                pairs
                    .get(&FormKey::new(&pair[0], hash))
                    .is_some_and(|a| equivalent(a, &pair[1], hash))
            })
        }
        (Kind::Set, Kind::Set) if a.children.len() == b.children.len() => {
            let members: HashSet<_> = a
                .children
                .iter()
                .map(|item| FormKey::new(item, hash))
                .collect();
            b.children
                .iter()
                .all(|item| members.contains(&FormKey::new(item, hash)))
        }
        (Kind::Tagged, Kind::Tagged) => match (a.tag(), b.tag()) {
            (Some("uuid"), Some("uuid")) => {
                uuid_units(&a.children[0]) == uuid_units(&b.children[0])
            }
            (Some("inst"), Some("inst")) => instant_value(a) == instant_value(b),
            _ => false,
        },
        (Kind::String | Kind::Atom, Kind::String | Kind::Atom) => {
            match (numeric_value(a), numeric_value(b)) {
                (Some(a), Some(b)) => a == b,
                (Some(_), _) | (_, Some(_)) => false,
                _ => match (legacy_character(a), legacy_character(b)) {
                    (Some(a), Some(b)) => a == b,
                    (Some(c), _) => char_matches(c, b),
                    (_, Some(c)) => char_matches(c, a),
                    _ => a.leaf == b.leaf,
                },
            }
        }
        _ => false,
    }
}

fn char_matches(c: char, form: &Form) -> bool {
    matches!(&form.leaf, Some(Edn::Str(text)) if text == c.encode_utf8(&mut [0; 4]))
}

fn identity(form: &Form, hash: &impl BuildHasher) -> u64 {
    let mut state = hash.build_hasher();
    if let Some(sequence) = form.sequence() {
        "sequence".hash(&mut state);
        for item in sequence {
            identity(item, hash).hash(&mut state);
        }
    } else if let Some(number) = numeric_value(form) {
        let bits = if number.is_nan() {
            form.span.start as u64
        } else if number == 0.0 {
            0
        } else {
            number.to_bits()
        };
        ("number", bits).hash(&mut state);
    } else {
        match (form.kind, form.leaf.as_ref()) {
            (Kind::Map, _) => {
                let sum = form.children.chunks_exact(2).fold(0u64, |sum, pair| {
                    sum.wrapping_add(
                        hash.hash_one((identity(&pair[0], hash), identity(&pair[1], hash))),
                    )
                });
                ("map", sum).hash(&mut state);
            }
            (Kind::Set, _) => {
                let sum = form
                    .children
                    .iter()
                    .fold(0u64, |sum, child| sum.wrapping_add(identity(child, hash)));
                ("set", sum).hash(&mut state);
            }
            (Kind::Tagged, _) => match form.tag() {
                Some("uuid") => ("uuid", uuid_units(&form.children[0])).hash(&mut state),
                Some("inst") => ("inst", instant_value(form)).hash(&mut state),
                // Every occurrence of #js has its own identity.
                _ => ("tagged", form.span.start).hash(&mut state),
            },
            (_, Some(Edn::Symbol(raw))) => {
                if let Some(c) = legacy_character(form) {
                    ("string", &*c.encode_utf8(&mut [0; 4])).hash(&mut state);
                } else {
                    ("symbol", raw).hash(&mut state);
                }
            }
            (_, Some(Edn::Keyword(raw))) => ("keyword", raw).hash(&mut state),
            (_, Some(Edn::Str(text))) => ("string", text).hash(&mut state),
            (_, Some(Edn::Utf16(units))) => ("utf16", units).hash(&mut state),
            (_, Some(Edn::Bool(value))) => ("bool", value).hash(&mut state),
            _ => "nil".hash(&mut state),
        }
    }
    state.finish()
}

fn numeric_value(form: &Form) -> Option<f64> {
    match form.leaf.as_ref()? {
        Edn::Symbol(raw) if raw == "##NaN" => Some(f64::NAN),
        Edn::Symbol(raw) if raw == "##Inf" => Some(f64::INFINITY),
        Edn::Symbol(raw) if raw == "##-Inf" => Some(f64::NEG_INFINITY),
        Edn::Symbol(raw) if starts_numeric(raw) && raw.ends_with(['M', 'N']) => {
            number(raw)?.as_f64()
        }
        value => value.as_f64(),
    }
}

fn legacy_character(form: &Form) -> Option<char> {
    match form.leaf.as_ref()? {
        Edn::Symbol(raw) => character(raw.strip_prefix('\\')?),
        _ => None,
    }
}

fn instant_value(form: &Form) -> Option<i64> {
    instant(form.children[0].leaf.as_ref()?.as_str()?)
}

fn uuid_units(form: &Form) -> Option<Vec<u16>> {
    form.leaf.as_ref().map(lowercase_units)
}

pub(super) fn lowercase_units(value: &Edn) -> Vec<u16> {
    match value {
        Edn::Str(text) => text.to_lowercase().encode_utf16().collect(),
        Edn::Utf16(units) => {
            let mut lower = Vec::new();
            let mut text = String::new();
            for unit in char::decode_utf16(units.iter().copied()) {
                match unit {
                    Ok(ch) => text.push(ch),
                    Err(err) => {
                        lower.extend(text.to_lowercase().encode_utf16());
                        text.clear();
                        lower.push(err.unpaired_surrogate());
                    }
                }
            }
            lower.extend(text.to_lowercase().encode_utf16());
            lower
        }
        _ => Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_fingerprint_collision_is_never_equality() {
        let hash = RandomState::new();
        let src = "{:a [1 2]} {:a [1 3]} {:a (1.0 2)}";
        let forms = super::super::forms(src, super::super::Mode::Edn).unwrap();
        let key = |index: usize| FormKey {
            form: &forms[index],
            hash: &hash,
            fingerprint: 5,
        };
        assert!(key(0) != key(1));
        assert!(key(0) == key(2));
    }
}
