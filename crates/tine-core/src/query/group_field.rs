//! Canonical group-field grammar shared by native query readback and WASM.
//! O(value bytes), borrowed result, no allocation or I/O. Rust Unicode trim;
//! builtin or nonempty prop:/formula: token, excluding NUL and line breaks.

/// The six sheet builtins a canonical grouping `FieldId` can name.
pub(crate) const GROUP_BUILTINS: [&str; 6] =
    ["state", "priority", "scheduled", "deadline", "tags", "page"];

/// A token that could survive a property line at all. `;` is legal in a
/// grouping value (it is a single field, not a list) but a NUL or a line break
/// is not: it would not read back.
pub(crate) fn group_token_serializable(token: &str) -> bool {
    !token.contains(|c| matches!(c, '\0' | '\r' | '\n'))
}

/// The **new** key's grammar: exactly a builtin, or `prop:`/`formula:` with a
/// nonempty suffix, after trimming. Anything else — including the empty value —
/// is an explicit no-grouping statement rather than a value to guess at.
pub fn canonical_group_token(value: &str) -> Option<&str> {
    let token = value.trim();
    if token.is_empty() || !group_token_serializable(token) {
        return None;
    }
    if GROUP_BUILTINS.contains(&token) {
        return Some(token);
    }
    if let Some(rest) = token.strip_prefix("prop:") {
        return (!rest.is_empty()).then_some(token);
    }
    if let Some(rest) = token.strip_prefix("formula:") {
        return (!rest.is_empty()).then_some(token);
    }
    None
}
