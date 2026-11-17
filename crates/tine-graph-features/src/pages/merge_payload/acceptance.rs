//! One comparison of allocation's intended description with the output reader.
use super::{Header, MergePart, RealizationError, RealizationFailure, Writer};
use std::collections::HashMap;
use tine_core::{lsdoc::ast::Block, refs, DocBlock};

#[cfg(test)]
#[path = "../merge_acceptance_tests.rs"]
mod tests;

#[cfg(test)]
thread_local! {
    static CHECK_BYTES: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

#[derive(Default)]
pub(super) struct Intended {
    lines: Vec<(String, Option<(String, bool)>)>,
}

pub(super) struct Counts {
    pub blocks: [usize; 3],
    pub preamble: bool,
}

impl Intended {
    pub fn emit(
        &mut self,
        w: &mut Writer,
        file: usize,
        h: &Header,
        n: usize,
        united: bool,
        additions: &[Vec<usize>],
        kept: &[bool],
    ) -> Result<(), RealizationFailure> {
        let line = &h.lines[n];
        if line.frame {
            return w.original(file, line.content.clone());
        }
        let raw = if united {
            w.field(file, line, additions, kept)?
        } else {
            let mut raw = String::new();
            w.payload(file, line.payload.clone(), &mut raw)?;
            w.original(file, line.content.clone())?;
            raw
        };
        if !line.blank {
            self.lines.push((
                raw,
                line.property
                    .as_ref()
                    .map(|(k, _)| (k.clone(), line.drawer.is_none())),
            ));
        }
        Ok(())
    }

    pub fn check(
        &self,
        w: &Writer,
        inputs: &[Header; 2],
        leftover: &[usize],
        indented: bool,
        normalize: bool,
        org: bool,
        target: &str,
        format: &refs::RenameFormat<'_>,
    ) -> Result<Counts, RealizationFailure> {
        let read = Header::read(&w.text, org);
        let refused = || RealizationError::BodyLaw.at(MergePart::Output);
        if read.title.as_ref().is_some_and(|name| {
            !refs::same_page(name, target)
                && !format.days.is_some_and(|days| {
                    days.parse(name)
                        .is_some_and(|day| days.parse(target) == Some(day))
                })
        }) {
            return Err(RealizationError::SurvivorIdentity.at(MergePart::Output));
        }
        self.check_header(&read, &w.text)?;
        let mut roots = read.document.roots.iter();
        let mut scratch = String::new();
        let [dst, src] = inputs;
        let target_roots =
            dst.document.roots.len() - usize::from(normalize) + usize::from(dst.prose.is_some());
        let source_roots = src.document.roots.len() - usize::from(src.first_root)
            + usize::from(src.prose.is_some());
        // The leftover quote must own its lines: a top-level Org directive in
        // that output root, outside its literal regions, escaped it.
        let directive = |root: &DocBlock| {
            let is_directive = |b: &Block| matches!(b, Block::Directive { .. });
            let ast = tine_core::render::parse_text_bounded(root.raw(), true);
            ast.is_none_or(|ast| ast.blocks.iter().any(is_directive))
        };
        let quoted = read.document.roots.get(target_roots);
        if org && !leftover.is_empty() && quoted.is_some_and(directive) {
            return Err(RealizationError::DirectiveEscape.at(MergePart::Source));
        }
        if read.document.roots.len()
            != target_roots + usize::from(!leftover.is_empty()) + source_roots
        {
            return Err(if indented {
                RealizationError::IndentedBody.at(MergePart::Source)
            } else {
                refused()
            });
        }
        if dst.prose.is_some() {
            prose(w, 0, dst, roots.next().unwrap(), &mut scratch)?;
        }
        let skip = usize::from(normalize);
        let mut native = skip;
        let mut target_count = 0;
        for (n, original) in dst.document.roots[skip..].iter().enumerate() {
            target_count += tree(
                w,
                0,
                original,
                roots.next().unwrap(),
                dst,
                &mut native,
                dst.first_root && !normalize && n == 0,
                &mut scratch,
            )?;
        }
        let mut source_native = usize::from(src.first_root);
        let source_children = if src.first_root {
            src.document.roots[0].children.as_slice()
        } else {
            &[]
        };
        let mut child_count = 0;
        if !leftover.is_empty() {
            let actual = roots.next().unwrap();
            scratch.clear();
            if org {
                scratch.push_str("\n#+BEGIN_QUOTE");
            }
            for &n in leftover {
                scratch.push('\n');
                append(w, 1, src.lines[n].payload.clone(), &mut scratch)?;
            }
            if org {
                scratch.push_str("\n#+END_QUOTE");
            }
            if scratch != actual.raw() {
                return Err(RealizationError::BodyLaw.at(MergePart::Source));
            }
            if source_children.len() != actual.children.len() {
                return Err(RealizationError::BodyLaw.at(MergePart::Source));
            }
            for (a, b) in source_children.iter().zip(&actual.children) {
                child_count += tree(w, 1, a, b, src, &mut source_native, false, &mut scratch)?;
            }
            if org {
                let mut required = HashMap::new();
                for &n in leftover {
                    for r in &src.lines[n].references {
                        *required
                            .entry((r.rule, refs::page_key(&r.name)))
                            .or_insert(0usize) += 1;
                    }
                }
                let mut found = HashMap::new();
                for r in actual.projection().reference_source().explicit {
                    if !matches!(r.rule, "explicit_task_marker" | "explicit_priority") {
                        *found
                            .entry((r.rule, refs::page_key(&r.name)))
                            .or_insert(0usize) += 1;
                    }
                }
                if required != found {
                    return Err(RealizationError::HiddenOccurrence.at(MergePart::Source));
                }
            }
        }
        if src.prose.is_some() {
            prose(w, 1, src, roots.next().unwrap(), &mut scratch)?;
        }
        let mut source_count = 0;
        for original in &src.document.roots[usize::from(src.first_root)..] {
            source_count += tree(
                w,
                1,
                original,
                roots.next().unwrap(),
                src,
                &mut source_native,
                false,
                &mut scratch,
            )?;
        }
        Ok(Counts {
            blocks: [
                target_count - usize::from(dst.first_root && !normalize),
                child_count,
                source_count,
            ],
            preamble: read.document.pre_block.is_some(),
        })
    }

    fn check_header(&self, read: &Header, text: &str) -> Result<(), RealizationFailure> {
        let refused = || RealizationError::BodyLaw.at(MergePart::Output);
        if read.document.pre_block.is_some() && read.prose.is_some() {
            return Err(refused());
        }
        let mut expected = self.lines.iter();
        for line in read.lines.iter().filter(|l| !l.blank && !l.frame) {
            let Some((raw, property)) = expected.next() else {
                return Err(refused());
            };
            #[cfg(test)]
            CHECK_BYTES.with(|n| n.set(n.get() + line.payload.len()));
            if raw != &text[line.payload.clone()]
                || property
                    != &line
                        .property
                        .as_ref()
                        .map(|(k, _)| (k.clone(), line.drawer.is_none()))
            {
                return Err(refused());
            }
        }
        if expected.next().is_some() {
            return Err(refused());
        }
        Ok(())
    }
}

fn append(
    w: &Writer,
    file: usize,
    r: std::ops::Range<usize>,
    raw: &mut String,
) -> Result<(), RealizationFailure> {
    #[cfg(test)]
    super::BODY_BYTES.with(|n| n.set(n.get() + r.len()));
    w.payload(file, r, raw)
}

fn prose(
    w: &Writer,
    file: usize,
    h: &Header,
    actual: &DocBlock,
    raw: &mut String,
) -> Result<(), RealizationFailure> {
    raw.clear();
    append(w, file, h.prose.clone().unwrap(), raw)?;
    if tine_core::doc::normalize_line_endings(raw) != actual.raw() || !actual.children.is_empty() {
        return Err(RealizationError::BodyLaw.at(file.into()));
    }
    Ok(())
}

fn tree(
    w: &Writer,
    file: usize,
    original: &DocBlock,
    actual: &DocBlock,
    h: &Header,
    block: &mut usize,
    header: bool,
    raw: &mut String,
) -> Result<usize, RealizationFailure> {
    let refused = || RealizationError::BodyLaw.at(file.into());
    let lines = h.map.block(*block);
    *block += 1;
    if original.children.len() != actual.children.len() {
        return Err(refused());
    }
    if !header {
        raw.clear();
        for i in lines.clone() {
            if i > lines.start {
                raw.push('\n');
            }
            append(w, file, h.map.payload(i), raw)?;
        }
        if raw != actual.raw() {
            return Err(refused());
        }
    }
    let mut count = 1;
    for (a, b) in original.children.iter().zip(&actual.children) {
        count += tree(w, file, a, b, h, block, false, raw)?;
    }
    Ok(count)
}
