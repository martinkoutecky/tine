//! Reversible Logseq page filename codec shared by native graph operations and wasm.
//! No parsing, filesystem, or graph state; each call costs O(input bytes).

/// Encode a page title as a reversible Windows-safe file stem using the
/// Logseq format (`legacy = false` selects triple-lowbar). Cost O(title bytes),
/// no I/O or failure. Existing file paths are never recoded.
pub fn encode_page_name(name: &str, legacy: bool) -> String {
    let trailing_safe = name.trim_end_matches([' ', '.']).len();
    let mut escaped = String::with_capacity(name.len());
    for (offset, character) in name.char_indices() {
        let encode = character == '%'
            || character <= '\u{1f}'
            || character == '\u{7f}'
            || matches!(
                character,
                '<' | '>' | ':' | '"' | '\\' | '|' | '?' | '*' | '#'
            )
            || (character == '.' && (legacy || offset == 0 || offset >= trailing_safe))
            || (character == ' ' && offset >= trailing_safe);
        if encode {
            let mut bytes = [0_u8; 4];
            for byte in character.encode_utf8(&mut bytes).as_bytes() {
                push_percent_byte(&mut escaped, *byte);
            }
        } else {
            escaped.push(character);
        }
    }
    let mut encoded = match legacy {
        true => escaped.replace('/', "%2F"),
        false => escaped
            .replace("___", "%5F%5F%5F")
            .replace("_/", "%5F/")
            .replace("/_", "/%5F")
            .replace('/', "___"),
    };
    let device = encoded
        .split('.')
        .next()
        .unwrap_or("")
        .trim_end_matches(' ')
        .to_uppercase();
    let reserved = matches!(device.as_str(), "CON" | "PRN" | "AUX" | "NUL")
        || ["COM", "LPT"].iter().any(|prefix| {
            device.strip_prefix(prefix).is_some_and(|suffix| {
                matches!(
                    suffix,
                    "1" | "2" | "3" | "4" | "5" | "6" | "7" | "8" | "9" | "¹" | "²" | "³"
                )
            })
        });
    if reserved {
        let first_len = encoded.chars().next().map(char::len_utf8).unwrap_or(0);
        let mut safe = String::with_capacity(encoded.len() + 2);
        for byte in &encoded.as_bytes()[..first_len] {
            push_percent_byte(&mut safe, *byte);
        }
        safe.push_str(&encoded[first_len..]);
        encoded = safe;
    }
    encoded
}

fn push_percent_byte(output: &mut String, byte: u8) {
    const HEX: &[u8; 16] = b"0123456789ABCDEF";
    output.push('%');
    output.push(char::from(HEX[usize::from(byte >> 4)]));
    output.push(char::from(HEX[usize::from(byte & 0x0f)]));
}

/// Decode a page stem in Logseq legacy or triple-lowbar (`legacy = false`)
/// format: the ONE file-name decoder (every caller, wasm included, calls it).
/// Cost O(stem bytes), no failure. OG 6e7afa8eb
/// `deps/graph-parser/src/logseq/graph_parser/util.cljs` (D-20261103-CMA):
///
/// - Legacy (`legacy-title-parsing`): dots become `/`, then ONE
///   `decodeURIComponent` over the whole stem; a malformed escape (a `%` not
///   followed by two hex digits, or bytes that are not UTF-8) keeps the whole
///   stem undecoded (`50%25 of 100%` stays as written).
/// - Triple-lowbar (`tri-lb-title-parsing`): `___` becomes `/`, each `%XX`
///   escape decodes, and `make-valid-namespaces` drops empty namespace parts
///   (`___lead` is `lead`, `a______b` is `a/b`). OG decodes each escape on its
///   own, so a lone byte of a multi-byte character stays as written (`%E9`).
///   Deliberate difference: escapes that together spell valid UTF-8 decode
///   (`caf%C3%A9` is "café", OG "caf%C3%A9"); only bytes that are not UTF-8
///   keep their escape text. A stem with no non-empty part (`______`) keeps
///   its empty parts rather than naming a page "" as OG does.
pub fn decode_page_name(stem: &str, legacy: bool) -> String {
    if legacy {
        let slashed = stem.replace('.', "/");
        return decode_uri_component(&slashed).unwrap_or(slashed);
    }
    let decoded = decode_escapes_keeping_malformed(&stem.replace("___", "/"));
    if !(decoded.starts_with('/') || decoded.ends_with('/') || decoded.contains("//")) {
        return decoded;
    }
    let valid = decoded
        .split('/')
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join("/");
    if valid.is_empty() {
        decoded
    } else {
        valid
    }
}

