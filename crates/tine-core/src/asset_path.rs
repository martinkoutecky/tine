//! Pure URL-to-asset identity shared by the PDF reader and highlights writer.
//! No filesystem access; the asset backend validates names before opening them.

use std::borrow::Cow;

/// Normalize local path separators, borrowing when already normalized. O(path bytes).
pub fn slash_normalized(path: &str) -> Cow<'_, str> {
    if path.contains('\\') {
        Cow::Owned(path.replace('\\', "/"))
    } else {
        Cow::Borrowed(path)
    }
}

/// Resolve the suffix after the first case-insensitive `assets/` marker.
/// The caller supplies decoded path bytes with forward slashes. O(path bytes).
pub fn asset_rel_path(path: &str) -> Option<&str> {
    let marker = b"assets/";
    let offset = path
        .match_indices(['a', 'A'])
        .map(|(offset, _)| offset)
        .find(|&offset| {
            path.as_bytes()
                .get(offset..offset + marker.len())
                .is_some_and(|part| part.eq_ignore_ascii_case(marker))
        })?;
    Some(&path[offset + marker.len()..])
}

/// Attachment target from decoded slash-normalized path bytes. An exact
/// final `assets` directory names the root. O(path bytes).
pub fn attachment_path(path: &str) -> Option<&str> {
    let folder = path.trim_end_matches('/');
    if folder.rsplit('/').next()?.eq_ignore_ascii_case("assets") {
        Some("")
    } else {
        asset_rel_path(folder)
    }
}

/// PDF identity is its path under `assets/` (case-insensitive directory name),
/// or the basename for paths outside that directory. Decode URL escapes,
/// normalize separators and omit current-directory components. O(path bytes).
pub fn pdf_asset_file(path: &str) -> String {
    let path = percent_decode(path);
    let normalized = slash_normalized(&path);
    pdf_asset_file_decoded(&path, asset_rel_path(&normalized))
}

/// PDF identity from a decoded path and its resolved asset suffix. O(path bytes).
pub fn pdf_asset_file_decoded(path: &str, asset: Option<&str>) -> String {
    if let Some(rel) = asset.filter(|rel| !rel.is_empty()) {
        rel.split('/')
            .filter(|part| *part != ".")
            .collect::<Vec<_>>()
            .join("/")
    } else {
        path.rsplit(['/', '\\'])
            .next()
            .filter(|part| !part.is_empty())
            .unwrap_or(path)
            .to_owned()
    }
}

/// Generate a PDF pointer that resolves back to the literal asset name. O(path bytes).
pub(crate) fn pdf_asset_pointer(filename: &str) -> String {
    format!("../assets/{}", filename.replace('%', "%25"))
}

/// Decode `%XX` once. Malformed or truncated escapes stay literal. Invalid
/// UTF-8 bytes use the Unicode replacement character. O(input bytes).
pub fn percent_decode(s: &str) -> String {
    if !s.contains('%') {
        return s.to_string();
    }
    let b = s.as_bytes();
    let mut out: Vec<u8> = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%' && i + 2 < b.len() {
            if let (Some(h), Some(l)) = (hex_nibble(b[i + 1]), hex_nibble(b[i + 2])) {
                out.push((h << 4) | l);
                i += 3;
                continue;
            }
        }
        out.push(b[i]);
        i += 1;
    }
    String::from_utf8(out).unwrap_or_else(|error| String::from_utf8_lossy(error.as_bytes()).into_owned())
}

fn hex_nibble(c: u8) -> Option<u8> {
    match c {
        b'0'..=b'9' => Some(c - b'0'),
        b'a'..=b'f' => Some(c - b'a' + 10),
        b'A'..=b'F' => Some(c - b'A' + 10),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn asset_path_decodes_once_and_keeps_broken_escapes() {
        let decoded = percent_decode("../%41ssets/./文%20%252F%ZZ%2%FF%.PDF");
        assert_eq!(decoded, "../Assets/./文 %2F%ZZ%2�%.PDF");
        assert_eq!(asset_rel_path(&decoded), Some("./文 %2F%ZZ%2�%.PDF"));
        assert_eq!(
            pdf_asset_file_decoded(&decoded, attachment_path(&decoded)),
            "文 %2F%ZZ%2�%.PDF"
        );
        assert_eq!(attachment_path("docs.v1/myassets/"), None);
        assert_eq!(attachment_path("docs.v1/ASSETS//"), Some(""));
        assert_eq!(
            pdf_asset_file("..\\%41ssets\\.\\文%20%252F%ZZ%2%FF%.PDF"),
            "文 %2F%ZZ%2�%.PDF"
        );
        assert_eq!(pdf_asset_file(r"docs\"), r"docs\");
    }
}
