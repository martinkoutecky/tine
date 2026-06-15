//! Minimal reader for `logseq/config.edn`. We only extract the handful of keys
//! that affect file layout/parsing. This is intentionally a light string scan,
//! not a full EDN parser (swap in `edn-rs` later if needed).

use std::collections::HashMap;

#[derive(Debug, Clone)]
pub struct Config {
    pub journals_dir: String,
    pub pages_dir: String,
    pub preferred_workflow: Workflow,
    /// User keybinding overrides from `:shortcuts {:cmd "binding"}` (string
    /// bindings only; vectors/`false` are ignored for now).
    pub shortcuts: HashMap<String, String>,
    /// `:publishing/all-pages-public?` — when true, HTML export publishes every
    /// page; otherwise only pages with `public:: true`.
    pub all_pages_public: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Workflow {
    /// NOW / LATER
    Now,
    /// TODO / DOING
    Todo,
}

impl Default for Config {
    fn default() -> Self {
        Config {
            journals_dir: "journals".into(),
            pages_dir: "pages".into(),
            preferred_workflow: Workflow::Now,
            shortcuts: HashMap::new(),
            all_pages_public: false,
        }
    }
}

impl Config {
    pub fn parse(edn: &str) -> Config {
        // Strip line comments (`;` to end of line) so commented-out example
        // keys like `;; :journals-directory "foo"` aren't mistaken for real
        // settings.
        let edn: String = edn
            .lines()
            .map(|l| match l.find(';') {
                Some(i) => &l[..i],
                None => l,
            })
            .collect::<Vec<_>>()
            .join("\n");
        let edn = edn.as_str();

        let mut cfg = Config::default();
        if let Some(v) = string_value(edn, ":journals-directory") {
            cfg.journals_dir = v;
        }
        if let Some(v) = string_value(edn, ":pages-directory") {
            cfg.pages_dir = v;
        }
        if let Some(v) = keyword_value(edn, ":preferred-workflow") {
            cfg.preferred_workflow = if v == "todo" { Workflow::Todo } else { Workflow::Now };
        }
        cfg.shortcuts = parse_shortcuts(edn);
        if let Some(i) = edn.find(":publishing/all-pages-public?") {
            let after = edn[i + ":publishing/all-pages-public?".len()..].trim_start();
            cfg.all_pages_public = after.starts_with("true");
        }
        cfg
    }
}

/// Extract the `:shortcuts {...}` map as command-id -> binding (string values).
fn parse_shortcuts(edn: &str) -> HashMap<String, String> {
    let mut map = HashMap::new();
    let Some(i) = edn.find(":shortcuts") else { return map };
    let after = &edn[i + ":shortcuts".len()..];
    let Some(open) = after.find('{') else { return map };
    let body = &after[open + 1..];
    // Shortcut maps contain only string/keyword values (no nested maps), so the
    // first '}' closes the section.
    let Some(close) = body.find('}') else { return map };
    let inner = &body[..close];

    let bytes = inner.as_bytes();
    let mut i = 0;
    while i < inner.len() {
        if bytes[i] == b':' {
            let start = i + 1;
            let mut j = start;
            while j < inner.len() && !bytes[j].is_ascii_whitespace() {
                j += 1;
            }
            let key = &inner[start..j];
            let mut k = j;
            while k < inner.len() && bytes[k].is_ascii_whitespace() {
                k += 1;
            }
            if k < inner.len() && bytes[k] == b'"' {
                let vstart = k + 1;
                if let Some(rel) = inner[vstart..].find('"') {
                    map.insert(key.to_string(), inner[vstart..vstart + rel].to_string());
                    i = vstart + rel + 1;
                    continue;
                }
            }
            i = k.max(j);
        } else {
            i += 1;
        }
    }
    map
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_shortcuts_map() {
        let edn = r#"{:preferred-format "Markdown"
                      :shortcuts {:go/search "mod+shift+k"
                                  :ui/toggle-theme "t d"}}"#;
        let cfg = Config::parse(edn);
        assert_eq!(cfg.shortcuts.get("go/search").map(String::as_str), Some("mod+shift+k"));
        assert_eq!(cfg.shortcuts.get("ui/toggle-theme").map(String::as_str), Some("t d"));
    }

    #[test]
    fn no_shortcuts_section_is_empty() {
        let cfg = Config::parse(r#"{:preferred-format "Markdown"}"#);
        assert!(cfg.shortcuts.is_empty());
    }
}

/// Extract the string after `key` ignoring comments crudely: find `key`, then
/// the next double-quoted token.
fn string_value(edn: &str, key: &str) -> Option<String> {
    let idx = edn.find(key)? + key.len();
    let rest = &edn[idx..];
    let start = rest.find('"')? + 1;
    let end = rest[start..].find('"')? + start;
    Some(rest[start..end].to_string())
}

/// Extract the keyword (`:foo` -> `foo`) following `key`.
fn keyword_value(edn: &str, key: &str) -> Option<String> {
    let idx = edn.find(key)? + key.len();
    let rest = edn[idx..].trim_start();
    let rest = rest.strip_prefix(':')?;
    let end = rest
        .find(|c: char| c.is_whitespace() || c == '}' || c == ')')
        .unwrap_or(rest.len());
    Some(rest[..end].to_string())
}

#[cfg(test)]
mod commented_dirs_test {
    use super::*;
    #[test]
    fn commented_example_dirs_fall_back_to_defaults() {
        let edn = r#"{
 ;; :preferred-format ""
 ;; if not specified, notes are stored in `pages` directory
 ;; :pages-directory "your-directory"
 ;; if not specified, journals are stored in `journals` directory
 ;; :journals-directory "your-directory"
}"#;
        let cfg = Config::parse(edn);
        assert_eq!(cfg.journals_dir, "journals");
        assert_eq!(cfg.pages_dir, "pages");
    }
}
