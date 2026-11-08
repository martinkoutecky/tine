//! The page-properties form's text edit, shared by the window and owner verbs.
//! Parser-owned entries only; unrelated lines and preamble prose retain bytes.
use crate::block_regions::{page_header, parse_document};

fn entries(raw: &str, org: bool) -> Result<Vec<(usize, String)>, String> {
    let regions = parse_document(raw, org);
    if regions.quarantined {
        return Err("page properties could not be parsed".into());
    }
    Ok(if org {
        regions
            .page_properties()
            .map(|p| (raw[..p.line.0].matches('\n').count(), p.key.to_lowercase()))
            .collect()
    } else {
        page_header(raw)
            .entries
            .into_iter()
            .map(|p| (p.line, p.key))
            .collect()
    })
}

/// Replace the first match, remove duplicate matches, or prepend a new property
/// to the first source. Blank values remove. Same policy as the page form.
/// O(total source bytes); no I/O or persisted format of its own.
pub fn set(
    parts: &[String],
    org: bool,
    key: &str,
    value: Option<&str>,
) -> Result<Vec<String>, String> {
    if parts.is_empty() {
        return Ok(vec![]);
    }
    let value = value.map(str::trim).filter(|v| !v.is_empty());
    if key.is_empty()
        || key.contains(['\n', '\r', ':'])
        || value.is_some_and(|v| v.contains(['\n', '\r']))
    {
        return Err("Invalid page property edit".into());
    }
    let lower = key.to_lowercase();
    let mut found = false;
    let mut out = Vec::new();
    for raw in parts {
        let properties = entries(raw, org)?;
        let mut lines: Vec<String> = if raw.is_empty() {
            vec![]
        } else {
            raw.split('\n').map(str::to_owned).collect()
        };
        let mut dropped = std::collections::HashSet::new();
        for (line, spelling) in &properties {
            if spelling.to_lowercase() != lower {
                continue;
            }
            if found || value.is_none() {
                dropped.insert(*line);
            } else {
                let old = &lines[*line];
                let indent = &old[..old.len() - old.trim_start().len()];
                lines[*line] = if org {
                    format!(
                        "{indent}{}{lower}: {}",
                        if old.trim_start().starts_with("#+") {
                            "#+"
                        } else {
                            ":"
                        },
                        value.unwrap()
                    )
                } else {
                    format!("{spelling}:: {}", value.unwrap())
                };
            }
            found = true;
        }
        let mut kept: Vec<_> = lines
            .into_iter()
            .enumerate()
            .filter(|(i, _)| !dropped.contains(i))
            .collect();
        if org {
            let mut empty_drawers = std::collections::HashSet::new();
            for (open, close) in parse_document(raw, true).property_drawer_boundaries(raw) {
                let start = raw[..open].matches('\n').count();
                let end = raw[..close].matches('\n').count();
                if kept
                    .windows(2)
                    .any(|pair| pair[0].0 == start && pair[1].0 == end)
                    && dropped.iter().any(|d| start < *d && *d < end)
                {
                    empty_drawers.extend([start, end]);
                }
            }
            kept.retain(|(i, _)| !empty_drawers.contains(i));
        } else if dropped.contains(&0) {
            if let Some(first) = kept.iter().position(|(_, line)| !line.trim().is_empty()) {
                if properties.iter().any(|(i, _)| *i == kept[first].0) {
                    kept.drain(..first);
                }
            }
        }
        out.push(
            kept.into_iter()
                .map(|(_, line)| line)
                .collect::<Vec<_>>()
                .join("\n"),
        );
    }
    if !found {
        if let Some(value) = value {
            let line = if org {
                format!("#+{lower}: {value}")
            } else {
                format!("{key}:: {value}")
            };
            // Validate the generated line with the same reader, before touching text.
            if !entries(&line, org)?
                .iter()
                .any(|(_, k)| k.to_lowercase() == lower)
            {
                return Err("Invalid page property key".into());
            }
            out[0] = if out[0].is_empty() {
                line
            } else {
                format!("{line}\n{}", out[0])
            };
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn form_policy_keeps_prose_and_removes_duplicate_and_empty_drawer() {
        assert_eq!(
            set(
                &["Tags:: old\n\ntags:: duplicate\n\nprose".into()],
                false,
                "tags",
                Some("new")
            )
            .unwrap(),
            ["Tags:: new\n\n\nprose"]
        );
        assert_eq!(
            set(
                &["#+ALIAS: one\n:PROPERTIES:\n:alias: two\n:END:\nprose".into()],
                true,
                "alias",
                None
            )
            .unwrap(),
            ["prose"]
        );
    }
}
