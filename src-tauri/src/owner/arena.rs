//! A page as the outline kernel takes it (`tine_outline::outline`), and back:
//! the converter the command line's structural verbs run through (design §3).
//! Keys are positional (`b0`, `b1`, … in document order), assigned per read;
//! the page's pre-block stays the pre-block, so no header root is ever made
//! (the window marks one only while you edit a page header). Saves derive
//! block identity from the file, so a rebuilt `PageDto` carries raw text and
//! children, plus each surviving block's other fields as read.

use std::collections::HashMap;

use tine_core::model::{BlockDto, Format, PageDto};
use tine_core::outline_kernel::{Block, Change, Format as OutlineFormat, Key, OutlinePage};

/// A page in arena form, with each key's path in the page read it came from.
pub(crate) struct Arena<'a> {
    pub(crate) page: OutlinePage,
    pub(crate) paths: HashMap<Key, Vec<usize>>,
    originals: HashMap<Key, &'a BlockDto>,
}

impl<'a> Arena<'a> {
    pub(crate) fn of(doc: &'a PageDto) -> Self {
        let mut arena = Arena {
            page: OutlinePage {
                name: doc.name.clone(),
                format: match doc.format {
                    Format::Md => OutlineFormat::Md,
                    Format::Org => OutlineFormat::Org,
                },
                pre_block: doc.pre_block.clone(),
                roots: Vec::new(),
                blocks: HashMap::new(),
                header_root: None,
                read_only: doc.read_only || doc.guide,
                frozen: false,
            },
            paths: HashMap::new(),
            originals: HashMap::new(),
        };
        let mut path = Vec::new();
        arena.page.roots = arena.add(&doc.blocks, &mut path);
        arena
    }

    fn add(&mut self, blocks: &'a [BlockDto], path: &mut Vec<usize>) -> Vec<Key> {
        let mut keys = Vec::with_capacity(blocks.len());
        for (index, block) in blocks.iter().enumerate() {
            path.push(index);
            let key = format!("b{}", self.paths.len());
            self.paths.insert(key.clone(), path.clone());
            let children = self.add(&block.children, path);
            self.page.blocks.insert(
                key.clone(),
                Block {
                    raw: block.raw.clone(),
                    children,
                },
            );
            self.originals.insert(key.clone(), block);
            keys.push(key);
            path.pop();
        }
        keys
    }

    /// The key of the block at `path`.
    pub(crate) fn key_at(&self, path: &[usize]) -> Option<Key> {
        self.paths
            .iter()
            .find(|(_, p)| p.as_slice() == path)
            .map(|(key, _)| key.clone())
    }

    /// Apply the kernel's change for this page and rebuild the `PageDto`, with
    /// the new paths of `keys` (blocks the reply names).
    pub(crate) fn apply(
        mut self,
        doc: &PageDto,
        change: &Change,
        keys: &[Key],
    ) -> (PageDto, Vec<Vec<usize>>) {
        let name = self.page.name.clone();
        for patch in change.pages.iter().filter(|p| p.name == name) {
            patch.apply_to(&mut self.page);
        }
        let mut paths = HashMap::new();
        let mut path = Vec::new();
        let roots = self.page.roots.clone();
        let blocks = self.build(&roots, &mut path, &mut paths);
        let changed = keys.iter().filter_map(|k| paths.get(k).cloned()).collect();
        (page_with_blocks(doc, blocks), changed)
    }

    fn build(
        &self,
        keys: &[Key],
        path: &mut Vec<usize>,
        paths: &mut HashMap<Key, Vec<usize>>,
    ) -> Vec<BlockDto> {
        keys.iter()
            .enumerate()
            .filter_map(|(index, key)| {
                let block = self.page.blocks.get(key)?;
                path.push(index);
                paths.insert(key.clone(), path.clone());
                let children = self.build(&block.children, path, paths);
                path.pop();
                let mut dto = self
                    .originals
                    .get(key)
                    .map(|original| block_metadata(original))
                    .unwrap_or_default();
                dto.raw = block.raw.clone();
                dto.children = children;
                Some(dto)
            })
            .collect()
    }
}

