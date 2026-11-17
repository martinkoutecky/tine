//! Native placement acceptance: content, children and order from reader coordinates.
use super::{Header, MergePart, RealizationError, RealizationFailure, Writer};
use std::ops::Range;
use tine_core::{model::BlockDto, DocBlock};

pub(super) struct Body {
    /// Logical lines of each block in preorder, without native layout or EOLs.
    lines: Vec<Vec<Range<usize>>>,
    pub prefixes: Vec<Range<usize>>,
}

fn append(
    writer: &Writer,
    file: usize,
    range: Range<usize>,
    raw: &mut String,
) -> Result<(), RealizationFailure> {
    #[cfg(test)]
    super::BODY_BYTES.with(|count| count.set(count.get() + range.len()));
    for id in writer.interval(file, range)? {
        if writer.trace.wrappers.contains(&id) {
            raw.push_str("[[");
        }
        raw.push_str(
            writer
                .trace
                .substitution
                .get(&id)
                .map(String::as_str)
                .unwrap_or_else(|| writer.trace.original(id)),
        );
        if writer.trace.wrappers.contains(&id) {
            raw.push_str("]]");
        }
    }
    Ok(())
}

/// Input-derived prose and O6's fixed leftover payload, checked by the same
/// output read as the native bodies. O(payload bytes), no additional parse.
pub(super) fn check_payloads(
    writer: &Writer,
    file: usize,
    header: &Header,
    prose: Option<&DocBlock>,
    leftover: Option<(&[usize], &DocBlock)>,
    org: bool,
) -> Result<(), RealizationFailure> {
    let refused = || RealizationError::BodyLaw.at(file.into());
    let mut raw = String::new();
    if let (Some(range), Some(block)) = (&header.prose, prose) {
        append(writer, file, range.clone(), &mut raw)?;
        let normalized = tine_core::doc::normalize_line_endings(&raw);
        if normalized != block.raw() || !block.children.is_empty() {
            return Err(refused());
        }
    }
    if let Some((lines, block)) = leftover {
        raw.clear();
        if org {
            raw.push_str("\n#+BEGIN_QUOTE");
        }
        for &n in lines {
            raw.push('\n');
            append(writer, file, header.lines[n].payload.clone(), &mut raw)?;
        }
        if org {
            raw.push_str("\n#+END_QUOTE");
        }
        if raw != block.raw() {
            return Err(refused());
        }
    }
    Ok(())
}

impl Body {
    /// Reuse Header's outline read. A root's offset applies only to its subtree;
    /// each line loses at most that many bytes of its own reader-owned prefix.
    /// O(body bytes); coordinate ranges only, no parse or text copy.
    pub fn read(
        text: &str,
        outline: &[(Range<usize>, usize, u32, usize)],
        body_start: usize,
        org: bool,
    ) -> Self {
        let mut body = Self {
            lines: Vec::with_capacity(outline.len()),
            prefixes: Vec::new(),
        };
        let (mut root_level, mut offset) = (u32::MAX, 0);
        for (range, content, level, indent) in outline {
            if *level <= root_level {
                root_level = *level;
                offset = if org {
                    0
                } else {
                    text[range.start..*content]
                        .bytes()
                        .take_while(|b| matches!(b, b' ' | b'\t' | 0x0c))
                        .count()
                };
            }
            // Org's reader omits final line breaks; Markdown omits just the
            // final terminator (physical_lines already does that).
            let end = if org && range.end == text.len() {
                text.trim_end_matches(['\r', '\n']).len()
            } else {
                range.end
            };
            let mut at = range.start;
            let mut lines = Vec::new();
            for (n, line) in tine_core::doc::physical_lines(&text[range.start..end]).enumerate() {
                let raw = line.trim_end_matches(['\r', '\n']);
                let skip = if n == 0 {
                    content - at
                } else {
                    raw.len() - tine_core::doc::continuation_raw(raw, *indent).len()
                };
                lines.push(at + skip..at + raw.len());
                if at >= body_start && offset > 0 {
                    #[cfg(test)]
                    super::BODY_LINES.with(|count| count.set(count.get() + 1));
                    let layout = if n == 0 {
                        raw.bytes()
                            .take_while(|b| matches!(b, b' ' | b'\t' | 0x0c))
                            .count()
                    } else {
                        skip
                    };
                    let prefix = offset.min(layout);
                    if prefix > 0 {
                        body.prefixes.push(at..at + prefix);
                    }
                }
                at += line.len();
            }
            body.lines.push(lines);
        }
        body
    }

    pub fn cuts(&self) -> impl Iterator<Item = usize> + '_ {
        self.lines
            .iter()
            .flatten()
            .chain(&self.prefixes)
            .flat_map(|r| [r.start, r.end])
    }

    /// Count accepted blocks; reject a placement that changes any raw body,
    /// child tree or order. `start` is the native preorder index; `header`
    /// permits only the first root's allocated header text to change.
    /// O(compared bytes + pieces), one reusable block-sized comparison buffer.
    pub fn check(
        &self,
        writer: &Writer,
        file: usize,
        expected: &[BlockDto],
        actual: &[DocBlock],
        start: usize,
        header: bool,
    ) -> Result<usize, RealizationFailure> {
        let mut native = self.lines[start..].iter();
        let mut raw = String::new();
        fn walk(
            writer: &Writer,
            file: usize,
            expected: &[BlockDto],
            actual: &[DocBlock],
            native: &mut std::slice::Iter<'_, Vec<Range<usize>>>,
            header: bool,
            raw: &mut String,
        ) -> Result<usize, RealizationFailure> {
            let refused = || RealizationError::BodyLaw.at(MergePart::from(file));
            if expected.len() != actual.len() {
                return Err(refused());
            }
            let mut count = 0;
            for (n, (original, placed)) in expected.iter().zip(actual).enumerate() {
                let lines = native.next().ok_or_else(refused)?;
                if !(header && n == 0) {
                    raw.clear();
                    for (n, line) in lines.iter().enumerate() {
                        if n > 0 {
                            raw.push('\n');
                        }
                        append(writer, file, line.clone(), raw)?;
                    }
                    if raw != placed.raw() {
                        return Err(refused());
                    }
                }
                count += 1 + walk(
                    writer,
                    file,
                    &original.children,
                    &placed.children,
                    native,
                    false,
                    raw,
                )?;
            }
            Ok(count)
        }
        walk(
            writer,
            file,
            expected,
            actual,
            &mut native,
            header,
            &mut raw,
        )
    }
}
