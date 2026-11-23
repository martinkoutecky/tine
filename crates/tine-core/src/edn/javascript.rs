//! The own properties produced by ClojureScript's #js map tag.
//! CLJS applies `name` to keys in the decoded map's iteration order. Its
//! array maps preserve order for up to eight entries. Larger maps traverse
//! the hash trie in five-bit hash order, including when two names collide.

use super::Edn;
use std::collections::BTreeMap;

pub(super) fn properties(mut pairs: Vec<(Edn, Edn)>) -> Vec<(Edn, Edn)> {
    if pairs.len() > 8 {
        pairs.sort_by_cached_key(|(key, _)| {
            let hash = cljs_hash(key);
            std::array::from_fn::<_, 7, _>(|digit| (hash >> (digit * 5)) & 31)
        });
    }
    let mut properties = BTreeMap::new();
    let mut prototype_setter = true;
    for (key, value) in pairs {
        let key = match key {
            Edn::Symbol(key) | Edn::Keyword(key) => Edn::Str(parts(&key).1.into()),
            key => key,
        };
        if key.text() == Some("__proto__") && prototype_setter {
            // Assignment to Object.prototype's setter does not create an own
            // property. A null prototype removes that inherited setter.
            prototype_setter = value != Edn::Nil;
        } else {
            properties.insert(key.string_units(), (key, value));
        }
    }
    properties.into_values().collect()
}

fn parts(key: &str) -> (&str, &str) {
    key.split_once('/')
        .filter(|(ns, _)| !ns.is_empty())
        .unwrap_or(("", key))
}

// CLJS core.cljs hash, hash-symbol and hash-keyword. String hashing operates
// on UTF-16, including lone units, and Murmur3 uses wrapping 32-bit arithmetic.
fn cljs_hash(key: &Edn) -> u32 {
    match key {
        Edn::Symbol(name) | Edn::Keyword(name) => {
            let (ns, name) = parts(name);
            let mut units = name.encode_utf16();
            let mut hash = 0;
            while let Some(low) = units.next() {
                let Some(high) = units.next() else {
                    hash ^= mix_key(u32::from(low));
                    break;
                };
                hash = mix_hash(hash, u32::from(low) | (u32::from(high) << 16));
            }
            let seed = finish(hash, (name.encode_utf16().count() * 2) as u32);
            let combined = seed
                ^ hash_string(ns.encode_utf16())
                    .wrapping_add(0x9e37_79b9)
                    .wrapping_add(seed << 6)
                    .wrapping_add(((seed as i32) >> 2) as u32);
            if matches!(key, Edn::Keyword(_)) {
                combined.wrapping_add(0x9e37_79b9)
            } else {
                combined
            }
        }
        Edn::Str(text) => hash_int(hash_string(text.encode_utf16())),
        Edn::Utf16(units) => hash_int(hash_string(units.iter().copied())),
        _ => unreachable!("validated #js map key"),
    }
}

fn hash_string(units: impl Iterator<Item = u16>) -> u32 {
    units.fold(0u32, |hash, unit| {
        hash.wrapping_mul(31).wrapping_add(u32::from(unit))
    })
}

fn mix_key(key: u32) -> u32 {
    key.wrapping_mul(0xcc9e_2d51)
        .rotate_left(15)
        .wrapping_mul(0x1b87_3593)
}

fn mix_hash(hash: u32, key: u32) -> u32 {
    (hash ^ mix_key(key))
        .rotate_left(13)
        .wrapping_mul(5)
        .wrapping_add(0xe654_6b64)
}

fn hash_int(value: u32) -> u32 {
    if value == 0 {
        0
    } else {
        finish(mix_hash(0, value), 4)
    }
}

fn finish(hash: u32, len: u32) -> u32 {
    let hash = hash ^ len;
    let hash = (hash ^ (hash >> 16)).wrapping_mul(0x85eb_ca6b);
    let hash = (hash ^ (hash >> 13)).wrapping_mul(0xc2b2_ae35);
    hash ^ (hash >> 16)
}
