use super::*;

fn is_properties_drawer(raw: &str, r: &Range) -> bool {
    r.slice(raw)
        .lines()
        .next()
        .is_some_and(|line| is_property_drawer_marker(line, true))
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
            for line in line_ranges(raw, *region) {
                if is_properties_drawer(raw, &line) {
                    open = Some(line.0);
                } else if is_property_drawer_marker(
                    line.slice(raw).trim_end_matches(['\r', '\n']),
                    false,
                ) {
                    if let Some(start) = open.take() {
                        out.push((start, line.0));
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
            .and_then(|span| raw_range(raw, span, true))
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
