use super::*;

/// Real page presence, icon-bearing page slots, and alias claimants are
/// persistent per-name trees. Publication changes only the edited page's rows;
/// requested names resolve from these captured trees, including ordered alias
/// inheritance. No graph walk is moved to the query path (I-12/I-25).
#[derive(Clone, Default, serde::Serialize, serde::Deserialize)]
pub(super) struct IconIndex {
    real: SharedMap<String, SharedMap<usize, ()>>,
    icons: SharedMap<String, SharedMap<usize, String>>,
    aliases: SharedMap<String, SharedMap<(PathBuf, String), String>>,
}
impl IconIndex {
    fn update(&mut self, slot: usize, entry: &PageEntry, doc: &Document, added: bool) {
        #[cfg(feature = "test-faults")]
        crate::cost_counters::icon_page_probe();
        let key = tine_core::refs::page_key(&entry.name);
        if entry.kind == PageKind::Page {
            let mut real = self.real.get(&key).cloned().unwrap_or_default();
            let mut icons = self.icons.get(&key).cloned().unwrap_or_default();
            if added {
                real.insert(slot, ());
                if let Some(icon) = pre_block_icon(entry, doc) {
                    icons.insert(slot, icon);
                }
            } else {
                real.remove(&slot);
                icons.remove(&slot);
            }
            if real.is_empty() {
                self.real.remove(&key);
            } else {
                self.real.insert(key.clone(), real);
            }
            if icons.is_empty() {
                self.icons.remove(&key);
            } else {
                self.icons.insert(key.clone(), icons);
            }
        }
        for alias in crate::query::document_aliases(doc) {
            let row = (entry.path.clone(), alias.clone());
            let mut owners = self.aliases.get(&alias).cloned().unwrap_or_default();
            if added {
                owners.insert(row, key.clone());
            } else {
                owners.remove(&row);
            }
            if owners.is_empty() {
                self.aliases.remove(&alias);
            } else {
                self.aliases.insert(alias, owners);
            }
        }
    }
    pub(super) fn capture(old: Option<(&Self, &Pages)>, pages: &Pages, paths: &[String]) -> Self {
        if let Some((old, before)) = old {
            let mut next = old.clone();
            for path in paths {
                let was = before
                    .positions
                    .get(path)
                    .and_then(|&slot| before.get(slot).map(|p| (slot, p)));
                let now = pages
                    .positions
                    .get(path)
                    .and_then(|&slot| pages.get(slot).map(|p| (slot, p)));
                if let (Some((_, (a, ad))), Some((_, (b, bd)))) = (was, now) {
                    if a.kind == b.kind
                        && a.name == b.name
                        && pre_block_icon(a, ad) == pre_block_icon(b, bd)
                        && crate::query::document_aliases(ad) == crate::query::document_aliases(bd)
                    {
                        continue;
                    }
                }
                if let Some((slot, (e, d))) = was {
                    next.update(slot, e, d, false);
                }
                if let Some((slot, (e, d))) = now {
                    next.update(slot, e, d, true);
                }
            }
            next
        } else {
            let mut next = Self::default();
            for (slot, (e, d)) in pages.slots() {
                next.update(slot, e, d, true);
            }
            next
        }
    }
    // Alias rows are applied in (path, alias) order. An alias inherits its
    // owner's icon only from earlier rows; decreasing bounds also stop cycles.
    fn get(&self, key: &str) -> Option<&String> {
        let real_icon = |key: &str| {
            self.icons
                .get(key)
                .and_then(|icons| icons.first())
                .map(|(_, icon)| icon)
        };
        if let Some(icon) = real_icon(key) {
            return Some(icon);
        }
        if self.real.contains_key(key) {
            return None;
        }
        let mut stack = vec![(None, self.aliases.get(key)?.iter())];
        let mut visited = std::collections::HashSet::new();
        while let Some((bound, rows)) = stack.last_mut() {
            let Some((row, owner)) = rows.next() else {
                stack.pop();
                continue;
            };
            if bound.is_some_and(|bound| row >= bound) {
                stack.pop();
                continue;
            }
            if let Some(icon) = real_icon(owner) {
                return Some(icon);
            }
            if self.real.contains_key(owner) || !visited.insert((owner.as_str(), row)) {
                continue;
            }
            if let Some(owners) = self.aliases.get(owner) {
                stack.push((Some(row), owners.iter()));
            }
        }
        None
    }
}
impl ReadSnapshot {
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
