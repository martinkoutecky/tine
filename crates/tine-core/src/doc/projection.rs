//! One block parse builds the held projection and, when requested, file-local
//! rename evidence. Only the ordinary projection enters the block's memo.

use super::*;

impl DocBlock {
    /// Build the ordinary projection and, only for this caller, native slots.
    /// A warm block needs a fresh AST; a fresh block shares its one parse.
    pub(crate) fn read_for_rename(&self) -> crate::reference_evidence::RenameSource {
        let mut source = crate::reference_evidence::RenameSource::default();
        if self.proj.get().is_some() {
            let parsed = crate::render::parse_projection(&self.raw, self.is_org);
            let mut projection = crate::reference_evidence::ReferenceSourceProjection {
                rewrite_targets: Some(Vec::new()),
                ..Default::default()
            };
            crate::reference_evidence::project_into(
                &mut projection,
                &self.raw,
                self.is_org,
                &parsed.blocks,
                &self.projection().regions,
                self.property_ref_config
                    .as_deref()
                    .unwrap_or_else(|| crate::query::atom::ParseConfig::default_ref()),
                Some(&mut source.explicit),
            );
            source.targets = projection.rewrite_targets.unwrap_or_default();
        } else {
            self.projection_with_slots(Some(&mut source));
        }
        source
    }

    pub(super) fn projection_with_slots(
        &self,
        slots: Option<&mut crate::reference_evidence::RenameSource>,
    ) -> &BlockProjection {
        self.proj.get_or_init(|| {
            // Test-only fault: a block projection (the lsdoc parse every index
            // reads) that panics, for the load's one-bad-file isolation tests.
            #[cfg(feature = "test-faults")]
            if self.raw.contains(TEST_PROJECTION_PANIC_SENTINEL) {
                panic!("deterministic test sentinel for a block projection panic");
            }
            // ONE lsdoc parse of the block body yields every header facet (marker,
            // heading level, properties, scheduled/deadline) AND the visible text —
            // so `doc.rs`, the TS `blockView`, and lsdoc can no longer disagree
            // about a block's grammar.
            // ONE lsdoc parse yields AST, facets and refs; region ownership also
            // recovers an author swallowed by a final quote (GH #895).
            let proj = crate::render::parse_projection(&self.raw, self.is_org);
            let (marker, priority, heading_level, mut properties) = header_facets(&proj.blocks);
            let (scheduled, deadline) = planning_dates(&proj.blocks, &self.raw);
            let tags = tags_from_blocks(&proj.blocks);
            let regions = crate::block_regions::from_blocks(&self.raw, self.is_org, &proj.blocks);
            crate::block_regions::extend_author_facet(&mut properties, &regions);
            let visible = regions
                .apply(&self.raw, self.is_org, crate::block_regions::Edit::Visible)
                .expect("parsed regions");
            let regions = shared_regions(regions);
            let visible_lower = crate::search_query::canonical_fold(&visible);
            let config = self
                .property_ref_config
                .as_deref()
                .unwrap_or_else(|| crate::query::atom::ParseConfig::default_ref());
            let reference_source = if let Some(slots) = slots {
                let mut projection = crate::reference_evidence::ReferenceSourceProjection {
                    rewrite_targets: Some(Vec::new()),
                    ..Default::default()
                };
                crate::reference_evidence::project_into(
                    &mut projection,
                    &self.raw,
                    self.is_org,
                    &proj.blocks,
                    &regions,
                    config,
                    Some(&mut slots.explicit),
                );
                slots.targets = projection.rewrite_targets.take().unwrap_or_default();
                projection
            } else {
                crate::reference_evidence::project_with_config(
                    &self.raw,
                    self.is_org,
                    &proj.blocks,
                    &regions,
                    config,
                )
            };
            // OG 0.10.9 with-page-refs: the parser's references outside
            // property values (its projection leaves those out), headline
            // tags, and every property value's members, which are
            // `property_refs`' answer as the reference walk projected it.
            let explicit = &reference_source.explicit;
            let directives = crate::reference_evidence::markdown_directives(
                &self.raw,
                self.is_org,
                &proj.blocks,
            );
            let mut applicable: Vec<_> = (regions.properties.iter())
                .filter(|p| p.applicable)
                .chain(&directives)
                .collect();
            if !directives.is_empty() {
                applicable.sort_by_key(|p| p.value_range.0);
            }
            let values = crate::reference_evidence::property_references_in(
                explicit.iter(),
                applicable.into_iter(),
                |r| r,
            );
            let tag_refs = explicit.iter().filter(|r| r.rule == "explicit_tag");
            let mut refs_page = proj.refs.page;
            let before = refs_page.len();
            refs_page.extend(
                values
                    .map(|(_, r)| r)
                    .chain(tag_refs)
                    .map(|r| r.name.clone()),
            );
            if refs_page.len() > before {
                refs_page.sort();
                refs_page.dedup();
            }
            // OG `with-page-refs` also puts the block's marker and priority in its
            // refs, so `[[A]]` / `[[TODO]]` queries find them. They stay out of
            // `refs_page`, the virtual-page list behind `[[` autocomplete: Tine
            // does not offer a page per marker or priority letter.
            let mut refs_norm: Vec<String> = refs_page
                .iter()
                .map(|r| crate::refs::normalize(r))
                .collect();
            for r in reference_source
                .explicit
                .iter()
                .filter(|r| r.rule == "explicit_task_marker" || r.rule == "explicit_priority")
            {
                let key = crate::refs::normalize(&r.name);
                if !refs_norm.contains(&key) {
                    refs_norm.push(key);
                }
            }
            // These lists never grow after publication. Boxed slices trim the
            // build's slack and avoid retaining a capacity word per list; serde
            // keeps the same sequence encoding, including in checkpoints.
            let plain_ranges = PlainRanges::from(reference_source.plain_ranges);
            let extra = ProjectionExtra {
                search: std::sync::OnceLock::new(),
                refs_norm: refs_norm.into_boxed_slice(),
                refs_page: refs_page.into_boxed_slice(),
                block_refs: proj.refs.block.into_boxed_slice(),
                marker: marker.map(String::into_boxed_str),
                priority: priority.map(String::into_boxed_str),
                heading_level,
                properties: properties.into_boxed_slice(),
                scheduled: scheduled.map(String::into_boxed_str),
                deadline: deadline.map(String::into_boxed_str),
                tags: tags.into_boxed_slice(),
                explicit: reference_source.explicit.into_boxed_slice(),
                withheld_ranges: reference_source.withheld_ranges.into_boxed_slice(),
            };
            let extra = (!extra.is_empty()).then(|| Box::new(extra));
            let visible_lower = (visible_lower != visible).then(|| visible_lower.into_boxed_str());
            let visible = (*visible != *self.raw).then(|| visible.into_boxed_str());
            BlockProjection {
                regions,
                texts: VisibleTexts::boxed(visible, visible_lower),
                visible_literal: std::sync::OnceLock::new(),
                plain_ranges,
                extra,
            }
        })
    }
}
