//! Minimal reader for `logseq/config.edn`. We only extract the handful of keys
//! that affect file layout/parsing. This is intentionally a light string scan,
//! not a full EDN parser (swap in `edn-rs` later if needed).

#[derive(Debug, Clone)]
pub struct Config {
    pub journals_dir: String,
    pub pages_dir: String,
    pub preferred_workflow: Workflow,
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
        }
    }
}

impl Config {
    pub fn parse(edn: &str) -> Config {
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
        cfg
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
