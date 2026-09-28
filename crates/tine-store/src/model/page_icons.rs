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
            if let Some(icon) = doc.pre_block.as_deref().and_then(pre_block_icon) {
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

/// A page's `icon::` property value from its pre-block, handling markdown
/// (`icon:: 🏁`), org property drawers (`:icon: 🏁`) and org `#+ICON:` directives.
/// None if absent or blank.
pub(super) fn pre_block_icon(pre: &str) -> Option<String> {
    for line in pre.lines() {
        // Markdown `icon:: value` (single shared parser; needs the `::`).
        if let Some((k, v)) = tine_core::doc::parse_property_line(line) {
            let v = v.trim();
            if k.eq_ignore_ascii_case("icon") && !v.is_empty() {
                return Some(v.to_string());
            }
        }
        let t = line.trim();
        // Org property drawer `:icon: value` or directive `#+ICON: value`.
        for stripped in [t.strip_prefix(':'), t.strip_prefix("#+")]
            .into_iter()
            .flatten()
        {
            if let Some(idx) = stripped.find(':') {
                let (k, v) = (&stripped[..idx], stripped[idx + 1..].trim());
                if k.eq_ignore_ascii_case("icon") && !v.is_empty() {
                    return Some(v.to_string());
                }
            }
        }
    }
    None
}
