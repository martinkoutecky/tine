//! PDF resource identity for native persistence and live preview (I-4/I-12).
//! Existing suffix policies are explicit: persistence accepts .pdf/.PDF,
//! preview accepts ASCII case variants. Neither adapter owns sanitization.

#[derive(Clone, Copy)]
pub enum PdfSuffix {
    Native,
    Preview,
}

pub fn asset_key(pdf_filename: &str, suffix: PdfSuffix) -> String {
    let stem = match suffix {
        PdfSuffix::Native => pdf_filename
            .strip_suffix(".pdf")
            .or_else(|| pdf_filename.strip_suffix(".PDF")),
        PdfSuffix::Preview => pdf_filename
            .get(pdf_filename.len().saturating_sub(4)..)
            .filter(|tail| tail.eq_ignore_ascii_case(".pdf"))
            .map(|_| &pdf_filename[..pdf_filename.len() - 4]),
    }
    .unwrap_or(pdf_filename);
    sanitize_filename(stem)
}

/// Strip only the characters the npm `sanitize-filename` 1.6.3 library removes
/// (default empty-string replacement), so the result matches OG's key byte-for-
/// byte: drop the reserved set `/ ? < > \ : * | "`, control chars
/// (`0x00–0x1f`, `0x80–0x9f`), trailing dots/spaces, and Windows reserved device
/// names (CON/PRN/AUX/NUL/COM0-9/LPT0-9). Nothing is lowercased or substituted.
fn sanitize_filename(s: &str) -> String {
    let illegal = |c: char| matches!(c, '/' | '?' | '<' | '>' | '\\' | ':' | '*' | '|' | '"');
    let control = |c: char| {
        let n = c as u32;
        n <= 0x1f || (0x80..=0x9f).contains(&n)
    };
    let mut out: String = s.chars().filter(|&c| !illegal(c) && !control(c)).collect();
    // Windows: strip trailing dots and spaces.
    while out.ends_with('.') || out.ends_with(' ') {
        out.pop();
    }
    // Windows reserved device names (optionally with an extension) → removed.
    let base = out.split('.').next().unwrap_or("").to_ascii_lowercase();
    let reserved = matches!(base.as_str(), "con" | "prn" | "aux" | "nul")
        || ((base.starts_with("com") || base.starts_with("lpt"))
            && base.len() == 4
            && base.as_bytes()[3].is_ascii_digit());
    if reserved {
        out.clear();
    }
    out
}

/// Tine's pre-launch key scheme (lowercased, every non-alphanumeric → `_`).
/// Retained ONLY so highlight files written by older Tine builds can still be
/// located (read-fallback) and migrated forward to the OG-compatible
/// [`asset_key`] on the next write. Do not use for new writes.
pub fn legacy_asset_key(pdf_filename: &str) -> String {
    let stem = pdf_filename.strip_suffix(".pdf").unwrap_or(pdf_filename);
    stem.chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() {
                c.to_ascii_lowercase()
            } else {
                '_'
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn suffix_policies_preserve_existing_resource_names() {
        for filename in [
            "Paper.pdf",
            "Paper.PDF",
            "CON.pdf",
            "a/b:c.pdf",
            "résumé.pdf",
        ] {
            assert_eq!(
                asset_key(filename, PdfSuffix::Native),
                asset_key(filename, PdfSuffix::Preview)
            );
        }
        assert_eq!(asset_key("Paper.PdF", PdfSuffix::Native), "Paper.PdF");
        assert_eq!(asset_key("Paper.PdF", PdfSuffix::Preview), "Paper");
        assert_eq!(legacy_asset_key("Paper-1.pdf"), "paper_1");
    }
}
