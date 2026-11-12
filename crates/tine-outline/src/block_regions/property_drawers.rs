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
