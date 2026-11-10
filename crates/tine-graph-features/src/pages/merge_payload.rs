//! The merge survivor's header and moved blocks: pure text over parsed
//! pages, split from `pages.rs` along the plan/commit seam (no store, no I/O).

use std::collections::HashSet;

use tine_core::model::{Format, PageDto};
use tine_core::refs;

use super::MergeNames;

/// Rewrite the merge payload while carrying the logical names of applied
/// edits. Both the survivor and source use this same block/pre-block writer.
pub(super) fn rename_doc(
    doc: &mut PageDto,
    names: MergeNames<'_>,
    org: bool,
    format: tine_core::config::FileNameFormat,
) {
    fn walk(
        blocks: &mut [tine_core::model::BlockDto],
        names: MergeNames<'_>,
        org: bool,
        format: tine_core::config::FileNameFormat,
    ) {
        for block in blocks {
            block.raw = refs::rename_block_raw(
                &block.raw,
                org,
                names.links,
                Some(names.queries),
                format,
                Some(&mut names.rewritten.borrow_mut()),
            );
            walk(&mut block.children, names, org, format);
        }
    }
    if let Some(pre) = doc.pre_block.as_mut() {
        *pre = refs::rename_pre_block(
            pre,
            org,
            names.links,
            Some(names.queries),
            format,
            Some(&mut names.rewritten.borrow_mut()),
        );
    }
    walk(&mut doc.blocks, names, org, format);
}

fn lines<'a>(mut parts: impl Iterator<Item = &'a str>) -> String {
    let first = parts.next().unwrap_or_default().to_owned();
    parts.fold(first, |text, line| text + "\n" + line)
}

/// Each line of a page preamble, with its lowercase key and value when the
/// parser accepts it as a page-header property (Markdown `key:: value`, Org
/// `#+KEY: value`) outside every literal container. A fenced or `#+BEGIN_…`
/// example is literal text, never a header property (I-12: the parser owns
/// regions; REG-OG-C5-L03-S2). One parse of `text`, O(text bytes).
fn header_lines(text: &str, org: bool) -> Vec<(&str, Option<(String, String)>)> {
    let regions = crate::conflicts::pre_regions(text, if org { Format::Org } else { Format::Md });
    let mut at = 0;
    text.split_inclusive('\n')
        .map(|raw| {
            let start = at;
            at += raw.len();
            let line = raw.strip_suffix('\n').unwrap_or(raw);
            let line = line.strip_suffix('\r').unwrap_or(line);
            let property = regions
                .container_of(start, at)
                .is_none()
                .then(|| regions.property(start).cloned())
                .flatten();
            (line, property)
        })
        .collect()
}

#[cfg(test)]
mod header_property_tests {
    use super::header_lines;

    fn property(text: &str, org: bool) -> Option<(String, String)> {
        header_lines(text, org)
            .into_iter()
            .next()
            .and_then(|(_, p)| p)
    }

    #[test]
    fn merge_recognizes_unicode_page_headers_in_both_formats() {
        assert_eq!(
            property("klíč:: hodnota", false),
            Some(("klíč".into(), "hodnota".into()))
        );
        assert_eq!(
            property("#+klíč: hodnota", true),
            Some(("klíč".into(), "hodnota".into()))
        );
        assert_eq!(
            property("#+a.b/c: value", true),
            Some(("a.b/c".into(), "value".into()))
        );
    }

    #[test]
    fn a_fenced_example_is_never_a_header_property() {
        let lines = header_lines("```\nnote:: keep me\n```\nkey:: v\n", false);
        assert_eq!(lines.len(), 4);
        assert!(lines[..3].iter().all(|(_, p)| p.is_none()), "{lines:?}");
        assert_eq!(lines[3].1, Some(("key".into(), "v".into())));
    }
}

/// Where a page's header is, by the store's alias answer
/// (`tine_store::query::page_header_text`, I-12): the pre-block, else a
/// properties-only first root, else none.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum HeaderAt {
    PreBlock,
    FirstRoot,
}

