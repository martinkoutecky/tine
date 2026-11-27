use super::*;

fn is_properties_drawer(raw: &str, r: &Range) -> bool {
    r.slice(raw)
        .lines()
        .next()
        .is_some_and(|line| is_property_drawer_marker(line, true))
}

/// Each line of a parser-accepted property region, with whether it is a
/// property drawer's own delimiter: `:PROPERTIES:` opens a drawer only outside
/// one, `:END:` closes only an open one, and an opener that never closes is an
/// ordinary line. So an entry spelled `:properties:` or `:end:` inside a drawer
/// stays an entry, and a region the parser folded from several drawers,
/// directives or Markdown rows keeps each drawer's delimiters. The one walk
/// both entry ownership and drawer bounds read. O(region bytes).
pub(super) fn region_lines(raw: &str, region: Range) -> Vec<(Range, bool)> {
    let mut lines: Vec<(Range, bool)> = Vec::new();
    let mut open = None;
    for line in line_ranges(raw, region) {
        let text = line.slice(raw).trim_end_matches(['\r', '\n']);
        let delimiter = if open.is_none() {
            is_property_drawer_marker(text, true)
        } else {
            is_property_drawer_marker(text, false)
        };
        if delimiter {
            open = if open.is_none() {
                Some(lines.len())
            } else {
                None
            };
        }
        lines.push((line, delimiter));
    }
    if let Some(opener) = open {
        lines[opener].1 = false;
    }
    lines
}

impl BlockRegions {
    pub(super) fn own_org_region(&self, raw: &str) -> Option<usize> {
        let at = self.head_end(raw, true);
        self.property_regions
            .iter()
            .position(|r| (r.0 == at || r.0 == 0) && is_properties_drawer(raw, r))
    }
    /// Whether a `:PROPERTIES:` drawer region starts at `at`.
    pub fn properties_drawer_at(&self, raw: &str, at: usize) -> bool {
        self.property_regions
            .iter()
            .any(|r| r.0 == at && is_properties_drawer(raw, r))
    }
    /// Delimiter byte offsets inside parser-accepted property regions. Used by
    /// the page form to remove an emptied drawer without reading literal text.
    pub fn property_drawer_boundaries(&self, raw: &str) -> Vec<(usize, usize)> {
        let mut out = Vec::new();
        for region in &self.property_regions {
            let mut open = None;
            for (line, delimiter) in region_lines(raw, *region) {
                if delimiter {
                    match open.take() {
                        None => open = Some(line.0),
                        Some(start) => out.push((start, line.0)),
                    }
                }
            }
        }
        out
    }
    /// Where each Org page-property line stands, as whole lines: a
    /// directive's line, a head drawer from `:PROPERTIES:` to `:END:` (an
    /// empty one too), in source order. The text a page header owns; every
    /// other non-blank byte before the first headline is prose. O(entries).
    pub fn org_page_property_spans(&self, raw: &str) -> Vec<Range> {
        let mut spans: Vec<Range> = (self.page_properties())
            .map(|p| {
                self.property_regions
                    .get(p.region)
                    .copied()
                    .unwrap_or(p.line)
            })
            .chain(self.find_org_preamble_property_region(raw))
            .collect();
        spans.sort_by_key(|r| (r.0, r.1));
        spans.dedup();
        spans
    }
    /// The Org half of the page-header rule (`mapped_regions`): OG reads
    /// every directive as a page property and the text around them as the
    /// pre-block's body (graph-parser `mldoc.cljc collect-page-properties`,
    /// `block.cljs with-pre-block-if-exists`). Tine shows a page as a header
    /// plus one prose row, so the prose runs from the first text line before
    /// the first headline to the last one, and a directive or drawer between
    /// two text lines is prose, never a page property (a named difference).
    /// O(bytes before the first headline + entries).
    pub(super) fn demote_org_properties_inside_text(&mut self, raw: &str, blocks: &[Block]) {
        let end = blocks
            .iter()
            .find_map(|b| match b {
                Block::Bullet { span, .. } | Block::Heading { span, .. } => Some(span),
                _ => None,
            })
            .and_then(|span| raw_range(raw, span, Coords::of(raw, true)))
            .map_or(raw.len(), |r| r.0);
        let (mut first, mut last, mut at) = (None, None, 0);
        for span in self
            .org_page_property_spans(raw)
            .iter()
            .chain([&Range(end, end)])
        {
            let gap = &raw[at.min(span.0)..span.0.min(end)];
            if let Some(i) = gap.find(|c: char| !c.is_whitespace() && c != '\u{feff}') {
                first = first.or(Some(at + i));
                last = Some(at + gap.trim_end().len());
            }
            at = at.max(span.1);
        }
        let (Some(first), Some(last)) = (first, last) else {
            return;
        };
        for p in &mut self.properties {
            let line = self
                .property_regions
                .get(p.region)
                .copied()
                .unwrap_or(p.line);
            if (p.primary || p.directive) && first < line.0 && line.0 < last {
                p.primary = false;
                p.directive = false;
            }
        }
    }
    /// Native Org preamble container after leading physical trivia, including
    /// an empty one. Uses parsed ownership; O(raw bytes), no I/O.
    pub fn find_org_preamble_property_region(&self, raw: &str) -> Option<Range> {
        let (open, close) = self.property_drawer_boundaries(raw).first().copied()?;
        if !raw[..open]
            .bytes()
            .all(|b| matches!(b, b' ' | b'\t' | b'\x0c' | b'\x1a' | b'\r' | b'\n'))
        {
            return None;
        }
        self.property_regions
            .iter()
            .copied()
            .find(|r| r.0 <= open && close < r.1)
    }
}
