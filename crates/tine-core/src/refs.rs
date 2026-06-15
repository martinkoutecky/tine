//! Reference extraction from block text: `[[page]]`, `#tag` (and `#[[multi
//! word]]`), and `((block-uuid))`. Used for the backlink index and queries.
//! UTF-8 safe (advances by char boundaries).

/// Normalize a page name for indexing/matching: trim + lowercase. Display uses
/// the original.
pub fn normalize(name: &str) -> String {
    name.trim().to_lowercase()
}

fn is_tag_char(c: char) -> bool {
    c.is_alphanumeric() || matches!(c, '-' | '_' | '/' | '.')
}

/// All page references in `raw`: `[[name]]`, `#tag`, and `#[[name]]`. Tags are
/// included because Logseq treats `#foo` as a reference to page `foo`.
pub fn page_refs(raw: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut i = 0;
    while i < raw.len() {
        let rest = &raw[i..];
        if let Some(after) = rest.strip_prefix("[[") {
            if let Some(end) = after.find("]]") {
                out.push(after[..end].to_string());
                i += 2 + end + 2;
                continue;
            }
        }
        if rest.starts_with('#') {
            if let Some(after) = rest.strip_prefix("#[[") {
                if let Some(end) = after.find("]]") {
                    out.push(after[..end].to_string());
                    i += 3 + end + 2;
                    continue;
                }
            }
            let after = &rest[1..];
            let len = after.find(|c: char| !is_tag_char(c)).unwrap_or(after.len());
            if len > 0 {
                out.push(after[..len].to_string());
                i += 1 + len;
                continue;
            }
        }
        i += rest.chars().next().map(|c| c.len_utf8()).unwrap_or(1);
    }
    out
}

/// All block references `((uuid))` in `raw`.
pub fn block_refs(raw: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut i = 0;
    while i < raw.len() {
        let rest = &raw[i..];
        if let Some(after) = rest.strip_prefix("((") {
            if let Some(end) = after.find("))") {
                out.push(after[..end].trim().to_string());
                i += 2 + end + 2;
                continue;
            }
        }
        i += rest.chars().next().map(|c| c.len_utf8()).unwrap_or(1);
    }
    out
}

/// Does `raw` reference page `target` (by `[[...]]` or `#tag`)?
pub fn references_page(raw: &str, target: &str) -> bool {
    let t = normalize(target);
    page_refs(raw).iter().any(|r| normalize(r) == t)
}

/// Rewrite every reference to page `from` (case-insensitive) as `to`, returning
/// the new text. Handles `[[from]]`, `#from`, and `#[[from]]`. A `#tag` becomes
/// `#[[to]]` when `to` contains characters that aren't valid in a bare tag
/// (e.g. spaces), matching Logseq.
pub fn rename_refs(raw: &str, from: &str, to: &str) -> String {
    let target = normalize(from);
    let mut out = String::with_capacity(raw.len());
    let mut i = 0;
    while i < raw.len() {
        let rest = &raw[i..];
        if let Some(after) = rest.strip_prefix("[[") {
            if let Some(end) = after.find("]]") {
                if normalize(&after[..end]) == target {
                    out.push_str(&format!("[[{to}]]"));
                } else {
                    out.push_str(&raw[i..i + 2 + end + 2]);
                }
                i += 2 + end + 2;
                continue;
            }
        }
        if let Some(after) = rest.strip_prefix("#[[") {
            if let Some(end) = after.find("]]") {
                if normalize(&after[..end]) == target {
                    out.push_str(&tag_for(to));
                } else {
                    out.push_str(&raw[i..i + 3 + end + 2]);
                }
                i += 3 + end + 2;
                continue;
            }
        }
        if rest.starts_with('#') {
            let after = &rest[1..];
            let len = after.find(|c: char| !is_tag_char(c)).unwrap_or(after.len());
            if len > 0 {
                if normalize(&after[..len]) == target {
                    out.push_str(&tag_for(to));
                } else {
                    out.push_str(&raw[i..i + 1 + len]);
                }
                i += 1 + len;
                continue;
            }
        }
        let ch = rest.chars().next().unwrap();
        out.push(ch);
        i += ch.len_utf8();
    }
    out
}

/// `#to` if `to` is a bare-tag-safe name, else `#[[to]]`.
fn tag_for(to: &str) -> String {
    if to.chars().all(is_tag_char) && !to.is_empty() {
        format!("#{to}")
    } else {
        format!("#[[{to}]]")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_page_refs_and_tags() {
        let raw = "see [[Foo Bar]] and #tag and #[[multi word]] end";
        assert_eq!(page_refs(raw), vec!["Foo Bar", "tag", "multi word"]);
    }

    #[test]
    fn extracts_block_refs() {
        assert_eq!(
            block_refs("ref ((628953c1-8d75-49fe-a648-f4c612109098)) here"),
            vec!["628953c1-8d75-49fe-a648-f4c612109098"]
        );
    }

    #[test]
    fn references_page_is_case_insensitive() {
        assert!(references_page("link to [[Logseq]]", "logseq"));
        assert!(references_page("a #project tag", "Project"));
        assert!(!references_page("no refs here", "logseq"));
    }

    #[test]
    fn utf8_safe() {
        let raw = "café [[naïve]] θ #tag";
        assert_eq!(page_refs(raw), vec!["naïve", "tag"]);
    }
}
