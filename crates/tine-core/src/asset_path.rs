//! Pure URL-to-asset identity shared by the PDF reader and highlights writer.
//! No filesystem access; the asset backend validates names before opening them.

/// Resolve a graph asset relative to `assets/`, normalizing separators and case
/// of the directory name. None when no asset directory occurs. O(URL bytes).
pub fn asset_rel_path(url: &str) -> Option<String> {
    let normalized = url.replace('\\', "/");
    let i = normalized.to_ascii_lowercase().find("assets/")?;
    Some(normalized[i + "assets/".len()..].to_owned())
}

/// PDF identity is its path under `assets/` (case-insensitive directory name),
/// or the basename for paths outside that directory. Decode URL escapes,
/// normalize separators and omit current-directory components. O(path bytes).
pub fn pdf_asset_file(path: &str) -> String {
    let path = percent_decode(path);
    if let Some(rel) = asset_rel_path(&path).filter(|rel| !rel.is_empty()) {
        rel.split('/')
            .filter(|part| *part != ".")
            .collect::<Vec<_>>()
            .join("/")
    } else {
        path.rsplit(['/', '\\'])
            .next()
            .filter(|part| !part.is_empty())
            .unwrap_or(&path)
            .to_owned()
    }
}

/// Generate a PDF pointer that resolves back to the literal asset name. O(path bytes).
pub(crate) fn pdf_asset_pointer(filename: &str) -> String {
    format!("../assets/{}", filename.replace('%', "%25"))
}

/// Decode `%XX` percent-escapes (UTF-8 aware, like JS `decodeURIComponent`). An
/// invalid or truncated escape is left literal rather than dropped.
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
    String::from_utf8_lossy(&out).into_owned()
}

fn hex_nibble(c: u8) -> Option<u8> {
    match c {
        b'0'..=b'9' => Some(c - b'0'),
        b'a'..=b'f' => Some(c - b'a' + 10),
        b'A'..=b'F' => Some(c - b'A' + 10),
        _ => None,
    }
}
