//! Shared ordering primitives for result rows.
//!
//! Row kinds remain responsible for deriving their own decorations and stable
//! tie. This module only compares already-computed keys and supplies the common
//! lexical property-or-fallback rule, so a comparator never parses row text.

use crate::doc::property_key_norm;
use std::cmp::Ordering;

/// A result row's precomputed sort key: a numeric axis or lexical text.
/// Within one requested field every row supplies the same variant.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub enum SortDecor {
    Num(i64),
    Text(String),
}

/// Compare precomputed multi-field keys in their requested directions.
///
/// The caller owns the stable tie because different row kinds use different
/// base identities. Direction applies only to requested keys, never to that tie.
pub fn compare_sort_decorations(
    left: &[SortDecor],
    right: &[SortDecor],
    ascending: &[bool],
) -> Ordering {
    debug_assert_eq!(left.len(), right.len());
    debug_assert_eq!(left.len(), ascending.len());
    left.iter()
        .zip(right)
        .zip(ascending)
        .map(|((left, right), ascending)| {
            let order = left.cmp(right);
            if *ascending {
                order
            } else {
                order.reverse()
            }
        })
        .find(|order| !order.is_eq())
        .unwrap_or(Ordering::Equal)
}

/// The existing saved-sort property rule, independent of a row DTO.
///
/// Match the first property whose normalized key equals the requested field,
/// lowercase its value for lexical ordering, and invoke the row kind's fallback
/// only when no property matched. The fallback is lowercased here by the same
/// rule, so callers provide exact row text rather than a pre-normalized shadow.
pub fn lexical_property_sort_text<'a>(
    properties: impl IntoIterator<Item = (&'a str, &'a str)>,
    field: &str,
    fallback: impl FnOnce() -> String,
) -> String {
    let field = property_key_norm(field);
    properties
        .into_iter()
        .find(|(key, _)| property_key_norm(key) == field)
        .map_or_else(
            || fallback().to_lowercase(),
            |(_, value)| value.to_lowercase(),
        )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;

    #[test]
    fn decoration_comparison_applies_each_direction_and_leaves_ties_equal() {
        let left = [SortDecor::Text("same".into()), SortDecor::Num(9)];
        let right = [SortDecor::Text("same".into()), SortDecor::Num(3)];
        assert_eq!(
            compare_sort_decorations(&left, &right, &[true, false]),
            Ordering::Less,
            "the descending secondary key puts nine before three"
        );
        assert_eq!(
            compare_sort_decorations(&left, &right, &[true, true]),
            Ordering::Greater,
            "the same secondary key reverses under ascending order"
        );
        assert_eq!(
            compare_sort_decorations(&left, &left, &[false, false]),
            Ordering::Equal,
            "the caller, not direction, supplies the stable tie"
        );
    }

    #[test]
    fn lexical_property_uses_first_normalized_match_and_lazy_unicode_fallback() {
        let fallback_calls = Cell::new(0);
        let properties = [
            ("  Project_Name ", "ŽLUŤOUČKÝ"),
            ("project-name", "later duplicate"),
        ];
        let present = lexical_property_sort_text(properties, "project name", || {
            fallback_calls.set(fallback_calls.get() + 1);
            "unused".into()
        });
        assert_eq!(present, "žluťoučký");
        assert_eq!(
            fallback_calls.get(),
            0,
            "a present property never parses fallback text"
        );

        let missing =
            lexical_property_sort_text(std::iter::empty::<(&str, &str)>(), "missing", || {
                fallback_calls.set(fallback_calls.get() + 1);
                "VISIBLE Ä".into()
            });
        assert_eq!(missing, "visible ä");
        assert_eq!(fallback_calls.get(), 1);
    }
}
