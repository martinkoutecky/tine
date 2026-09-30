use super::*;

impl ReadSnapshot {
    pub(super) fn build_page_icon_index(&self) -> HashMap<String, String> {
        let mut icons = HashMap::new();
        let mut real = std::collections::HashSet::new();
        let mut aliases = Vec::new();
        for (entry, doc) in self.pages.iter() {
            #[cfg(feature = "test-faults")]
            crate::cost_counters::icon_page_probe();
            for alias in crate::query::document_aliases(doc) {
                aliases.push((entry.path.clone(), alias, entry.name.clone()));
            }
            if entry.kind != PageKind::Page {
                continue;
            }
            let key = tine_core::refs::page_key(&entry.name);
            real.insert(key.clone());
            if let Some(icon) = pre_block_icon(entry, doc) {
                icons.entry(key).or_insert(icon);
            }
        }
        aliases.sort_by(|a, b| a.0.cmp(&b.0).then_with(|| a.1.cmp(&b.1)));
        for (_, alias, owner) in aliases {
            if real.contains(&alias) {
                continue;
            }
            if let Some(icon) = icons.get(&tine_core::refs::page_key(&owner)).cloned() {
                icons.entry(alias).or_insert(icon);
            }
        }
        icons
    }

    pub(crate) fn page_icons(&self, names: &[String]) -> HashMap<String, String> {
        let icons = self.icon_index.get().expect("icons built at publication");
        names
            .iter()
            .filter_map(|name| {
                icons
                    .get(&tine_core::refs::page_key(name))
                    .map(|icon| (name.clone(), icon.clone()))
            })
            .collect()
    }
}

/// The first nonblank parser-owned icon property for this file's format.
/// O(preblock bytes + AST nodes); no graph scan or I/O.
pub(super) fn pre_block_icon(entry: &PageEntry, doc: &Document) -> Option<String> {
    let org = Format::from_path(&entry.path) == Format::Org;
    crate::query::page_properties::page_property_lines(doc.pre_block.as_deref()?, org)
        .into_iter()
        .find(|(key, value)| key.eq_ignore_ascii_case("icon") && !value.trim().is_empty())
        .map(|(_, value)| value)
}
