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
    /// `:start-of-week` — first day of the week in the date picker, 0=Sunday … 6=Saturday.
    pub start_of_week: u32,
    /// `:block-hidden-properties #{:a :b}` — extra property keys to hide from the
    /// rendered properties area, on top of the built-in internal set.
    pub block_hidden_properties: Vec<String>,
    /// `:default-templates {:journals "Name"}` — template applied to a new,
    /// empty journal page.
    pub default_journal_template: Option<String>,
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
            start_of_week: 0,
            block_hidden_properties: Vec::new(),
            default_journal_template: None,
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
        if let Some(n) = int_value(edn, ":start-of-week") {
            if n <= 6 {
                cfg.start_of_week = n;
            }
        }
        cfg.block_hidden_properties = parse_keyword_set(edn, ":block-hidden-properties");
        cfg.default_journal_template = nested_string(edn, ":default-templates", ":journals")
            .filter(|s| !s.is_empty());
        cfg
    }
}

/// Extract a quoted string for `inner_key` inside the map following `outer_key`,
/// e.g. `:default-templates {:journals "Daily"}` -> "Daily".
fn nested_string(edn: &str, outer_key: &str, inner_key: &str) -> Option<String> {
    let i = edn.find(outer_key)? + outer_key.len();
    let after = &edn[i..];
    let open = after.find('{')?;
    let body = &after[open..];
    let close = body.find('}').unwrap_or(body.len());
    let inner = &body[..close];
    let j = inner.find(inner_key)? + inner_key.len();
    let rest = &inner[j..];
    let qs = rest.find('"')? + 1;
    let qe = rest[qs..].find('"')? + qs;
    Some(rest[qs..qe].to_string())
}

/// Extract the integer following `key`, if any.
fn int_value(edn: &str, key: &str) -> Option<u32> {
    let rest = edn[edn.find(key)? + key.len()..].trim_start();
    let digits: String = rest.chars().take_while(|c| c.is_ascii_digit()).collect();
    digits.parse().ok()
}

/// Parse `key #{:a :b}` (an EDN keyword set) into bare keyword strings.
fn parse_keyword_set(edn: &str, key: &str) -> Vec<String> {
    let Some(i) = edn.find(key) else { return Vec::new() };
    let after = &edn[i + key.len()..];
    let Some(open) = after.find('{') else { return Vec::new() };
    let body = &after[open + 1..];
    let Some(close) = body.find('}') else { return Vec::new() };
    body[..close]
        .split_whitespace()
        .filter_map(|t| t.strip_prefix(':'))
        .map(|t| t.to_string())
        .collect()
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
            // Value forms: "binding" | false (disable) | ["b1" "b2"] (first wins).
            if k < inner.len() && bytes[k] == b'"' {
                let vstart = k + 1;
                if let Some(rel) = inner[vstart..].find('"') {
                    map.insert(key.to_string(), inner[vstart..vstart + rel].to_string());
                    i = vstart + rel + 1;
                    continue;
                }
            } else if inner[k..].starts_with("false") {
                map.insert(key.to_string(), "false".to_string());
                i = k + "false".len();
                continue;
            } else if k < inner.len() && bytes[k] == b'[' {
                // Take the first quoted binding inside the vector.
                if let Some(close) = inner[k..].find(']') {
                    let vec_body = &inner[k + 1..k + close];
                    if let Some(qs) = vec_body.find('"') {
                        if let Some(qe) = vec_body[qs + 1..].find('"') {
                            map.insert(key.to_string(), vec_body[qs + 1..qs + 1 + qe].to_string());
                        }
                    }
                    i = k + close + 1;
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

    #[test]
    fn shortcut_false_and_vector_forms() {
        let edn = r#"{:shortcuts {:go/search false
                                  :editor/indent ["tab" "mod+]"]}}"#;
        let cfg = Config::parse(edn);
        assert_eq!(cfg.shortcuts.get("go/search").map(String::as_str), Some("false"));
        assert_eq!(cfg.shortcuts.get("editor/indent").map(String::as_str), Some("tab"));
    }

    #[test]
    fn default_journal_template() {
        let edn = r#"{:default-templates {:journals "Daily"}}"#;
        assert_eq!(Config::parse(edn).default_journal_template.as_deref(), Some("Daily"));
        // Absent → None.
        assert_eq!(Config::parse(r#"{:preferred-format "Markdown"}"#).default_journal_template, None);
    }

    #[test]
    fn start_of_week_and_hidden_properties() {
        let edn = r#"{:start-of-week 1
                      :block-hidden-properties #{:public :icon}}"#;
        let cfg = Config::parse(edn);
        assert_eq!(cfg.start_of_week, 1);
        assert_eq!(cfg.block_hidden_properties, vec!["public".to_string(), "icon".to_string()]);
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
