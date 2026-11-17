//! The page-properties form's text edit, shared by the window and owner verbs.
//! Parser-owned entries only; unrelated lines and preamble prose retain bytes.
use crate::block_regions::parse_document;

fn entries(raw: &str, org: bool) -> Result<Vec<(usize, String)>, String> {
    let regions = parse_document(raw, org);
    if regions.quarantined {
        return Err("page properties could not be parsed".into());
    }
    Ok(regions
        .page_properties()
        .map(|p| (raw[..p.line.0].matches('\n').count(), p.key.clone()))
        .collect())
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
            // A Markdown header may follow blank lines; the new line joins
            // it there, since a blank line below it would end the header.
            let headed = !org && !entries(&out[0], org)?.is_empty();
            let head: usize = out[0]
                .split_inclusive('\n')
                .take_while(|l| headed && l.ends_with('\n') && l.trim().is_empty())
                .map(str::len)
                .sum();
            let (blank, rest) = out[0].split_at(head);
            out[0] = if rest.is_empty() {
                format!("{blank}{line}")
            } else {
                format!("{blank}{line}\n{rest}")
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
                &["Tags:: old\ntags:: duplicate\n\nprose".into()],
                false,
                "tags",
                Some("new")
            )
            .unwrap(),
            ["Tags:: new\n\nprose"]
        );
        // A property node after a blank line is prose, not a page property
        // (OG reads only the first node): it keeps its bytes.
        assert_eq!(
            set(
                &["Tags:: old\n\ntags:: duplicate\n\nprose".into()],
                false,
                "tags",
                Some("new")
            )
            .unwrap(),
            ["Tags:: new\n\ntags:: duplicate\n\nprose"]
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

    /// The page properties and the prose `raw` reads as.
    fn reading(raw: &str) -> (Vec<String>, Option<&str>) {
        let header = crate::page_preamble::page_header(raw);
        let prose = crate::page_preamble::preamble_content_range(raw, false);
        (
            header.entries.into_iter().map(|e| e.key).collect(),
            prose.map(|r| &raw[r.0..r.1]),
        )
    }

    /// Review F2: editing one key keeps every other line's role and bytes.
    #[test]
    fn an_edit_keeps_the_other_lines_reading() {
        // Opening text before a property line: no page property, all prose. A
        // new key goes on top; the alias line stays prose, never a declaration.
        let before = "Intro [[X]]\nalias:: y";
        assert_eq!(reading(before), (vec![], Some(before)));
        let out = set(&[before.into()], false, "tags", Some("new")).unwrap();
        assert_eq!(out, ["tags:: new\nIntro [[X]]\nalias:: y"]);
        assert_eq!(reading(&out[0]), (vec!["tags".into()], Some(before)));
        // Removing the only page property leaves a later node first, so it
        // becomes the page's properties. OG does the same: its removal
        // (`batch-set-block-property!` → `property/remove-properties`, which
        // drops the leading `key:: value` lines of the pre-block's content)
        // saves `\nalias:: Surprise` (left-trimmed by `wrap-parse-block`), and
        // mldoc 1.5.9 reads `alias` from that text. Tine keeps the newline.
        let out = set(&["a:: 1\n\nalias:: Surprise".into()], false, "a", None).unwrap();
        assert_eq!(out, ["\nalias:: Surprise"]);
        assert_eq!(reading(&out[0]), (vec!["alias".into()], None));
        // A header after blank lines stays one header when a key is added.
        for (before, after) in [
            (
                "\ntitle:: Existing\nalias:: y",
                "\ntags:: new\ntitle:: Existing\nalias:: y",
            ),
            (
                "\r\n \r\ntitle:: Existing\r\nalias:: y",
                "\r\n \r\ntags:: new\ntitle:: Existing\r\nalias:: y",
            ),
        ] {
            assert_eq!(reading(before).0, ["title", "alias"]);
            let out = set(&[before.into()], false, "tags", Some("new")).unwrap();
            assert_eq!(out, [after]);
            assert_eq!(reading(after).0, ["tags", "title", "alias"]);
        }
        let out = set(&["\n\nprose".into()], false, "tags", Some("new")).unwrap();
        assert_eq!(out, ["tags:: new\n\n\nprose"]);
        assert_eq!(reading(&out[0]), (vec!["tags".into()], Some("prose")));
    }
}
