//! Template names in page-slot/preorder order, duplicates included. Derived at
//! publication from the core template visitor; patched only for changed pages.
//! Only template-bearing pages occupy the persistent tree. No persisted state.
use super::*;

#[derive(Clone, Default)]
pub(super) struct TemplateNames(SharedMap<usize, Box<[String]>>);

impl TemplateNames {
    pub(super) fn capture(
        old: Option<&ReadSnapshot>,
        pages: &Arc<Pages>,
        paths: &[String],
    ) -> Self {
        if let Some(old) = old.filter(|old| !paths.is_empty() || Arc::ptr_eq(&old.pages, pages)) {
            let mut next = old.template_names.clone();
            let before = &old.pages;
            for path in paths {
                if let Some(slot) = before.positions.get(path) {
                    next.0.remove(slot);
                }
                if let Some(&slot) = pages.positions.get(path) {
                    if let Some((_, doc)) = pages.get(slot) {
                        if let Some(names) = Self::names(doc) {
                            next.0.insert(slot, names);
                        }
                    }
                }
            }
            return next;
        }
        Self(SharedMap::from_sorted_unique(
            pages
                .slots()
                .filter_map(|(slot, (_, doc))| Self::names(doc).map(|names| (slot, names)))
                .collect(),
        ))
    }

    fn names(doc: &Document) -> Option<Box<[String]>> {
        let mut names = Vec::new();
        tine_core::template::template_definitions_in_document(doc, &mut |name, _| names.push(name));
        (!names.is_empty()).then(|| names.into())
    }
}

impl ReadSnapshot {
    pub(crate) fn template_names(&self) -> Vec<String> {
        self.template_names
            .0
            .iter()
            .flat_map(|(_, names)| names.iter().cloned())
            .collect()
    }
}

#[cfg(test)]
#[path = "template_names_tests.rs"]
mod tests;
