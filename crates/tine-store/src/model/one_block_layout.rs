//! Preserve physical lines outside one changed Markdown block.
//!
//! The DTO carries semantic block text, not continuation indentation or the
//! number of blank lines at EOF. Rebuilding the whole page would rewrite those
//! lines even when a save changes only one block.

use tine_core::doc::{self, DocBlock, Document, SerializeOpts};

/// Return a minimal one-block edit when the source and DTO have the same tree
/// and preamble, and the splice re-parses to the same blocks (see `same_blocks`).
/// Cost: three parses and two serializations of the page. Falls back to the ordinary serializer for structural edits or
/// files whose physical lines cannot be mapped safely to the parsed tree.
pub(super) fn serialize(doc: &Document, source: &str, opts: &SerializeOpts) -> Option<String> {
    if source.contains('\r') {
        return None;
    }
    let old = doc::parse(source);
    if old.pre_block != doc.pre_block || old.roots.len() != doc.roots.len() {
        return None;
    }
    let mut changed = None;
    let mut line = old.pre_block.as_ref().map_or(0, |s| s.split('\n').count());
    if old.pre_block.is_some() && !old.roots.is_empty() && opts.blank_after_props {
        line += 1;
    }
    for (before, after) in old.roots.iter().zip(&doc.roots) {
        compare_blocks(before, after, &mut line, &mut changed)?;
    }
    let (start, mut old_len, mut new_len, same_tail) = changed?;
    let trailing = source.len() - source.trim_end_matches('\n').len();
    let physical: Vec<_> = source[..source.len() - trailing].split('\n').collect();
    let mut no_tail = opts.clone();
    no_tail.trailing_newlines = 0;
    let old_rendered = doc::serialize_with(&old, &no_tail);
    let old_rendered: Vec<_> = old_rendered.split('\n').collect();
    if old_rendered.len() != line
        || physical.len() > line
        || old_rendered[physical.len()..]
            .iter()
            .any(|line| !line.is_empty())
    {
        return None;
    }
    // The parser assigns blank EOF lines to the last block's raw body. The
    // physical file already carries them in its trailing newline run.
    let overflow = line - physical.len();
    if physical.get(start)?.trim() != old_rendered.get(start)?.trim() {
        return None;
    }
    if start + old_len > physical.len() {
        if start + old_len != line || old_len < overflow || new_len < overflow || !same_tail {
            return None;
        }
        old_len -= overflow;
        new_len -= overflow;
    }
    let rendered = doc::serialize_with(doc, &no_tail);
    let rendered: Vec<_> = rendered.split('\n').collect();
    if start + old_len > physical.len() || start + new_len > rendered.len() {
        return None;
    }
    let mut result: Vec<String> = Vec::with_capacity(physical.len() - old_len + new_len);
    // The serializer indents from column 0; the file may carry a base offset
    // (every root under one tab). Re-base the edited lines on the file's own prefix.
    let lead = |line: &str| line.len() - line.trim_start_matches([' ', '\t']).len();
    let (file_prefix, ours) = (
        &physical[start][..lead(physical[start])],
        &rendered[start][..lead(rendered[start])],
    );
    let edited: Vec<String> = rendered[start..start + new_len]
        .iter()
        .map(|line| match line.strip_prefix(ours) {
            Some(rest) if !line.is_empty() => format!("{file_prefix}{rest}"),
            _ => line.to_string(),
        })
        .collect();
    result.extend(physical[..start].iter().map(|line| line.to_string()));
    result.extend(edited);
    result.extend(
        physical[start + old_len..]
            .iter()
            .map(|line| line.to_string()),
    );
    let mut result = result.join("\n");
    result.push_str(&"\n".repeat(trailing));
    // The line mapping above is a heuristic. Keep the splice only when every
    // untouched block parses exactly as before and the edited block parses as
    // the ordinary serializer renders it; otherwise a misaligned span could
    // overwrite a neighbour.
    let spliced = doc::parse(&result);
    let full = doc::parse(&doc::serialize_with(doc, opts));
    if spliced.pre_block != old.pre_block
        || !same_blocks(&old.roots, &doc.roots, &spliced.roots, &full.roots)
    {
        return None;
    }
    Some(result)
}

/// `true` when `spliced` keeps `old`'s raw for blocks the DTO left alone and
/// matches `full` (EOF newlines aside) for the block it changed.
fn same_blocks(
    old: &[DocBlock],
    dto: &[DocBlock],
    spliced: &[DocBlock],
    full: &[DocBlock],
) -> bool {
    spliced.len() == old.len()
        && full.len() == old.len()
        && old
            .iter()
            .zip(dto)
            .zip(spliced)
            .zip(full)
            .all(|(((o, d), s), f)| {
                let raw_ok = if o.raw() == d.raw() {
                    s.raw() == o.raw()
                } else {
                    s.raw().trim_end_matches('\n') == f.raw().trim_end_matches('\n')
                };
                raw_ok && same_blocks(&o.children, &d.children, &s.children, &f.children)
            })
}

fn compare_blocks(
    before: &DocBlock,
    after: &DocBlock,
    line: &mut usize,
    changed: &mut Option<(usize, usize, usize, bool)>,
) -> Option<()> {
    if before.children.len() != after.children.len() {
        return None;
    }
    let old_len = before.raw().split('\n').count();
    if before.raw() != after.raw() {
        if changed.is_some() {
            return None;
        }
        let old_tail = before
            .raw()
            .bytes()
            .rev()
            .take_while(|byte| *byte == b'\n')
            .count();
        let new_tail = after
            .raw()
            .bytes()
            .rev()
            .take_while(|byte| *byte == b'\n')
            .count();
        *changed = Some((
            *line,
            old_len,
            after.raw().split('\n').count(),
            old_tail == new_tail,
        ));
    }
    *line += old_len;
    for (old_child, new_child) in before.children.iter().zip(&after.children) {
        compare_blocks(old_child, new_child, line, changed)?;
    }
    Some(())
}