pub(super) fn header_of(page: &PageDto) -> Option<(HeaderAt, &str)> {
    let first = page.blocks.first().map(|block| block.raw.as_str());
    let text = tine_store::query::page_header_text(page.pre_block.as_deref(), first)?;
    Some(if page.pre_block.is_some() {
        (HeaderAt::PreBlock, text)
    } else {
        (HeaderAt::FirstRoot, text)
    })
}

/// The survivor's new header (when it changes) and the blocks `source` adds,
/// with `renames` applied to the source. Both headers are found where the
/// store's alias answer reads them ([`header_of`]): a source header written as
/// its first bullet joins the survivor's header like a pre-block (its child
/// blocks, if any, move as top-level blocks), and the caller writes the new
/// header back where the survivor's was, so a survivor whose header is its
/// first bullet keeps it (REG-CM-MERGE-LOSES-ALIASES).
pub(super) fn payload(
    survivor: &PageDto,
    source: &PageDto,
    renames: Option<super::MergeNames<'_>>,
    org: bool,
    format: tine_core::config::FileNameFormat,
) -> (Option<String>, Vec<tine_core::model::BlockDto>) {
    let mut source = source.clone();
    if let Some(renames) = renames {
        rename_doc(&mut source, renames, org, format);
    }
    let mut new_header = None;
    let mut blocks = Vec::new();
    let mut header_children = Vec::new();
    let source_header = match header_of(&source) {
        Some((HeaderAt::PreBlock, text)) => Some(text.to_owned()),
        Some((HeaderAt::FirstRoot, text)) => {
            let text = text.to_owned();
            let first = source.blocks.remove(0);
            header_children = first.children;
            Some(text)
        }
        None => None,
    };
    if let Some(pre) = source_header.as_deref() {
        let mut header: Vec<(String, Option<(String, String)>)> = header_of(survivor)
            .map(|(_, kept)| {
                header_lines(kept, org)
                    .into_iter()
                    .map(|(line, property)| (line.to_owned(), property))
                    .collect()
            })
            .unwrap_or_default();
        let mut moved = Vec::new();
        for (line, property) in header_lines(pre, org) {
            let Some((key, value)) = property else {
                moved.push(line);
                continue;
            };
            let clash = header
                .iter()
                .position(|(_, kept)| kept.as_ref().is_some_and(|(k, _)| *k == key));
            let equal = clash.is_some_and(|at| {
                header[at]
                    .1
                    .as_ref()
                    .is_some_and(|(_, v)| v.trim() == value.trim())
            });
            match clash {
                Some(_) if equal => {}
                None if key != "title" => header.push((line.to_owned(), Some((key, value)))),
                Some(at) if tine_store::query::is_alias_key(&key) => {
                    let kept_value = header[at]
                        .1
                        .as_ref()
                        .map(|(_, v)| v.clone())
                        .unwrap_or_default();
                    // Members split like the reference evidence and OG
                    // `sep-by-comma`: `,` or `，` (C3Y Y4).
                    let known: HashSet<String> = kept_value
                        .split(refs::is_linkable_property_separator)
                        .map(refs::normalize)
                        .collect();
                    let extra: Vec<&str> = value
                        .split(refs::is_linkable_property_separator)
                        .map(str::trim)
                        .filter(|alias| {
                            !alias.is_empty() && !known.contains(&refs::normalize(alias))
                        })
                        .collect();
                    if !extra.is_empty() {
                        let join = |start: &str| {
                            extra
                                .iter()
                                .fold(start.trim_end().to_owned(), |line, alias| {
                                    format!("{line}, {alias}")
                                })
                        };
                        header[at] = (join(&header[at].0), Some((key, join(&kept_value))));
                    }
                }
                _ => moved.push(line),
            }
        }
        if !header.is_empty() {
            new_header = Some(lines(header.iter().map(|(line, _)| line.as_str())));
        }
        if moved.iter().any(|line| !line.trim().is_empty()) {
            blocks.push(tine_core::model::BlockDto {
                raw: lines(moved.into_iter()),
                ..Default::default()
            });
        }
    }
    blocks.extend(header_children);
    blocks.extend(source.blocks);
    (new_header, blocks)
}
