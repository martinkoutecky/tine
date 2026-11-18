//! Reader-owned name spellings for local reference sites (O1/O7).
//! Both the span writer and P2's input-derived expectation ask this answer.

use std::ops::Range;

/// A file label as the reader sees it: its complete link (including the
/// target, image prefix and formatting), label extent, and the unique run
/// spelling the old name. A split name has no run. All offsets are in `node`.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct FileLabel {
    pub node: String,
    pub label: Range<usize>,
    pub run: Option<Range<usize>>,
    org: bool,
}

impl FileLabel {
    /// Recognize the label of a parser-reported file link. Page targets
    /// (`[[page]]`, Org `[[page][label]]`, Markdown `[label]([[page]])`)
    /// name their target instead and return `None`. O(link bytes).
    pub fn read(
        raw: &str,
        reference: &crate::reference_evidence::SlottedPageRef,
        org: bool,
    ) -> Option<Self> {
        reference.slot.label.as_ref()?;
        let relative =
            |r: &Range<usize>| r.start - reference.range.start..r.end - reference.range.start;
        let label = relative(&reference.slot.extent);
        let run = reference.slot.run().map(relative);
        Some(Self {
            node: raw[reference.range.clone()].to_owned(),
            label,
            run,
            org,
        })
    }

    /// The replacement for `run`, or no faithful spelling here. Preserve
    /// formatting and try the name, then Markdown punctuation escapes. The
    /// complete candidate must parse as one link naming exactly `to`.
    /// No line break can be written into a name run. At most two parses of
    /// this link node; no page or graph scan and no edit application.
    pub fn spelling(&self, to: &str) -> Option<String> {
        let run = self.run.as_ref()?;
        reference_spelling(&self.node, run.clone(), to, self.org, "explicit_link", &[])
    }
}

/// A name run in one parser-reported reference, kept in its real node.
/// Return a replacement only if that complete reference still reads the
/// new name and exactly the expected inner occurrences (name and native text,
/// in preorder). At most two local projections; no edit application.
pub(crate) fn reference_spelling(
    node: &str,
    run: Range<usize>,
    to: &str,
    org: bool,
    rule: &str,
    inner: &[(&str, &str)],
) -> Option<String> {
    if to.contains(['\n', '\r']) {
        return None;
    }
    std::iter::once(to.to_owned())
        .chain((!org).then(|| markdown_escaped(to)))
        .find(|spelling| {
            let node = format!("{}{spelling}{}", &node[..run.start], &node[run.end..]);
            reads_reference_inner(&node, org, rule, to, Some(0..node.len()), Some(inner))
        })
}

/// O1/Lean `nsSubst`: keep sites outside the replaced name prefix. Input is
/// in preorder, with native ranges; dropping a site drops its descendants.
/// The spelling confirmation and P2's expected side share this one rule.
pub(crate) fn retained_inner<T>(
    slot: Range<usize>,
    prefix: usize,
    sites: impl Iterator<Item = (Range<usize>, T)>,
) -> impl Iterator<Item = (Range<usize>, T)> {
    let mut dropped_until = 0;
    sites.filter(move |(range, _)| {
        if range.start < dropped_until {
            return false;
        }
        let keep = !(slot.start <= range.start && range.end <= slot.end)
            || slot.start + prefix <= range.start;
        if !keep {
            dropped_until = range.end;
        }
        keep
    })
}

/// An Org file stem must remain a parser-reported file link, and decode
/// through the graph's codec to the new name. A delimiter in the encoded
/// stem that destroys that link has no spelling here (O7).
pub(crate) fn file_stem_spelling(
    node: &str,
    to: &str,
    names: crate::config::FileNameFormat,
) -> Option<String> {
    let run = crate::model::org_file_link_stem(node)?;
    let spelling = crate::model::encode_page_name(to, names);
    let candidate = format!("{}{}{}", &node[..run.start], spelling, &node[run.end..]);
    let reading = |text: &str| {
        let mut block = crate::doc::DocBlock::new(text);
        block.set_org(true);
        block
            .projection()
            .reference_source()
            .explicit
            .iter()
            .map(|r| (r.rule, r.name.clone()))
            .collect::<Vec<_>>()
    };
    // A delimiter in the stem can keep a FileLink target yet swallow its
    // label. The other names of the link must retain their old reading.
    if reading(node) != reading(&candidate) {
        return None;
    }
    let targets = crate::reference_evidence::rename_targets(&candidate, true);
    targets
        .into_iter()
        .any(|target| match target {
            crate::reference_evidence::RewriteTarget::FileLink(range)
                if range == (0..candidate.len()) =>
            {
                crate::model::org_file_link_stem(&candidate).is_some_and(|stem| {
                    crate::model::decode_page_name(&candidate[stem], names) == to
                })
            }
            _ => false,
        })
        .then_some(spelling)
}

/// A bracketed page name, including escapes when Markdown reads them away.
pub(crate) fn bracket_name(to: &str, org: bool) -> Option<String> {
    reference_spelling("[[Old]]", 2..5, to, org, "explicit_link", &[])
}

/// The parser's one reference of `rule` has exactly `name`, optionally at
/// the required range. Used for site-specific name spellings.
pub(crate) fn reads_reference(
    node: &str,
    org: bool,
    rule: &str,
    name: &str,
    range: Option<Range<usize>>,
) -> bool {
    reads_reference_inner(node, org, rule, name, range, None)
}

fn reads_reference_inner(
    node: &str,
    org: bool,
    rule: &str,
    name: &str,
    range: Option<Range<usize>>,
    inner: Option<&[(&str, &str)]>,
) -> bool {
    let mut block = crate::doc::DocBlock::new(node);
    block.set_org(org);
    let source = block.read_for_rename();
    let mut references = source.explicit.iter().filter(|r| {
        (r.rule == rule || matches!((rule, r.rule), ("explicit_nested_link", "explicit_link")))
            && range.as_ref().is_none_or(|range| &r.range == range)
    });
    let (Some(reference), None) = (references.next(), references.next()) else {
        return false;
    };
    reference.name == name
        && inner.is_none_or(|inner| {
            source
                .explicit
                .iter()
                .filter(|r| {
                    !r.slot.encoding
                        && r.range != reference.range
                        && reference.range.start <= r.range.start
                        && r.range.end <= reference.range.end
                })
                .map(|r| (r.name.as_str(), &node[r.range.clone()]))
                .eq(inner.iter().copied())
        })
}

/// CommonMark's backslash-escaped ASCII punctuation; the parser, rather
/// than this candidate generator, decides whether it spells the name.
fn markdown_escaped(to: &str) -> String {
    let mut out = String::with_capacity(to.len() * 2);
    for c in to.chars() {
        if c.is_ascii_punctuation() {
            out.push('\\');
        }
        out.push(c);
    }
    out
}
