//! Ownership verification for opaque editor metadata restoration (GH #741).
//! Shared by property reattachment and drawer insertion, in release too.
use super::{parse, BlockRegions};

impl BlockRegions {
    pub(crate) fn insertion_preserves_metadata(
        &self,
        raw: &str,
        org: bool,
        inserted: &str,
        candidate: &str,
    ) -> bool {
        let payload = parse(inserted, org);
        let parsed = parse(candidate, org);
        if payload.quarantined
            || parsed.quarantined
            || parsed.id.as_ref().map(|id| &id.value)
                != self.id.as_ref().or(payload.id.as_ref()).map(|id| &id.value)
        {
            return false;
        }
        let properties = |r: &BlockRegions| {
            let mut values: Vec<_> = r
                .properties
                .iter()
                .map(|p| (p.key.clone(), p.value.clone(), p.primary))
                .collect();
            values.sort();
            values
        };
        let mut expected = properties(self);
        expected.extend(properties(&payload));
        expected.sort();
        let drawers = |r: &BlockRegions, text: &str| {
            r.drawers
                .iter()
                .map(|d| {
                    (
                        d.name.to_ascii_lowercase(),
                        d.range
                            .slice(text)
                            .trim_end_matches(['\r', '\n'])
                            .to_string(),
                    )
                })
                .collect::<Vec<_>>()
        };
        let mut expected_drawers = drawers(self, raw);
        expected_drawers.extend(drawers(&payload, inserted));
        expected_drawers.sort();
        let mut actual_drawers = drawers(&parsed, candidate);
        actual_drawers.sort();
        properties(&parsed) == expected && actual_drawers == expected_drawers
    }
}
