//! Friendly-search text is independent of visible labels and exact query text.
use super::{BlockProjection, DocBlock};
use std::sync::OnceLock;

#[derive(Debug, Clone)]
pub(super) struct SearchProjection {
    // One allocation for original and canonical text; offset 0 shares identical
    // forms. Inline in the existing sparse extra, avoiding another box/lookup.
    storage: Box<str>,
    fold_offset: usize,
    literal: OnceLock<Option<Box<str>>>,
}

impl SearchProjection {
    fn new(mut text: String) -> Self {
        let folded = crate::search_query::canonical_fold(&text);
        let fold_offset = if folded == text {
            0
        } else {
            let offset = text.len();
            text.push_str(&folded);
            offset
        };
        Self {
            storage: text.into_boxed_str(),
            fold_offset,
            literal: OnceLock::new(),
        }
    }

    fn text(&self) -> &str {
        if self.fold_offset == 0 {
            &self.storage
        } else {
            &self.storage[..self.fold_offset]
        }
    }

    fn folded(&self, remove_accents: bool) -> &str {
        let canonical = &self.storage[self.fold_offset..];
        if remove_accents {
            canonical
        } else {
            self.literal
                .get_or_init(|| {
                    let literal = crate::search_query::literal_fold(self.text());
                    (literal != canonical).then(|| literal.into_boxed_str())
                })
                .as_deref()
                .unwrap_or(canonical)
        }
    }
}

fn hidden(key: &str, configured: &[String]) -> bool {
    let key = tine_outline::property_visibility::normalize(key);
    // OG 6e7afa8eb: util/property.cljs built-in-properties aliases the HIDDEN
    // set, not editable-built-in-properties. Tine additionally hides tine.*.
    key.starts_with("tine.")
        || tine_outline::property_visibility::BUILTIN_HIDDEN.contains(&key.as_str())
        || configured
            .iter()
            .any(|k| tine_outline::property_visibility::normalize(k.trim_start_matches(':')) == key)
}

impl DocBlock {
    fn search_projection(&self) -> Option<&SearchProjection> {
        self.search_projection_in(self.projection())
    }

    fn search_projection_in<'a>(
        &'a self,
        projection: &'a BlockProjection,
    ) -> Option<&'a SearchProjection> {
        let extra = projection.extra()?;
        extra
            .search
            .get_or_init(|| {
                if extra.properties.is_empty()
                    || !extra.properties.iter().any(|(key, _)| !hidden(key, &[]))
                {
                    return None;
                }
                let text = self.search_text_excluding(&[]);
                (text != self.visible_text()).then(|| SearchProjection::new(text))
            })
            .as_ref()
    }

    /// Friendly-search text, including user property keys and values. Hidden
    /// built-ins and Tine metadata stay out; literal property-shaped code stays.
    /// Cached only in memory, on blocks already carrying sparse facets.
    pub fn search_text(&self) -> &str {
        self.search_projection()
            .map_or_else(|| self.visible_text(), SearchProjection::text)
    }

    /// Friendly-search text folded under the graph's accent policy, once per body.
    pub fn search_folded(&self, remove_accents: bool) -> &str {
        let Some(p) = self.search_projection() else {
            return self.visible_folded(remove_accents);
        };
        p.folded(remove_accents)
    }

    /// Text and fold from one projection lookup, for whole-graph scan loops.
    /// Blocks without user properties borrow the original visible storage.
    pub fn search_text_pair(&self, remove_accents: bool) -> (&str, &str) {
        let projection = self.projection();
        match self.search_projection_in(projection) {
            Some(search) => (search.text(), search.folded(remove_accents)),
            None => (
                projection.visible_text(&self.raw),
                projection.visible_folded(&self.raw, remove_accents),
            ),
        }
    }

    /// Whether graph-configured hidden keys need a different search projection.
    pub fn has_hidden_search_properties(&self, configured: &[String]) -> bool {
        !configured.is_empty()
            && self
                .projection()
                .properties()
                .iter()
                .any(|(key, _)| !hidden(key, &[]) && hidden(key, configured))
    }

    /// Build text for a graph with additional hidden keys. Parser-owned property
    /// lines retain spelling and syntax; drawer delimiters are never search terms.
    pub fn search_text_excluding(&self, configured: &[String]) -> String {
        let mut text = self.visible_text().to_owned();
        for property in self
            .projection()
            .regions
            .properties
            .iter()
            .filter(|p| p.primary)
        {
            if hidden(&property.key, configured) {
                continue;
            }
            if !text.is_empty() {
                text.push('\n');
            }
            text.push_str(property.line.slice(self.raw()).trim());
        }
        text
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn search_keeps_visible_text_and_checkpoint_projection_unchanged() {
        let mut block = DocBlock::new("Body\nreviewer:: Café\nid:: private\ntine.view:: table");
        let persisted = serde_json::to_vec(block.projection()).unwrap();
        assert_eq!(block.search_text(), "Body\nreviewer:: Café");
        assert_eq!(block.search_folded(true), "body\nreviewer:: cafe");
        assert_eq!(block.search_folded(false), "body\nreviewer:: café");
        assert_eq!(block.visible_text(), "Body");
        assert_eq!(block.visible_folded(true), "body");
        assert_eq!(serde_json::to_vec(block.projection()).unwrap(), persisted);
        block.set_raw("Body\nreviewer:: Updated");
        assert_eq!(block.search_text(), "Body\nreviewer:: Updated");
        assert!(block.search_folded(true).contains("updated"));
    }

    #[test]
    fn search_hides_og_registered_builtins_and_keeps_editable_ones() {
        for key in tine_outline::property_visibility::BUILTIN_HIDDEN
            .iter()
            .chain(
                [
                    "background_color",
                    "created_at",
                    "last_modified_at",
                    "tine.custom",
                ]
                .iter(),
            )
        {
            let block = DocBlock::new(format!("Body\n{key}:: Secret"));
            assert_eq!(block.search_text(), "Body", "{key}");
        }
        for key in [
            "title",
            "template",
            "tags",
            "alias",
            "logseq.color",
            "hl-user-note",
        ] {
            let block = DocBlock::new(format!("Body\n{key}:: Visible"));
            assert!(block.search_text().contains("Visible"), "{key}");
        }
        let block = DocBlock::new("Body\nreviewer:: Secret\nowner:: Avery");
        assert_eq!(
            block.search_text_excluding(&["reviewer".into()]),
            "Body\nowner:: Avery"
        );
    }

    #[test]
    fn property_shaped_literal_code_remains_searchable() {
        let block = DocBlock::new("```\nid:: literal\n```");
        assert!(block.search_text().contains("id:: literal"));
    }
}
