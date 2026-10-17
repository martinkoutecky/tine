//! Borrowed Markdown property-line recognition shared by core callers.
//!
//! The grammar follows lsdoc's `markdown_property_line`. It is a cheap O(line)
//! syntactic read with no allocation or I/O; invalid input returns `None`.

fn parser_space(byte: u8) -> bool {
    matches!(byte, b' ' | b'\t' | 0x1a | 0x0c)
}

fn skip_parser_spaces(s: &str) -> &str {
    let n = s
        .as_bytes()
        .iter()
        .take_while(|&&b| parser_space(b))
        .count();
    &s[n..]
}

fn trim_value(s: &str) -> &str {
    let trim = |b: u8| matches!(b, b' ' | b'\t' | b'\n' | b'\r' | 0x0c);
    let bytes = s.as_bytes();
    let start = bytes.iter().take_while(|&&b| trim(b)).count();
    let end = bytes
        .iter()
        .rposition(|&b| !trim(b))
        .map_or(start, |i| i + 1);
    &s[start..end]
}

/// Recognize lsdoc's Markdown `key:: value` line, returning slices of `line`.
///
/// Leading parser spaces (space, tab, SUB, FF) are allowed. The nonempty key
/// may contain any bytes except colon, parser spaces, CR and LF. A nonempty
/// value requires a literal space after `::`; no-space syntax is accepted only
/// for an empty value. Values are trimmed with lsdoc's property-value trim set.
/// Cost O(line), no allocation; malformed lines return `None`. Both slices
/// borrow from the argument so source-offset callers may subtract pointers.
pub fn parse_property_line(line: &str) -> Option<(&str, &str)> {
    let rest = skip_parser_spaces(line);
    let pos = rest.find("::")?;
    let key = &rest[..pos];
    if key.is_empty()
        || key
            .as_bytes()
            .iter()
            .any(|&b| b == b':' || parser_space(b) || b == b'\n' || b == b'\r')
    {
        return None;
    }
    let value = &rest[pos + 2..];
    if let Some(value) = value.strip_prefix(' ') {
        return Some((key, trim_value(skip_parser_spaces(value))));
    }
    value
        .as_bytes()
        .iter()
        .all(|&b| parser_space(b))
        .then(|| (key, &value[value.len()..]))
}

#[cfg(test)]
mod tests {
    use super::parse_property_line;

    #[test]
    fn follows_lsdoc_and_master_property_boundaries() {
        assert_eq!(
            parse_property_line("klíč:: hodnota"),
            Some(("klíč", "hodnota"))
        );
        assert_eq!(
            parse_property_line("\tlogseq.order-list-type::  number \t"),
            Some(("logseq.order-list-type", "number"))
        );
        assert_eq!(parse_property_line("empty::"), Some(("empty", "")));
        assert_eq!(parse_property_line("key::value"), None);
        assert_eq!(parse_property_line("a b:: value"), None);
        assert_eq!(parse_property_line("#tag:: prose"), Some(("#tag", "prose")));
    }

    #[test]
    fn returned_slices_always_borrow_from_input() {
        for line in ["key:: value", "key::", "key:: ", "\tklíč::  value  "] {
            let (key, value) = parse_property_line(line).unwrap();
            let start = line.as_ptr() as usize;
            let end = start + line.len();
            for part in [key, value] {
                let at = part.as_ptr() as usize;
                assert!(at >= start && at + part.len() <= end, "{line:?}");
            }
        }
    }
}
