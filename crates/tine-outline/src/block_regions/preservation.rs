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
                // The parser folds duplicate keys without case and gives all
                // entries the first spelling in a folded property region.
                // Compare that same identity; values and ownership stay exact.
                .map(|p| (p.key.to_ascii_lowercase(), p.value.clone(), p.primary))
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gh741_ownership_guard_still_refuses_swallowed_metadata() {
        let id = "11111111-1111-4111-8111-111111111111";
        let properties = format!(":PROPERTIES:\n:id: {id}\n:END:");
        let drawer = ":LOGBOOK:\nCLOCK: [2026-10-21 Wed 08:54:45]\n:END:";
        let bad = format!(":PROPERTIES:\n{drawer}\n:id: {id}\n:END:");
        assert!(!parse(&properties, true).insertion_preserves_metadata(
            &properties,
            true,
            drawer,
            &bad
        ));
        for org in [false, true] {
            let quote = "> quoted";
            let bad = format!("{quote}\n{drawer}");
            assert!(!parse(quote, org).insertion_preserves_metadata(quote, org, drawer, &bad));
        }
    }
}