/// Copy only one block's metadata; descendants and old raw are deliberately
/// omitted. Exhaustive destructuring makes new DTO fields a compiler decision.
fn block_metadata(block: &BlockDto) -> BlockDto {
    let BlockDto {
        id,
        has_id,
        raw: _,
        collapsed,
        children: _,
        breadcrumb,
        page_property,
        marker,
        priority,
        heading_level,
        scheduled,
        deadline,
        tags,
        properties,
    } = block;
    BlockDto {
        id: id.clone(),
        has_id: *has_id,
        raw: String::new(),
        collapsed: *collapsed,
        children: Vec::new(),
        breadcrumb: breadcrumb.clone(),
        page_property: *page_property,
        marker: marker.clone(),
        priority: priority.clone(),
        heading_level: *heading_level,
        scheduled: scheduled.clone(),
        deadline: deadline.clone(),
        tags: tags.clone(),
        properties: properties.clone(),
    }
}

fn page_with_blocks(doc: &PageDto, blocks: Vec<BlockDto>) -> PageDto {
    let PageDto {
        name,
        kind,
        title,
        pre_block,
        blocks: _,
        rev,
        format,
        read_only,
        guide,
    } = doc;
    PageDto {
        name: name.clone(),
        kind: *kind,
        title: title.clone(),
        pre_block: pre_block.clone(),
        blocks,
        rev: rev.clone(),
        format: *format,
        read_only: *read_only,
        guide: *guide,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn block(raw: &str, children: Vec<BlockDto>) -> BlockDto {
        BlockDto {
            raw: raw.into(),
            children,
            ..Default::default()
        }
    }

    #[test]
    fn deep_arena_borrows_originals_and_preserves_all_metadata() {
        let mut tree = Vec::new();
        for depth in 0..64 {
            let mut node = block(&format!("node {depth}"), tree);
            node.id = format!("id-{depth}");
            node.has_id = Some(true);
            node.marker = Some("TODO".into());
            node.tags = vec!["tag".into()];
            node.properties = vec![("key".into(), "value".into())];
            tree = vec![node];
        }
        let doc = PageDto {
            name: "deep".into(),
            kind: tine_core::model::PageKind::Page,
            title: "Deep title".into(),
            blocks: tree,
            pre_block: Some("alias:: deeper".into()),
            rev: Some("rev".into()),
            format: Format::Md,
            read_only: false,
            guide: false,
        };
        let arena = Arena::of(&doc);
        assert_eq!(arena.originals.len(), 64);
        assert!(std::ptr::eq(
            *arena.originals.get("b0").unwrap(),
            &doc.blocks[0]
        ));
        let (rebuilt, _) = arena.apply(
            &doc,
            &Change {
                pages: vec![],
                focus: None,
                notices: vec![],
                keys_used: 0,
                transfers: vec![],
            },
            &[],
        );
        assert_eq!(
            serde_json::to_value(rebuilt).unwrap(),
            serde_json::to_value(doc).unwrap()
        );
    }

    #[test]
    fn a_page_round_trips_through_the_arena_and_a_move_lands() {
        let doc = PageDto {
            name: "p".into(),
            blocks: vec![block("a", vec![block("a1", vec![])]), block("b", vec![])],
            kind: tine_core::model::PageKind::Page,
            title: "p".into(),
            pre_block: None,
            rev: None,
            format: Format::Md,
            read_only: false,
            guide: false,
        };
        let arena = Arena::of(&doc);
        let b = arena.key_at(&[1]).unwrap();
        let a1 = arena.key_at(&[0, 0]).unwrap();
        let unchanged = Arena::of(&doc).apply(
            &doc,
            &Change {
                pages: vec![],
                focus: None,
                notices: vec![],
                keys_used: 0,
                transfers: vec![],
            },
            &[],
        );
        assert_eq!(
            serde_json::to_value(&unchanged.0).unwrap(),
            serde_json::to_value(&doc).unwrap()
        );

        let verb = tine_core::outline_kernel::Verb::MoveTo {
            keys: vec![b.clone()],
            to: tine_core::outline_kernel::Place::After { key: a1.clone() },
            inherit_from: Some(a1),
        };
        let change =
            tine_core::outline_kernel::apply(&[arena.page.clone()], &verb, &Default::default())
                .unwrap();
        let (moved, paths) = arena.apply(&doc, &change, &[b]);
        assert_eq!(moved.blocks.len(), 1);
        assert_eq!(moved.blocks[0].children[1].raw, "b");
        assert_eq!(paths, vec![vec![0, 1]]);
    }
}
