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

/// Decode a page stem in Logseq legacy or triple-lowbar (`legacy = false`) format.
/// Legacy dots become namespace separators before percent decoding.
/// Cost O(stem bytes); malformed percent escapes are preserved.
pub fn decode_page_name(stem: &str, legacy: bool) -> String {
    let encoded = match legacy {
        true => stem.replace('.', "/"),
        false => stem.replace("___", "/"),
    };
    let bytes = encoded.as_bytes();
    let mut output = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'%' && index + 2 < bytes.len() {
            let nibble = |byte| match byte {
                b'0'..=b'9' => Some(byte - b'0'),
                b'a'..=b'f' => Some(byte - b'a' + 10),
                b'A'..=b'F' => Some(byte - b'A' + 10),
                _ => None,
            };
            if let (Some(high), Some(low)) = (nibble(bytes[index + 1]), nibble(bytes[index + 2])) {
                output.push((high << 4) | low);
                index += 3;
                continue;
            }
        }
        output.push(bytes[index]);
        index += 1;
    }
    String::from_utf8_lossy(&output).into_owned()
}
