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

/// Resolve the suffix after the first case-insensitive asset marker. O(path bytes).
pub fn asset_rel_path(path: &str) -> Option<&str> {
    asset_suffix(path, false)
}

fn asset_suffix(path: &str, folder: bool) -> Option<&str> {
    let bytes = path.as_bytes();
    let offset = path
        .match_indices(['a', 'A'])
        .map(|(offset, _)| offset)
        .find(|&offset| {
            (!folder || offset == 0 || matches!(bytes[offset - 1], b'/' | b'\\'))
                && bytes
                    .get(offset..offset + 6)
                    .is_some_and(|part| part.eq_ignore_ascii_case(b"assets"))
                && matches!(bytes.get(offset + 6), Some(b'/' | b'\\'))
        })?;
    Some(&path[offset + 7..])
}

/// Attachment suffix from the authored path. Its folder and separator must be
/// written literally. An exact final `assets` names the root. O(path bytes).
pub fn attachment_path(path: &str) -> Option<&str> {
    let folder = path.trim_end_matches(['/', '\\']);
    if folder.rsplit(['/', '\\']).next()?.eq_ignore_ascii_case("assets") {
        Some("")
    } else {
        asset_suffix(folder, true)
    }
}

/// Decode a local path and carry the authored attachment boundary into the
/// result. The optional offset locates the attachment suffix in the returned
/// path. Split at the written separator so every input byte is decoded once.
/// O(path bytes), no I/O. Plain slash-normalized paths are borrowed.
pub fn decoded_local_path(path: &str, allow_attachment: bool) -> (Cow<'_, str>, Option<usize>) {
    fn normalize(part: &str) -> Cow<'_, str> {
        if part.contains('%') {
            let decoded = percent_decode(part);
            Cow::Owned(match slash_normalized(&decoded) {
                Cow::Owned(normalized) => normalized,
                Cow::Borrowed(_) => decoded,
            })
        } else {
            slash_normalized(part)
        }
    }

    match allow_attachment.then(|| attachment_path(path)).flatten() {
        Some("") => {
            let path = normalize(path);
            let end = path.len();
            (path, Some(end))
        }
        Some(name) => {
            let start = path.trim_end_matches(['/', '\\']).len() - name.len();
            if !path.contains(['%', '\\']) {
                return (Cow::Borrowed(path), Some(start));
            }
            let (prefix, suffix) = path.split_at(start);
            let mut path = normalize(prefix).into_owned();
            let start = path.len();
            path.push_str(&normalize(suffix));
            (Cow::Owned(path), Some(start))
        }
        None => (normalize(path), None),
    }
}

/// Whether a decoded path has a PDF extension, treating its suffix as file bytes.
pub fn is_pdf_path(path: &str) -> bool {
    let bytes = path.as_bytes();
    bytes.len() >= 4 && bytes[bytes.len() - 4..].eq_ignore_ascii_case(b".pdf")
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
    String::from_utf8(out)
        .unwrap_or_else(|error| String::from_utf8_lossy(error.as_bytes()).into_owned())
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
            pdf_asset_file_decoded(&decoded, asset_rel_path(&decoded)),
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

    #[test]
    fn asset_path_carries_only_the_written_attachment_boundary() {
        for (written, resolved, attachment) in [
            ("../%61ssets/photo.png", "../assets/photo.png", None),
            ("assets%2Fphoto.png", "assets/photo.png", None),
            ("../myassets/photo.png", "../myassets/photo.png", None),
            ("../%61ssets/ASSETS/demo%2520.png", "../assets/ASSETS/demo%20.png", Some("demo%20.png")),
            ("../%FF/ASSETS/demo%252F.png", "../�/ASSETS/demo%2F.png", Some("demo%2F.png")),
            ("..\\AsSeTs\\nested%5Cphoto.png", "../AsSeTs/nested/photo.png", Some("nested/photo.png")),
        ] {
            let (path, start) = decoded_local_path(written, true);
            assert_eq!(path, resolved, "{written}");
            assert_eq!(start.map(|start| &path[start..]), attachment, "{written}");
        }
    }
}
