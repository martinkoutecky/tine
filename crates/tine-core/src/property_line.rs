//! Shared Markdown property-line recognition for `tine-core` and `lsdoc-wasm`.
//!
//! Keep this module dependency-free: the browser WASM wrapper includes this
//! exact source alongside `logbook.rs` instead of compiling all of tine-core.

/// lsdoc's parser-space set (`Parsers.is_space`: space, tab, SUB, FF).
fn mldoc_space(byte: u8) -> bool {
    matches!(byte, b' ' | b'\t' | 0x1a | 0x0c)
}

fn skip_mldoc_spaces(s: &str) -> &str {
    let bytes = s.as_bytes();
    let mut i = 0;
    while i < bytes.len() && mldoc_space(bytes[i]) {
        i += 1;
    }
    &s[i..]
}

/// lsdoc's property-value trim set: space, tab, newline, CR and FF, but not SUB.
fn trim_property_value(s: &str) -> &str {
    fn trim(byte: u8) -> bool {
        matches!(byte, b' ' | b'\t' | b'\n' | b'\r' | 0x0c)
    }
    let bytes = s.as_bytes();
    let mut start = 0usize;
    let mut end = bytes.len();
    while start < end && trim(bytes[start]) {
        start += 1;
    }
    while end > start && trim(bytes[end - 1]) {
        end -= 1;
    }
    &s[start..end]
}

/// The one hand-rolled Markdown property-line recognizer (`key:: value`).
///
/// This is transcribed from lsdoc's `markdown_property_line` for callers that
/// need a cheap borrowed view rather than an allocated document AST:
///
/// - leading lsdoc parser spaces (space/tab/SUB/FF) are skipped;
/// - the key is non-empty and contains no colon, parser space, CR or LF;
/// - `::` must be followed by a literal space, unless the remainder consists
///   only of parser spaces (an empty value).
pub fn parse_property_line(line: &str) -> Option<(&str, &str)> {
    let rest = skip_mldoc_spaces(line);
    let pos = rest.find("::")?;
    let key = &rest[..pos];
    if key.is_empty()
        || key
            .as_bytes()
            .iter()
            .any(|&b| b == b':' || mldoc_space(b) || b == b'\n' || b == b'\r')
    {
        return None;
    }
    let value = &rest[pos + 2..];
    if let Some(value) = value.strip_prefix(' ') {
        let value = skip_mldoc_spaces(value);
        return Some((key, trim_property_value(value)));
    }
    // The empty value must still BORROW from `line`. A `""` literal here points
    // into static memory, and the callers that recover source offsets by
    // pointer arithmetic (`reference_evidence`'s property sources) subtract the
    // line's base from it — which underflows and panics on a bare `key::`.
    value
        .as_bytes()
        .iter()
        .all(|&b| mldoc_space(b))
        .then(|| (key, &value[value.len()..]))
}

#[cfg(test)]
mod tests {
    use super::parse_property_line;

    /// Both returned slices must point INTO the argument: callers recover the
    /// key's and value's source offsets by subtracting the line's base pointer,
    /// so a slice borrowed from anywhere else underflows.
    #[test]
    fn both_halves_are_borrowed_from_the_line_even_when_the_value_is_empty() {
        for line in ["k:: v", "k:: ", "k::", "\tk::  spaced  "] {
            let (key, value) = parse_property_line(line).expect(line);
            let base = line.as_ptr() as usize;
            let end = base + line.len();
            for part in [key, value] {
                let at = part.as_ptr() as usize;
                assert!(at >= base && at + part.len() <= end, "{line:?} / {part:?}");
            }
        }
    }

    #[test]
    fn matches_lsdoc_property_boundaries() {
        assert_eq!(
            parse_property_line("\tlogseq.order-list-type::  number \t"),
            Some(("logseq.order-list-type", "number"))
        );
        assert_eq!(
            parse_property_line("unicode.klíč:: hodnota"),
            Some(("unicode.klíč", "hodnota"))
        );
        assert_eq!(parse_property_line("empty:: \t"), Some(("empty", "")));
        assert_eq!(parse_property_line("empty::"), Some(("empty", "")));
        assert_eq!(parse_property_line("key::value"), None);
        assert_eq!(parse_property_line("a b:: value"), None);
    }
}
