//! Native name coordinates are produced while the reader still holds the AST.
//! No parse is added: delimiters locate the target of an accepted link, and
//! label runs use inline source spans (whose bytes include Markdown escapes).

use super::{Inline, ProjectedPageRef, Range, SlottedPageRef, SpanMapper, Url};

#[cfg(test)]
thread_local! {
    pub(crate) static SLOT_WORK: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

fn work() {
    #[cfg(test)]
    SLOT_WORK.with(|n| n.set(n.get() + 1));
}

/// One reference's native name extent in block raw offsets. A formatted name
/// can occupy several runs; then `run` is absent and O7 leaves the site.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NameSlot {
    /// Complete extent containing the authored name, including markup.
    pub extent: Range<usize>,
    /// This name stands inside a different reference's name (O7).
    pub nested: bool,
    /// This link is the native wrapper spelling a tag's same name, rather
    /// than an independent rename site.
    pub encoding: bool,
    /// A property value's member inside a macro other than `embed` (mldoc
    /// reads property references with macros skipped). It names a page, but
    /// its rename site is the macro argument the walk reports as a rewrite
    /// target, so rename and observation skip it here.
    pub in_macro: bool,
    /// Only file labels need a second extent for the unique name run. A
    /// split label has no run; ordinary references allocate no extra record.
    pub label: Option<Box<LabelRun>>,
}

/// The replacement run of a file label, absent when markup splits its name.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LabelRun(pub Option<Range<usize>>);

impl NameSlot {
    pub(super) fn whole(extent: Range<usize>) -> Self {
        work();
        Self {
            extent,
            nested: false,
            encoding: false,
            in_macro: false,
            label: None,
        }
    }

    /// The native name run, or no contiguous spelling for a split label.
    pub fn run(&self) -> Option<&Range<usize>> {
        self.label.as_ref().map_or_else(
            || (!self.extent.is_empty()).then_some(&self.extent),
            |label| label.0.as_ref(),
        )
    }
}

pub(super) fn link(
    raw: &str,
    reference: &ProjectedPageRef,
    url: &Url,
    label: &[Inline],
    mapper: SpanMapper,
    org: bool,
) -> NameSlot {
    work();
    let range = &reference.range;
    let node = &raw[range.clone()];
    let label_name = matches!(url, Url::File { .. });
    let local = if !label_name {
        tine_outline::property_refs::page_link_run(node, org)
    } else if org {
        node.find("][").map(|at| at + 2..node.len() - 2)
    } else if node.starts_with("[[") {
        Some(2..node.len() - 2)
    } else {
        node.rfind("](")
            .map(|split| usize::from(node.starts_with('!')) + 1..split)
    };
    let Some(local) = local else {
        return NameSlot::whole(range.clone());
    };
    let extent = range.start + local.start..range.start + local.end;
    if !label_name {
        return NameSlot::whole(extent);
    }
    fn runs(
        nodes: &[Inline],
        name: &str,
        mapper: SpanMapper,
        raw: &str,
        found: &mut Vec<Range<usize>>,
    ) {
        let len = raw.len();
        for node in nodes {
            match node {
                Inline::Plain {
                    text,
                    span: Some(span),
                    ..
                } if text == name => {
                    if let Some(range) = mapper.map(span, len) {
                        found.push(range);
                    }
                }
                Inline::Code {
                    text,
                    span: Some(span),
                    ..
                }
                | Inline::Verbatim {
                    text,
                    span: Some(span),
                    ..
                } if text == name => {
                    if let Some(range) = mapper.map(span, len) {
                        let source = &raw[range.clone()];
                        let mut matches = source.match_indices(text);
                        if let (Some((at, _)), None) = (matches.next(), matches.next()) {
                            found.push(range.start + at..range.start + at + text.len());
                        }
                    }
                }
                Inline::Emphasis { children, .. }
                | Inline::Subscript { children, .. }
                | Inline::Superscript { children, .. } => runs(children, name, mapper, raw, found),
                _ => {}
            }
        }
    }
    let mut found = Vec::new();
    runs(label, &reference.name, mapper, raw, &mut found);
    let run = if found.len() == 1 { found.pop() } else { None };
    NameSlot {
        extent,
        nested: false,
        encoding: false,
        in_macro: false,
        label: Some(Box::new(LabelRun(run))),
    }
}

/// Name intervals, rather than node intervals, decide nesting. The equal
/// interval of the link spelling a bracketed tag is the same name run.
/// Sorting costs O(s log s); the stack visits each reference once.
pub(super) fn mark_nested(references: &mut [SlottedPageRef]) {
    work();
    let mut order: Vec<_> = (0..references.len()).collect();
    order.sort_by_key(|&i| {
        (
            references[i].slot.extent.start,
            std::cmp::Reverse(references[i].slot.extent.end),
        )
    });
    let mut open: Vec<usize> = Vec::new();
    for i in order {
        let r = &references[i];
        while open.last().is_some_and(|&p| {
            references[p].slot.extent.end < r.slot.extent.end
                || references[p].slot.extent.end <= r.slot.extent.start
        }) {
            open.pop();
        }
        let parent = open.last().map(|&p| &references[p]);
        let equal = parent.is_some_and(|p| p.slot.extent == r.slot.extent);
        let nested = parent.is_some_and(|p| p.slot.nested || !equal);
        let encoding = equal
            && parent.is_some_and(|p| {
                p.rule == "explicit_tag" && r.rule == "explicit_link" && p.name == r.name
            });
        references[i].slot.nested = nested;
        references[i].slot.encoding = encoding;
        // Equal extents do not hide their enclosing interval from later sites.
        if !equal {
            open.push(i);
        }
    }
}