fn hex_escape(bytes: &[u8], index: usize) -> Option<u8> {
    let nibble = |byte: u8| match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    };
    if bytes.get(index) != Some(&b'%') {
        return None;
    }
    Some((nibble(*bytes.get(index + 1)?)? << 4) | nibble(*bytes.get(index + 2)?)?)
}

/// JavaScript's `decodeURIComponent`: `None` when a `%` starts no escape or
/// the decoded bytes are not UTF-8.
fn decode_uri_component(text: &str) -> Option<String> {
    let bytes = text.as_bytes();
    let mut output = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'%' {
            output.push(hex_escape(bytes, index)?);
            index += 3;
        } else {
            output.push(bytes[index]);
            index += 1;
        }
    }
    String::from_utf8(output).ok()
}

/// Each run of `%XX` escapes decodes where its bytes are UTF-8; a byte that
/// is not keeps its escape text as written. Other text is copied. A run holds
/// only escaped bytes, and an unescaped character never continues an escaped
/// lead byte (its first byte is not a continuation byte), so a run decodes on
/// its own.
fn decode_escapes_keeping_malformed(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut output = String::with_capacity(text.len());
    let mut index = 0;
    while index < bytes.len() {
        let start = index;
        if bytes[index] != b'%' {
            // Copy the text up to the next `%` at once.
            let next = text[index..].find('%').map_or(text.len(), |at| index + at);
            output.push_str(&text[index..next]);
            index = next;
            continue;
        }
        let mut run = Vec::new();
        while let Some(byte) = hex_escape(bytes, index) {
            run.push(byte);
            index += 3;
        }
        if run.is_empty() {
            let len = text[index..].chars().next().map_or(1, char::len_utf8);
            output.push_str(&text[index..index + len]);
            index += len;
            continue;
        }
        let mut at = start;
        for chunk in run.utf8_chunks() {
            output.push_str(chunk.valid());
            at += 3 * chunk.valid().len();
            let invalid = 3 * chunk.invalid().len();
            output.push_str(&text[at..at + invalid]);
            at += invalid;
        }
    }
    output
}

#[cfg(test)]
mod tests {
    use super::{decode_page_name, encode_page_name};

    #[test]
    fn decodes_as_og_with_utf8_escapes_kept_decoded() {
        for (stem, triple, legacy) in [
            ("%E9", "%E9", "%E9"),
            ("%e9", "%e9", "%e9"),
            ("bad%E9", "bad%E9", "bad%E9"),
            ("caf%C3%A9", "café", "café"),
            ("caf%C3%A9%E9", "café%E9", "caf%C3%A9%E9"),
            ("%C3", "%C3", "%C3"),
            ("%C3%28", "%C3(", "%C3%28"),
            ("50%25 of 100%", "50% of 100%", "50%25 of 100%"),
            ("A%3AB%ZZ", "A:B%ZZ", "A%3AB%ZZ"),
            ("a%2Fb", "a/b", "a/b"),
            ("___lead", "lead", "___lead"),
            ("a___", "a", "a___"),
            ("a______b", "a/b", "a______b"),
            ("x%2F%2Fy", "x/y", "x//y"),
            ("______", "//", "______"),
            ("a.b", "a.b", "a/b"),
            (".lead", ".lead", "/lead"),
            ("字%E5%AD%97", "字字", "字字"),
        ] {
            assert_eq!(
                decode_page_name(stem, false),
                triple,
                "triple-lowbar {stem}"
            );
            assert_eq!(decode_page_name(stem, true), legacy, "legacy {stem}");
        }
    }

    #[test]
    fn a_name_with_a_percent_round_trips() {
        for name in ["%E9", "bad%E9", "50%25 of 100%", "100%", "%", "lead"] {
            for legacy in [true, false] {
                assert_eq!(
                    decode_page_name(&encode_page_name(name, legacy), legacy),
                    name
                );
            }
        }
    }
}
