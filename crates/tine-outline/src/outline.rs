//! The outline kernel: Tine's outline edit rules as pure functions over a page
//! arena (outline-kernel DESIGN §3–§5).
//!
//! A verb reads the pages it is given, never the graph, a clock or randomness,
//! and returns a [`Change`] (per-page patches, the focus, notices) or a closed
//! [`Reason`] it refused for. The window (through wasm), the page host and the
//! CLI call the same verbs; none of them is known here. A preflight is "call the
//! verb and discard the `Change`".
//!
//! Each verb transcribes the window's function named in its doc comment (line
//! numbers at `b217e8727`), and OG's where one exists. Cost: O(blocks of the
//! touched pages) to derive parents, plus O(affected blocks) of edit work.

use crate::block_regions;
use crate::edit_kind::EditKind;
use crate::editor_text::{self, js_blank, HidePolicy};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap};

mod header;
use header::header_root_kept;

/// The deepest outline the kernel builds (the window's `OUTLINE_MAX_DEPTH`,
/// the store's `PARSE_INPUT_MAX_DEPTH`): a root is depth 0, so the deepest
/// block sits at depth 127.
pub const OUTLINE_MAX_DEPTH: usize = 128;

/// The block-level list property (OG `:logseq.order-list-type`).
pub const ORDER_LIST_KEY: &str = "logseq.order-list-type";

/// A block key: the window's runtime id. Durable `id::` values are text.
pub type Key = String;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Format {
    Md,
    Org,
}

impl Format {
    pub fn is_org(self) -> bool {
        self == Format::Org
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Block {
    pub raw: String,
    #[serde(default)]
    pub children: Vec<Key>,
}

/// One page as the window holds it: an arena keyed by block key. Parents are
/// derived, so the tree has one encoding. `blocks` may hold only part of the
/// page (DESIGN §5.3, [`Verb::reads`]) if it is closed under parents: every
/// block is a root or a child of a block given. A verb that needs a block not
/// given refuses with [`Reason::Incomplete`]; it never reads it as absent.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OutlinePage {
    pub name: String,
    pub format: Format,
    #[serde(default)]
    pub pre_block: Option<String>,
    pub roots: Vec<Key>,
    pub blocks: HashMap<Key, Block>,
    /// The block materialized from an unbulleted Markdown page header, wherever
    /// it now sits: never an outline parent (GH #638).
    #[serde(default)]
    pub header_root: Option<Key>,
    /// An Org page Tine cannot round-trip, or a bundled Guide page.
    #[serde(default)]
    pub read_only: bool,
    /// A page whose text the host holds apart (a transfer or a graph rewrite).
    #[serde(default)]
    pub frozen: bool,
}

/// Fresh keys for blocks a verb creates: `prefix` + counter from `next`. The
/// window passes its `freshId` scheme; the kernel reports how many it used.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct KeySource {
    pub prefix: String,
    pub next: u64,
}

/// Why a verb did nothing. Closed: every caller branches on it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Reason {
    /// The result would nest deeper than [`OUTLINE_MAX_DEPTH`].
    DepthCap,
    /// The move would put a block inside its own subtree.
    Cycle,
    ReadOnly,
    Frozen,
    /// Merge refuses blocks on two pages.
    CrossPageMerge,
    /// The page-header root is never an outline parent.
    HeaderRoot,
    /// No outline step exists (first sibling for indent, a root for outdent).
    NoOutlineStep,
    /// A named key or position is not in the pages given.
    Incomplete,
    /// A block's parse is quarantined, so its metadata cannot be edited.
    Unparseable,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Focus {
    pub key: Key,
    /// A raw byte offset in the block's new text; `None` keeps the caret.
    pub raw_offset: Option<usize>,
}

/// What happened, beyond the patches. The window derives toasts from these.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Notice {
    /// Merge kept the absorbed block's `id::` on the survivor.
    IdTransferred { from: Key, to: Key },
}

/// A block's state after the change. `collapsed` is set only where the verb
/// decides the view's collapse (a created block; indent's new parent).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Upsert {
    pub key: Key,
    pub raw: String,
    pub children: Vec<Key>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub collapsed: Option<bool>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PagePatch {
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub roots: Option<Vec<Key>>,
    pub upserts: Vec<Upsert>,
    pub deleted: Vec<Key>,
    pub kinds: Vec<EditKind>,
}

impl PagePatch {
    /// Apply this patch to the page it was computed from.
    pub fn apply_to(&self, page: &mut OutlinePage) {
        for key in &self.deleted {
            page.blocks.remove(key);
        }
        for u in &self.upserts {
            page.blocks.insert(
                u.key.clone(),
                Block {
                    raw: u.raw.clone(),
                    children: u.children.clone(),
                },
            );
        }
        if let Some(roots) = &self.roots {
            page.roots = roots.clone();
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Change {
    pub pages: Vec<PagePatch>,
    pub focus: Option<Focus>,
    pub notices: Vec<Notice>,
    /// Keys taken from the [`KeySource`].
    pub keys_used: u64,
    /// `(source, destination)` pages between which blocks moved: the page
    /// host's move transfers (STEP3 §8).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub transfers: Vec<(String, String)>,
}

/// Where [`Verb::MoveTo`] puts the moved blocks.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "at", rename_all = "snake_case")]
pub enum Place {
    Before {
        key: Key,
    },
    After {
        key: Key,
    },
    /// After `key`'s last child (OG's nested drop).
    LastChild {
        key: Key,
    },
    /// Position `index` of `parent`'s children, or of `page`'s roots when
    /// `parent` is `None`. `index` counts the list as it stands before the move.
    Slot {
        #[serde(default)]
        parent: Option<Key>,
        page: String,
        index: usize,
    },
}

/// The verbs. Offsets are raw UTF-8 bytes in the block's current text.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "verb", rename_all = "snake_case")]
pub enum Verb {
    /// Enter at `raw_offset`. `children_visible`: the block's children are
    /// shown in this view; `keep_start_in_scope`: a split at the head of a
    /// zoom root keeps the new block inside the zoom.
    Split {
        key: Key,
        raw_offset: usize,
        #[serde(default)]
        force_child: bool,
        #[serde(default)]
        children_visible: bool,
        #[serde(default)]
        keep_start_in_scope: bool,
    },
    /// Append `absorbed` to `survivor` (Backspace at start, Delete at end).
    /// `absorbed_referenced` is reserved for the OG parity call (DESIGN §4.3).
    Merge {
        survivor: Key,
        absorbed: Key,
        #[serde(default)]
        absorbed_referenced: bool,
    },
    /// Tab on one block or a selection: `keys` (top-level, document order)
    /// become the last children of the first key's previous sibling.
    Indent { keys: Vec<Key> },
    /// Shift+Tab: `keys` land after the first key's parent. Traditional
    /// outdenting (one key, `logical` false) takes the following siblings along.
    Outdent {
        keys: Vec<Key>,
        #[serde(default)]
        logical: bool,
    },
    /// Remove blocks with their subtrees, on any of the pages given.
    Delete { keys: Vec<Key> },
    /// Move blocks with their subtrees to `to`, across pages too. A moved block
    /// without its own list type takes `inherit_from`'s (the drop target).
    MoveTo {
        keys: Vec<Key>,
        to: Place,
        #[serde(default)]
        inherit_from: Option<Key>,
    },
    /// Mod+Up/Down. One key takes one OG outline step, crossing a child-list
    /// edge into the adjacent parent sibling; `within_siblings` moves each key
    /// one slot in its own sibling list instead (a selection).
    MoveUpDown {
        keys: Vec<Key>,
        up: bool,
        #[serde(default)]
        within_siblings: bool,
    },
    /// A new empty block at child position `at` of `parent`.
    InsertEmptyChild { parent: Key, at: usize },
}

/// A block a verb reads (DESIGN §5.3), and whether it reads its subtree.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Read {
    pub key: Key,
    pub subtree: bool,
}

impl Verb {
    /// The verb's read set, for a caller that sends pages partly
    /// ([`OutlinePage`]): each read key's block, its ancestors, the siblings
    /// just before and after it and each ancestor, every page's roots, and
    /// the whole subtree where `subtree` is set. A verb given less refuses with
    /// [`Reason::Incomplete`].
    pub fn reads(&self) -> Vec<Read> {
        let read = |key: &Key, subtree: bool| Read {
            key: key.clone(),
            subtree,
        };
        // Blocks that change depth or page, or are deleted, are read whole.
        let whole = |keys: &[Key]| keys.iter().map(|k| read(k, true)).collect::<Vec<_>>();
        match self {
            Verb::Split { key, .. } => vec![read(key, false)],
            Verb::Merge {
                survivor, absorbed, ..
            } => vec![read(survivor, false), read(absorbed, true)],
            Verb::Indent { keys } | Verb::Outdent { keys, .. } | Verb::Delete { keys } => {
                whole(keys)
            }
            Verb::MoveTo {
                keys,
                to,
                inherit_from,
            } => {
                let mut reads = whole(keys);
                match to {
                    Place::Before { key } | Place::After { key } | Place::LastChild { key } => {
                        reads.push(read(key, false))
                    }
                    Place::Slot {
                        parent: Some(p), ..
                    } => reads.push(read(p, false)),
                    Place::Slot { parent: None, .. } => {}
                }
                reads.extend(inherit_from.iter().map(|k| read(k, false)));
                reads
            }
            Verb::MoveUpDown { keys, .. } => keys.iter().map(|k| read(k, false)).collect(),
            Verb::InsertEmptyChild { parent, .. } => vec![read(parent, false)],
        }
    }
}

/// Run one verb over the pages that hold every block it names.
pub fn apply(pages: &[OutlinePage], verb: &Verb, keys: &KeySource) -> Result<Change, Reason> {
    let mut cx = Cx {
        pages,
        txs: Vec::new(),
        keys: keys.clone(),
        used: 0,
        focus: None,
        notices: Vec::new(),
        transfers: Vec::new(),
    };
    match verb {
        Verb::Split {
            key,
            raw_offset,
            force_child,
            children_visible,
            keep_start_in_scope,
        } => split(
            &mut cx,
            key,
            *raw_offset,
            *force_child,
            *children_visible,
            *keep_start_in_scope,
        )?,
        Verb::Merge {
            survivor, absorbed, ..
        } => merge(&mut cx, survivor, absorbed)?,
        Verb::Indent { keys } => indent(&mut cx, keys)?,
        Verb::Outdent { keys, logical } => outdent(&mut cx, keys, *logical)?,
        Verb::Delete { keys } => delete(&mut cx, keys)?,
        Verb::MoveTo {
            keys,
            to,
            inherit_from,
        } => move_to(&mut cx, keys, to, inherit_from.as_deref())?,
        Verb::MoveUpDown {
            keys,
            up,
            within_siblings,
        } => move_up_down(&mut cx, keys, *up, *within_siblings)?,
        Verb::InsertEmptyChild { parent, at } => insert_empty_child(&mut cx, parent, *at)?,
    }
    header_root_kept(&cx, verb)?;
    Ok(cx.finish())
}

// ---------------------------------------------------------------------------
// Transaction: a copy-on-write overlay per touched page.
// ---------------------------------------------------------------------------

struct PageTx<'a> {
    base: &'a OutlinePage,
    parent: HashMap<&'a str, Option<&'a str>>,
    /// Touched blocks; `None` = deleted.
    blocks: BTreeMap<Key, Option<Block>>,
    roots: Option<Vec<Key>>,
    collapsed: HashMap<Key, bool>,
    kinds: Vec<EditKind>,
    /// A key listed twice (as a root or a child), or a block without its
    /// parent: the arena is not a tree, and every verb on the page refuses
    /// with [`Reason::Incomplete`]. On a partial page only the blocks given
    /// are checked.
    malformed: bool,
}

impl<'a> PageTx<'a> {
    fn new(base: &'a OutlinePage) -> Self {
        let mut parent = HashMap::with_capacity(base.blocks.len());
        let mut malformed = false;
        for root in &base.roots {
            malformed |= parent.insert(root.as_str(), None).is_some();
        }
        for (key, block) in &base.blocks {
            for child in &block.children {
                malformed |= parent.insert(child.as_str(), Some(key.as_str())).is_some();
            }
        }
        // A block no given block lists would read as a root: a partial page
        // must carry each block's parent.
        malformed |= base
            .blocks
            .keys()
            .any(|key| !parent.contains_key(key.as_str()));
        PageTx {
            base,
            parent,
            blocks: BTreeMap::new(),
            roots: None,
            collapsed: HashMap::new(),
            kinds: Vec::new(),
            malformed,
        }
    }

    /// `key`'s base ancestors, nearest first. A parent cycle outside the roots
    /// ends the walk after one lap and yields `None`, so callers fail closed.
    fn ancestors(&self, key: &str) -> Vec<Option<&'a str>> {
        let mut out = Vec::new();
        let mut at = self.parent_of(key);
        while let Some(p) = at {
            if out.len() > self.base.blocks.len() {
                out.push(None);
                break;
            }
            out.push(Some(p));
            at = self.parent_of(p);
        }
        out
    }

    fn block(&self, key: &str) -> Option<&Block> {
        match self.blocks.get(key) {
            Some(b) => b.as_ref(),
            None => self.base.blocks.get(key),
        }
    }

    fn block_mut(&mut self, key: &str) -> Result<&mut Block, Reason> {
        if !self.blocks.contains_key(key) {
            let b = self
                .base
                .blocks
                .get(key)
                .cloned()
                .ok_or(Reason::Incomplete)?;
            self.blocks.insert(key.to_string(), Some(b));
        }
        self.blocks
            .get_mut(key)
            .and_then(Option::as_mut)
            .ok_or(Reason::Incomplete)
    }

    fn insert(&mut self, key: Key, block: Block) {
        self.blocks.insert(key, Some(block));
    }

    fn remove_subtree(&mut self, key: &str) -> Result<(), Reason> {
        for k in self.subtree(key)? {
            self.blocks.insert(k, None);
        }
        Ok(())
    }

    /// The base parent of `key` (`None` for a root). Verbs read parents before
    /// they mutate.
    fn parent_of(&self, key: &str) -> Option<&'a str> {
        self.parent.get(key).copied().flatten()
    }

    /// The sibling list holding `key` under base parent `parent`.
    fn siblings_mut(&mut self, parent: Option<&str>) -> Result<&mut Vec<Key>, Reason> {
        match parent {
            Some(p) => Ok(&mut self.block_mut(p)?.children),
            None => Ok(self.roots.get_or_insert_with(|| self.base.roots.clone())),
        }
    }

    fn siblings(&self, parent: Option<&str>) -> Result<&[Key], Reason> {
        match parent {
            Some(p) => Ok(&self.block(p).ok_or(Reason::Incomplete)?.children),
            None => Ok(self.roots.as_deref().unwrap_or(&self.base.roots)),
        }
    }

    /// `key`'s depth (a root is 0); a parent cycle reads as too deep.
    fn depth_of(&self, key: &str) -> usize {
        let up = self.ancestors(key);
        if up.last() == Some(&None) {
            OUTLINE_MAX_DEPTH
        } else {
            up.len()
        }
    }

    /// The window's `existingSubtreeFits` (`tree.ts:146`): `key`'s subtree
    /// fits once reparented under `new_parent`. O(subtree).
    fn subtree_fits(&self, key: &str, new_parent: Option<&str>) -> Result<bool, Reason> {
        self.fits_from(key, new_parent.map_or(0, |p| self.depth_of(p) + 1))
    }

    /// `key`'s subtree fits with `key` itself at depth `first`.
    fn fits_from(&self, key: &str, first: usize) -> Result<bool, Reason> {
        let mut pending = vec![(key, first)];
        while let Some((k, depth)) = pending.pop() {
            if depth >= OUTLINE_MAX_DEPTH {
                return Ok(false);
            }
            let b = self.block(k).ok_or(Reason::Incomplete)?;
            pending.extend(b.children.iter().map(|c| (c.as_str(), depth + 1)));
        }
        Ok(true)
    }

    /// `key` is `root` or inside its subtree; a parent cycle reads as inside.
    fn in_subtree(&self, root: &str, key: &str) -> bool {
        key == root
            || self
                .ancestors(key)
                .iter()
                .any(|a| a.is_none_or(|a| a == root))
    }

    /// `key` and its descendants, in the transaction's current state.
    fn subtree(&self, key: &str) -> Result<Vec<Key>, Reason> {
        let mut out = Vec::new();
        let mut pending = vec![key.to_string()];
        while let Some(k) = pending.pop() {
            let b = self.block(&k).ok_or(Reason::Incomplete)?;
            pending.extend(b.children.iter().cloned());
            out.push(k);
        }
        Ok(out)
    }

    /// Remove `key` from its base parent's sibling list.
    fn detach(&mut self, key: &str) -> Result<(), Reason> {
        let parent = self.parent_of(key);
        let sibs = self.siblings_mut(parent)?;
        let at = sibs
            .iter()
            .position(|k| k == key)
            .ok_or(Reason::Incomplete)?;
        sibs.remove(at);
        Ok(())
    }

    fn kind(&mut self, kinds: &[EditKind]) {
        for k in kinds {
            if !self.kinds.contains(k) {
                self.kinds.push(*k);
            }
        }
    }

    fn patch(self) -> PagePatch {
        let mut upserts = Vec::new();
        let mut deleted = Vec::new();
        for (key, block) in self.blocks {
            match block {
                Some(b) => upserts.push(Upsert {
                    collapsed: self.collapsed.get(&key).copied(),
                    key,
                    raw: b.raw,
                    children: b.children,
                }),
                None if self.base.blocks.contains_key(&key) => deleted.push(key),
                None => {}
            }
        }
        PagePatch {
            name: self.base.name.clone(),
            roots: self.roots,
            upserts,
            deleted,
            kinds: self.kinds,
        }
    }
}

struct Cx<'a> {
    pages: &'a [OutlinePage],
    txs: Vec<PageTx<'a>>,
    keys: KeySource,
    used: u64,
    focus: Option<Focus>,
    notices: Vec<Notice>,
    transfers: Vec<(String, String)>,
}

impl<'a> Cx<'a> {
    /// The transaction of the page holding `key`.
    fn tx_of(&mut self, key: &str) -> Result<usize, Reason> {
        if let Some(i) = self.txs.iter().position(|t| t.block(key).is_some()) {
            return Ok(i);
        }
        let page = self
            .pages
            .iter()
            .find(|p| p.blocks.contains_key(key))
            .ok_or(Reason::Incomplete)?;
        if let Some(i) = self.txs.iter().position(|t| std::ptr::eq(t.base, page)) {
            return Ok(i);
        }
        self.open(page)
    }

    fn open(&mut self, page: &'a OutlinePage) -> Result<usize, Reason> {
        let tx = PageTx::new(page);
        if tx.malformed {
            return Err(Reason::Incomplete);
        }
        self.txs.push(tx);
        Ok(self.txs.len() - 1)
    }

    /// The transaction of the page named `name`.
    fn tx_of_page(&mut self, name: &str) -> Result<usize, Reason> {
        if let Some(i) = self.txs.iter().position(|t| t.base.name == name) {
            return Ok(i);
        }
        let page = self
            .pages
            .iter()
            .find(|p| p.name == name)
            .ok_or(Reason::Incomplete)?;
        self.open(page)
    }

    /// The transaction of each key, all resolved before anything changes.
    fn txs_of(&mut self, keys: &[Key]) -> Result<Vec<usize>, Reason> {
        if keys.is_empty() {
            return Err(Reason::Incomplete);
        }
        keys.iter().map(|k| self.tx_of(k)).collect()
    }

    /// The window's `pageWritable` (`edits/properties.ts:58`).
    fn writable(&self, tx: usize) -> Result<(), Reason> {
        let page = self.txs[tx].base;
        if page.read_only {
            Err(Reason::ReadOnly)
        } else if page.frozen {
            Err(Reason::Frozen)
        } else {
            Ok(())
        }
    }

    fn fresh_key(&mut self) -> Key {
        let key = format!("{}{}", self.keys.prefix, self.keys.next + self.used);
        self.used += 1;
        key
    }

    fn finish(self) -> Change {
        Change {
            pages: self
                .txs
                .into_iter()
                .filter(|t| !t.kinds.is_empty())
                .map(PageTx::patch)
                .collect(),
            focus: self.focus,
            notices: self.notices,
            keys_used: self.used,
            transfers: self.transfers,
        }
    }
}

// ---------------------------------------------------------------------------
// Block text helpers
// ---------------------------------------------------------------------------

fn join(visible: &str, hidden: &str, format: Format) -> Result<String, Reason> {
    editor_text::join(visible, hidden, format.is_org()).map_err(|_| Reason::Unparseable)
}

/// The block's own properties as the window's facets read them: every
/// top-level parser Properties node, in order (`render/facets.ts:172`).
fn own_properties(raw: &str, format: Format) -> Vec<(String, String)> {
    crate::lsdoc_block_parse::parse_block(raw, format.is_org())
        .into_iter()
        .filter_map(|b| match b {
            lsdoc::ast::Block::Properties { props, .. } => Some(props),
            _ => None,
        })
        .flatten()
        .map(|p| (p.0, p.1))
        .collect()
}

/// The value of the block's own property `key`, trimmed: the window's
/// `orderListTypeFromRaw` (`edits/properties.ts:281`, `norm`: keys compared by
/// `propertyKeyNorm`) and `blockProperty` (`:37`, keys compared lower-case).
fn own_property(raw: &str, format: Format, key: &str, norm: bool) -> Option<String> {
    let same = |k: &str| {
        if norm {
            editor_text::property_key_norm(k) == editor_text::property_key_norm(key)
        } else {
            k.to_lowercase() == key.to_lowercase()
        }
    };
    own_properties(raw, format)
        .into_iter()
        .find(|(k, _)| same(k))
        .map(|(_, v)| js_trim(&v).to_string())
}

/// The window's `isOrdered` (`edits/properties.ts:270`): the block's list type
/// is `number`.
pub fn is_ordered(raw: &str, format: Format) -> bool {
    let key = editor_text::property_key_norm(ORDER_LIST_KEY);
    own_properties(raw, format)
        .iter()
        .find(|(k, _)| editor_text::property_key_norm(k) == key)
        .is_some_and(|(_, v)| v.trim() == "number")
}

/// The window's `rawWithOrderListType` (`edits/properties.ts:292`). OG writes
/// the property at `outliner/core.cljs:420-433` (6e7afa8eb).
pub fn raw_with_order_list_type(
    raw: &str,
    value: Option<&str>,
    format: Format,
) -> Result<String, Reason> {
    let hide = HidePolicy::Keys {
        keys: vec![ORDER_LIST_KEY.to_string()],
        norm: true,
    };
    let visible = editor_text::split(raw, format.is_org(), &hide).visible;
    let Some(value) = value else {
        return Ok(visible);
    };
    let property = match format {
        Format::Org => format!(":{ORDER_LIST_KEY}: {value}"),
        Format::Md => format!("{ORDER_LIST_KEY}:: {value}"),
    };
    join(&visible, &property, format)
}

/// The window's `rawWithCollapsed(raw, false)` (`edits/properties.ts:492`):
/// drop the persisted `collapsed` property.
pub fn raw_expanded(raw: &str, format: Format) -> Result<String, Reason> {
    if format.is_org() {
        let edit = block_regions::Edit::Property {
            key: "collapsed".into(),
            value: None,
        };
        return block_regions::edit(raw, true, edit).map_err(|_| Reason::Unparseable);
    }
    let t = editor_text::split(raw, false, &HidePolicy::Builtin);
    let hidden = remove_property_lines(&t.hidden, "collapsed")?;
    join(&t.visible, &hidden, format)
}

/// `upsertPropertyLine(block, key, null) ?? ""` (`editor/properties.ts:295`):
/// every accepted line for `key` removed, the rest byte for byte; all blank
/// leaves nothing.
fn remove_property_lines(block: &str, key: &str) -> Result<String, Reason> {
    if block.is_empty() {
        return Ok(String::new());
    }
    let regions = block_regions::parse(block, false);
    if regions.quarantined {
        return Err(Reason::Unparseable);
    }
    let mut out = Vec::new();
    let mut start = 0;
    for line in block.split('\n') {
        let dropped = regions
            .properties
            .iter()
            .any(|p| p.primary && p.line.0 == start && p.key.to_lowercase() == key.to_lowercase());
        if !dropped {
            out.push(line);
        }
        start += line.len() + 1;
    }
    Ok(if out.iter().all(|l| js_blank(l)) {
        String::new()
    } else {
        out.join("\n")
    })
}

/// The editor-visible join offset as a raw offset in `raw`.
fn raw_offset_of_visible(raw: &str, format: Format, visible: usize) -> usize {
    let regions = block_regions::parse(raw, format.is_org());
    editor_text::visible_to_raw_with(
        raw,
        format.is_org(),
        &regions,
        &HidePolicy::Builtin,
        visible,
    )
}

// ---------------------------------------------------------------------------
// Verbs
// ---------------------------------------------------------------------------

/// Enter: the window's `splitBlock` (`edits/blocks.ts:202`); OG
/// `insert-new-block-aux!` / `insert-new-block-before-block-aux!`
/// (`handler/editor.cljs:487`, `:464`). Hidden properties stay on the original
/// block. A caret on either side of a line break splits at that break (GH #361).
/// A blank head with content after it inserts a new empty block before the
/// original, which keeps its identity, text and children.
fn split(
    cx: &mut Cx,
    key: &str,
    raw_offset: usize,
    force_child: bool,
    children_visible: bool,
    keep_start_in_scope: bool,
) -> Result<(), Reason> {
    let t = cx.tx_of(key)?;
    cx.writable(t)?;
    let format = cx.txs[t].base.format;
    let node = cx.txs[t].block(key).ok_or(Reason::Incomplete)?.clone();
    let regions = block_regions::parse(&node.raw, format.is_org());
    let et = editor_text::split_with(&node.raw, format.is_org(), &regions, &HidePolicy::Builtin);
    let offset = editor_text::raw_to_visible_with(
        &node.raw,
        format.is_org(),
        &regions,
        &HidePolicy::Builtin,
        raw_offset,
    );
    let visible = et.visible.as_bytes();
    let boundary_before = offset < visible.len() && visible[offset] == b'\n';
    let boundary_after = offset > 0 && visible[offset - 1] == b'\n';
    let split_before = if boundary_after { offset - 1 } else { offset };
    let split_after = if !boundary_after && boundary_before {
        offset + 1
    } else {
        offset
    };
    let before = &et.visible[..split_before];
    let after = &et.visible[split_after..];
    let head = js_blank(before) && !js_blank(after);
    // The page header is one text field (GH #210 F1; `header::newline_in_header`).
    let is_header = header::is_first_root_header(&cx.txs[t], key);
    if is_header && (head || !js_blank(after)) {
        return header::newline_in_header(cx, t, key, &et, offset, format);
    }
    let force_child = force_child && !is_header;
    let has_visible_children = !node.children.is_empty() && children_visible;
    let child_split = if head {
        keep_start_in_scope
    } else {
        has_visible_children || force_child
    };
    if child_split && cx.txs[t].depth_of(key) + 2 > OUTLINE_MAX_DEPTH {
        return Err(Reason::DepthCap);
    }
    // Ordered-list items propagate to the block split off them (OG inherits
    // `:logseq.order-list-type`).
    let ordered = |raw: &str| -> Result<String, Reason> {
        if is_ordered(&node.raw, format) {
            raw_with_order_list_type(raw, Some("number"), format)
        } else {
            Ok(raw.to_string())
        }
    };
    let new_key = cx.fresh_key();
    let tx = &mut cx.txs[t];
    let parent = tx.parent_of(key);
    if head {
        // The original keeps its key. At a later line boundary the blank prefix
        // and its separator become the new block, so the original keeps only
        // the text after the boundary.
        if offset > 0 {
            tx.block_mut(key)?.raw = join(after, &et.hidden, format)?;
        }
        tx.insert(
            new_key.clone(),
            Block {
                raw: ordered("")?,
                children: Vec::new(),
            },
        );
        tx.collapsed.insert(new_key.clone(), false);
        if keep_start_in_scope {
            tx.block_mut(key)?.children.insert(0, new_key.clone());
        } else {
            let sibs = tx.siblings_mut(parent)?;
            let at = sibs
                .iter()
                .position(|k| k == key)
                .ok_or(Reason::Incomplete)?;
            sibs.insert(at, new_key.clone());
        }
        tx.kind(if offset > 0 {
            &[EditKind::InsertBlocks, EditKind::SaveBlock]
        } else {
            &[EditKind::InsertBlocks]
        });
    } else {
        tx.block_mut(key)?.raw = join(before, &et.hidden, format)?;
        tx.insert(
            new_key.clone(),
            Block {
                raw: ordered(after)?,
                children: Vec::new(),
            },
        );
        tx.collapsed.insert(new_key.clone(), false);
        if child_split {
            tx.block_mut(key)?.children.insert(0, new_key.clone());
        } else {
            let sibs = tx.siblings_mut(parent)?;
            let at = sibs
                .iter()
                .position(|k| k == key)
                .ok_or(Reason::Incomplete)?;
            sibs.insert(at + 1, new_key.clone());
        }
        tx.kind(&[EditKind::SaveBlock, EditKind::InsertBlocks]);
    }
    cx.focus = Some(Focus {
        key: new_key,
        raw_offset: Some(0),
    });
    Ok(())
}

/// Backspace at start / Delete at end: the window's `absorbInto`
/// (`edits/blocks.ts:436`); OG `delete-block!` with `move-to-prev-block`
/// (`handler/editor.cljs:825`, `:790`). The absorbed block's editor text is
/// appended to the survivor's, its children follow the survivor's, and it is
/// removed. The survivor keeps its key and hidden properties; the absorbed
/// `id::` moves only when the survivor has none (OG parity candidate, §4.3).
fn merge(cx: &mut Cx, survivor: &str, absorbed: &str) -> Result<(), Reason> {
    let t = cx.tx_of(absorbed)?;
    let s = cx.tx_of(survivor)?;
    if s != t {
        return Err(Reason::CrossPageMerge);
    }
    cx.writable(t)?;
    let format = cx.txs[t].base.format;
    let tx = &mut cx.txs[t];
    if survivor == absorbed || tx.in_subtree(absorbed, survivor) {
        return Err(Reason::Cycle);
    }
    let gone = tx.block(absorbed).ok_or(Reason::Incomplete)?.clone();
    let keep_raw = tx.block(survivor).ok_or(Reason::Incomplete)?.raw.clone();
    // The absorbed children move under the survivor, which may sit deeper.
    if !all_fit(&gone.children, |c| tx.subtree_fits(c, Some(survivor)))? {
        return Err(Reason::DepthCap);
    }
    let keep = editor_text::split(&keep_raw, format.is_org(), &HidePolicy::Builtin);
    let lost = editor_text::split(&gone.raw, format.is_org(), &HidePolicy::Builtin);
    let survivor_has_id = block_regions::parse(&keep_raw, format.is_org())
        .id
        .is_some_and(|p| !p.value.trim().is_empty());
    let absorbed_id = block_regions::parse(&gone.raw, format.is_org())
        .id
        .map(|p| js_trim(p.line.slice(&gone.raw)).to_string());
    let mut hidden = keep.hidden.clone();
    if let (false, Some(id)) = (survivor_has_id, &absorbed_id) {
        hidden = if hidden.is_empty() {
            id.clone()
        } else {
            format!("{hidden}\n{id}")
        };
        cx.notices.push(Notice::IdTransferred {
            from: absorbed.to_string(),
            to: survivor.to_string(),
        });
    }
    let raw = join(
        &format!("{}{}", keep.visible, lost.visible),
        &hidden,
        format,
    )?;
    let focus = raw_offset_of_visible(&raw, format, keep.visible.len());
    let parent = tx.parent_of(absorbed);
    let b = tx.block_mut(survivor)?;
    b.raw = raw;
    b.children.extend(gone.children.iter().cloned());
    let sibs = tx.siblings_mut(parent)?;
    sibs.retain(|k| k != absorbed);
    tx.blocks.insert(absorbed.to_string(), None);
    tx.kind(&[
        EditKind::SaveBlock,
        EditKind::MoveBlocks,
        EditKind::DeleteBlocks,
    ]);
    cx.focus = Some(Focus {
        key: survivor.to_string(),
        raw_offset: Some(focus),
    });
    Ok(())
}

/// Tab: the window's `indentBlock` (`edits/blocks.ts:312`) and
/// `indentSelection` (`edits/selection.ts:345`); OG `indent-outdent-blocks`
/// (`outliner/core.cljs:802`). `keys` (all on one page) become the last
/// children of the first key's previous sibling, which is expanded, in the
/// file too.
fn indent(cx: &mut Cx, keys: &[Key]) -> Result<(), Reason> {
    let t = one_page(cx, keys)?;
    cx.writable(t)?;
    let format = cx.txs[t].base.format;
    let tx = &mut cx.txs[t];
    let first = &keys[0];
    let sibs = tx.siblings(tx.parent_of(first))?;
    let i = sibs
        .iter()
        .position(|k| k == first)
        .ok_or(Reason::Incomplete)?;
    if i == 0 {
        return Err(Reason::NoOutlineStep);
    }
    let new_parent = sibs[i - 1].clone();
    if tx.base.header_root.as_deref() == Some(new_parent.as_str()) {
        return Err(Reason::HeaderRoot);
    }
    if keys.iter().any(|k| tx.in_subtree(k, &new_parent)) {
        return Err(Reason::Cycle);
    }
    if !all_fit(keys, |k| tx.subtree_fits(k, Some(&new_parent)))? {
        return Err(Reason::DepthCap);
    }
    let expanded = raw_expanded(
        &tx.block(&new_parent).ok_or(Reason::Incomplete)?.raw,
        format,
    )?;
    for k in keys {
        tx.detach(k)?;
    }
    let np = tx.block_mut(&new_parent)?;
    np.children.extend(keys.iter().cloned());
    np.raw = expanded;
    tx.collapsed.insert(new_parent, false);
    tx.kind(&[EditKind::MoveBlocks, EditKind::SaveBlock]);
    cx.focus = keep_editing(keys);
    Ok(())
}

/// Shift+Tab: the window's `outdentBlock` (`edits/blocks.ts:363`) and
/// `outdentSelection` (`edits/selection.ts:372`); OG `indent-outdent-blocks`
/// (`outliner/core.cljs:835-852`). `keys` (all on one page) land after the
/// first key's parent, in order. Traditional outdenting of one block takes its
/// following siblings along as its last children; logical outdenting and a
/// selection move the blocks alone.
fn outdent(cx: &mut Cx, keys: &[Key], logical: bool) -> Result<(), Reason> {
    let t = one_page(cx, keys)?;
    cx.writable(t)?;
    let tx = &mut cx.txs[t];
    let first = &keys[0];
    let parent = tx.parent_of(first).ok_or(Reason::NoOutlineStep)?;
    let grand = tx.parent_of(parent);
    if keys.iter().any(|k| tx.in_subtree(k, parent)) {
        return Err(Reason::Cycle);
    }
    if !all_fit(keys, |k| tx.subtree_fits(k, grand))? {
        return Err(Reason::DepthCap);
    }
    if let ([key], false) = (keys, logical) {
        let children = &mut tx.block_mut(parent)?.children;
        let idx = children
            .iter()
            .position(|k| k == key)
            .ok_or(Reason::Incomplete)?;
        let following = children.split_off(idx + 1);
        children.pop();
        tx.block_mut(key)?.children.extend(following);
    } else {
        for k in keys {
            tx.detach(k)?;
        }
    }
    let g = tx.siblings_mut(grand)?;
    let at = g
        .iter()
        .position(|k| k == parent)
        .ok_or(Reason::Incomplete)?;
    g.splice(at + 1..at + 1, keys.iter().cloned());
    tx.kind(&[EditKind::MoveBlocks]);
    cx.focus = keep_editing(keys);
    Ok(())
}

/// The window's `deleteBlock` (`edits/blocks.ts:597`) and `deleteSelection`
/// (`edits/selection.ts:393`); OG `delete-blocks` (`outliner/core.cljs:654`):
/// each block is removed with its subtree.
/// Keys are deduplicated and a key inside another key's subtree goes with it,
/// as in [`move_to`].
fn delete(cx: &mut Cx, keys: &[Key]) -> Result<(), Reason> {
    let roots = outermost(cx, keys)?;
    for (_, t) in &roots {
        cx.writable(*t)?;
    }
    for (k, t) in &roots {
        let tx = &mut cx.txs[*t];
        tx.detach(k)?;
        tx.remove_subtree(k)?;
        tx.kind(&[EditKind::DeleteBlocks]);
    }
    Ok(())
}

/// `keys` deduplicated, without those inside another key's subtree, in order,
/// each with its page. A cycle above a key is `Incomplete`.
fn outermost(cx: &mut Cx, keys: &[Key]) -> Result<Vec<(Key, usize)>, Reason> {
    let mut unique: Vec<Key> = Vec::with_capacity(keys.len());
    let mut seen = std::collections::HashSet::new();
    for k in keys {
        if seen.insert(k.as_str()) {
            unique.push(k.clone());
        }
    }
    let ts = cx.txs_of(&unique)?;
    let mut roots = Vec::new();
    for (k, &t) in unique.iter().zip(&ts) {
        let ancestors = cx.txs[t].ancestors(k);
        if ancestors.contains(&None) {
            return Err(Reason::Incomplete);
        }
        if !ancestors.iter().flatten().any(|a| seen.contains(a)) {
            roots.push((k.clone(), t));
        }
    }
    Ok(roots)
}

/// Drag and drop, and a journal-feed move across days: the window's
/// `moveBlocksRelative` + `relativeMovePlan` (`edits/moves.ts:101`, `:172`),
/// `moveBlock` (`:16`) and `crossMoveBlocks` (`:372`); OG `move-blocks`
/// (`outliner/core.cljs:717`). Keys are deduplicated and a key inside another
/// key's subtree goes with it. The moved blocks keep their order and land
/// together at `to`; a block that changes page takes its subtree along, and
/// the pages it left are transfer sources. A moved block without its own list
/// type takes `inherit_from`'s, written in the destination page's syntax.
/// Refusals: a target inside a moved subtree is a `Cycle`, and the depth cap
/// is checked last, so a no-op drop into its own subtree stays silent.
fn move_to(
    cx: &mut Cx,
    keys: &[Key],
    to: &Place,
    inherit_from: Option<&str>,
) -> Result<(), Reason> {
    let roots = outermost(cx, keys)?;
    let (d, parent, anchor) = match to {
        Place::Before { key } | Place::After { key } => {
            let d = cx.tx_of(key)?;
            (
                d,
                cx.txs[d].parent_of(key).map(str::to_string),
                key.as_str(),
            )
        }
        Place::LastChild { key } => (cx.tx_of(key)?, Some(key.clone()), key.as_str()),
        Place::Slot {
            parent: Some(p), ..
        } => (cx.tx_of(p)?, Some(p.clone()), p.as_str()),
        Place::Slot {
            parent: None, page, ..
        } => (cx.tx_of_page(page)?, None, ""),
    };
    if !anchor.is_empty()
        && roots
            .iter()
            .any(|(k, t)| *t == d && cx.txs[d].in_subtree(k, anchor))
    {
        return Err(Reason::Cycle);
    }
    cx.writable(d)?;
    for (_, t) in &roots {
        cx.writable(*t)?;
    }
    let first = parent.as_deref().map_or(0, |p| cx.txs[d].depth_of(p) + 1);
    if !all_fit(&roots, |(k, t)| cx.txs[*t].fits_from(k, first))? {
        return Err(Reason::DepthCap);
    }
    let format = cx.txs[d].base.format;
    let inherited = match inherit_from {
        Some(k) => {
            let i = cx.tx_of(k)?;
            let raw = &cx.txs[i].block(k).ok_or(Reason::Incomplete)?.raw;
            own_property(raw, cx.txs[i].base.format, ORDER_LIST_KEY, false)
        }
        None => None,
    };
    let mut raws = Vec::with_capacity(roots.len());
    for (k, t) in &roots {
        let tx = &cx.txs[*t];
        let raw = &tx.block(k).ok_or(Reason::Incomplete)?.raw;
        raws.push(match &inherited {
            Some(v) if own_property(raw, tx.base.format, ORDER_LIST_KEY, true).is_none() => {
                Some(raw_with_order_list_type(raw, Some(v), format)?)
            }
            _ => None,
        });
    }
    // A slot counts the list before the move: moved blocks above it shift it.
    let slot = match to {
        Place::Slot { index, .. } => {
            let list = cx.txs[d].siblings(parent.as_deref())?;
            let above = list[..(*index).min(list.len())]
                .iter()
                .filter(|k| roots.iter().any(|(r, t)| *t == d && r == *k))
                .count();
            Some(index - above)
        }
        _ => None,
    };
    for (k, t) in &roots {
        cx.txs[*t].detach(k)?;
        cx.txs[*t].kind(&[EditKind::MoveBlocks]);
        if *t != d {
            for key in cx.txs[*t].subtree(k)? {
                let block = cx.txs[*t].block(&key).ok_or(Reason::Incomplete)?.clone();
                cx.txs[*t].blocks.insert(key.clone(), None);
                cx.txs[d].insert(key, block);
            }
            let edge = (cx.txs[*t].base.name.clone(), cx.txs[d].base.name.clone());
            if !cx.transfers.contains(&edge) {
                cx.transfers.push(edge);
            }
        }
    }
    let tx = &mut cx.txs[d];
    for ((k, _), raw) in roots.iter().zip(raws) {
        if let Some(raw) = raw.filter(|r| *r != tx.block(k).map_or("", |b| &b.raw)) {
            tx.block_mut(k)?.raw = raw;
        }
    }
    let list = tx.siblings_mut(parent.as_deref())?;
    let at = match to {
        Place::Before { key } | Place::After { key } => {
            let i = list
                .iter()
                .position(|k| k == key)
                .ok_or(Reason::Incomplete)?;
            if matches!(to, Place::After { .. }) {
                i + 1
            } else {
                i
            }
        }
        Place::LastChild { .. } => list.len(),
        Place::Slot { .. } => slot.unwrap_or(0).min(list.len()),
    };
    list.splice(at..at, roots.iter().map(|(k, _)| k.clone()));
    // A drop (one with a list-type source) is a block save too, as the
    // window's drag has always reported it.
    tx.kind(if inherit_from.is_some() {
        &[EditKind::MoveBlocks, EditKind::SaveBlock]
    } else {
        &[EditKind::MoveBlocks]
    });
    Ok(())
}

/// Mod+Up/Down: the window's `moveItem` + `outlineStepPlan`
/// (`edits/moves.ts:273`, `:300`) and `moveSelectionItems` (`:477`); OG
/// `move-blocks-up-down` (`outliner/core.cljs:760`). One key takes one outline
/// step: a sibling swap, or at a child-list edge into the adjacent parent
/// sibling (down: its first child; up: its last), inheriting the new parent's
/// list type (a block save only when that rewrote its text). `within_siblings` moves every key one slot in its own sibling
/// list, the edge-most key first, skipping a key already at its edge; the lead
/// key (topmost going up, bottommost going down) must be able to move.
/// `NoOutlineStep` lets the window try the journal feed's cross-day move.
fn move_up_down(cx: &mut Cx, keys: &[Key], up: bool, within_siblings: bool) -> Result<(), Reason> {
    let ts = cx.txs_of(keys)?;
    for &t in &ts {
        cx.writable(t)?;
    }
    let step = |list: &[Key], key: &str| -> Option<(usize, usize)> {
        let i = list.iter().position(|k| k == key)?;
        let j = if up { i.checked_sub(1)? } else { i + 1 };
        (j < list.len()).then_some((i, j))
    };
    if within_siblings {
        let lead = if up { 0 } else { keys.len() - 1 };
        let tx = &cx.txs[ts[lead]];
        if step(tx.siblings(tx.parent_of(&keys[lead]))?, &keys[lead]).is_none() {
            return Err(Reason::NoOutlineStep);
        }
        let order: Vec<usize> = if up {
            (0..keys.len()).collect()
        } else {
            (0..keys.len()).rev().collect()
        };
        for i in order {
            let tx = &mut cx.txs[ts[i]];
            let list = tx.siblings_mut(tx.parent_of(&keys[i]))?;
            if let Some((from, to)) = step(list, &keys[i]) {
                let k = list.remove(from);
                list.insert(to, k);
                tx.kind(&[EditKind::MoveBlocks]);
            }
        }
        return Ok(());
    }
    let [key] = keys else {
        return Err(Reason::Incomplete);
    };
    let tx = &mut cx.txs[ts[0]];
    let format = tx.base.format;
    let parent = tx.parent_of(key);
    let (new_parent, index) = match step(tx.siblings(parent)?, key) {
        Some((_, j)) => (parent.map(str::to_string), j),
        None => {
            let p = parent.ok_or(Reason::NoOutlineStep)?;
            let outer = tx.siblings(tx.parent_of(p))?;
            let i = outer
                .iter()
                .position(|k| k == p)
                .ok_or(Reason::Incomplete)?;
            let adjacent = if up { i.checked_sub(1) } else { Some(i + 1) }
                .and_then(|j| outer.get(j))
                .ok_or(Reason::NoOutlineStep)?
                .clone();
            let at = if up {
                tx.block(&adjacent)
                    .ok_or(Reason::Incomplete)?
                    .children
                    .len()
            } else {
                0
            };
            (Some(adjacent), at)
        }
    };
    let reparented = new_parent.as_deref() != parent;
    let raw = tx.block(key).ok_or(Reason::Incomplete)?.raw.clone();
    let inherited = new_parent
        .as_deref()
        .filter(|_| reparented && own_property(&raw, format, ORDER_LIST_KEY, true).is_none())
        .and_then(|p| tx.block(p))
        .and_then(|b| own_property(&b.raw, format, ORDER_LIST_KEY, false));
    tx.detach(key)?;
    let list = tx.siblings_mut(new_parent.as_deref())?;
    let at = index.min(list.len());
    list.insert(at, key.clone());
    let rewritten = match inherited {
        Some(v) => Some(raw_with_order_list_type(&raw, Some(&v), format)?).filter(|r| *r != raw),
        None => None,
    };
    let saved = rewritten.is_some();
    if let Some(r) = rewritten {
        tx.block_mut(key)?.raw = r;
    }
    tx.kind(if saved {
        &[EditKind::MoveBlocks, EditKind::SaveBlock]
    } else {
        &[EditKind::MoveBlocks]
    });
    Ok(())
}

/// A block edited through Tab or Shift+Tab keeps its editor and caret; a
/// selection command moves no caret.
fn keep_editing(keys: &[Key]) -> Option<Focus> {
    match keys {
        [key] => Some(Focus {
            key: key.clone(),
            raw_offset: None,
        }),
        _ => None,
    }
}

/// Whether every item fits; a block the check needs and was not given is
/// `Incomplete`.
fn all_fit<T>(
    items: &[T],
    mut fits: impl FnMut(&T) -> Result<bool, Reason>,
) -> Result<bool, Reason> {
    for item in items {
        if !fits(item)? {
            return Ok(false);
        }
    }
    Ok(true)
}

/// All `keys` on one page; the page's transaction.
fn one_page(cx: &mut Cx, keys: &[Key]) -> Result<usize, Reason> {
    let ts = cx.txs_of(keys)?;
    if ts.iter().any(|&t| t != ts[0]) {
        return Err(Reason::Incomplete);
    }
    Ok(ts[0])
}

/// The window's `insertEmptyChildBlock` (`edits/blocks.ts:118`).
fn insert_empty_child(cx: &mut Cx, parent: &str, at: usize) -> Result<(), Reason> {
    let t = cx.tx_of(parent)?;
    cx.writable(t)?;
    let len = cx.txs[t]
        .block(parent)
        .ok_or(Reason::Incomplete)?
        .children
        .len();
    if at > len {
        return Err(Reason::Incomplete);
    }
    if cx.txs[t].depth_of(parent) + 2 > OUTLINE_MAX_DEPTH {
        return Err(Reason::DepthCap);
    }
    let key = cx.fresh_key();
    let tx = &mut cx.txs[t];
    tx.insert(
        key.clone(),
        Block {
            raw: String::new(),
            children: Vec::new(),
        },
    );
    tx.collapsed.insert(key.clone(), false);
    tx.block_mut(parent)?.children.insert(at, key.clone());
    tx.kind(&[EditKind::InsertBlocks]);
    cx.focus = Some(Focus {
        key,
        raw_offset: Some(0),
    });
    Ok(())
}

/// JavaScript's `String.prototype.trim()` (see [`js_blank`]).
fn js_trim(s: &str) -> &str {
    let blank = |c: char| js_blank(c.encode_utf8(&mut [0; 4]));
    s.trim_matches(blank)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn page(name: &str, blocks: &[(&str, &str, &[&str])], roots: &[&str]) -> OutlinePage {
        OutlinePage {
            name: name.into(),
            format: Format::Md,
            pre_block: None,
            roots: roots.iter().map(|r| r.to_string()).collect(),
            blocks: blocks
                .iter()
                .map(|(k, raw, children)| {
                    (
                        k.to_string(),
                        Block {
                            raw: raw.to_string(),
                            children: children.iter().map(|c| c.to_string()).collect(),
                        },
                    )
                })
                .collect(),
            header_root: None,
            read_only: false,
            frozen: false,
        }
    }

    /// A chain d0 > d1 > … with `levels` blocks; the last sits at depth levels-1.
    fn chain(levels: usize) -> OutlinePage {
        let keys: Vec<String> = (0..levels).map(|i| format!("d{i}")).collect();
        let mut p = page("Deep", &[], &["d0"]);
        for (i, k) in keys.iter().enumerate() {
            let children = keys.get(i + 1).map_or(vec![], |c| vec![c.clone()]);
            p.blocks.insert(
                k.clone(),
                Block {
                    raw: format!("level {i}"),
                    children,
                },
            );
        }
        p
    }

    fn run(pages: &[OutlinePage], verb: Verb) -> Result<Change, Reason> {
        apply(
            pages,
            &verb,
            &KeySource {
                prefix: "n".into(),
                next: 0,
            },
        )
    }

    #[test]
    fn child_inserts_stop_at_the_depth_cap() {
        let p = chain(OUTLINE_MAX_DEPTH);
        let last = format!("d{}", OUTLINE_MAX_DEPTH - 1);
        let below = format!("d{}", OUTLINE_MAX_DEPTH - 2);
        let split = |key: &str| Verb::Split {
            key: key.into(),
            raw_offset: 2,
            force_child: true,
            children_visible: true,
            keep_start_in_scope: false,
        };
        assert_eq!(run(&[p.clone()], split(&last)), Err(Reason::DepthCap));
        assert!(run(&[p.clone()], split(&below)).is_ok());
        assert_eq!(
            run(
                &[p.clone()],
                Verb::InsertEmptyChild {
                    parent: last,
                    at: 0
                }
            ),
            Err(Reason::DepthCap)
        );
        assert!(run(
            &[p],
            Verb::InsertEmptyChild {
                parent: below,
                at: 1
            }
        )
        .is_ok());
    }

    #[test]
    fn reparenting_verbs_check_the_moved_subtree_depth() {
        // x (depth 0) + a 127-level chain beside it: indenting the chain under x
        // puts its leaf at depth 128.
        let mut p = chain(OUTLINE_MAX_DEPTH);
        p.blocks.insert(
            "x".into(),
            Block {
                raw: "x".into(),
                children: vec![],
            },
        );
        p.roots = vec!["x".into(), "d0".into()];
        assert_eq!(
            run(
                &[p.clone()],
                Verb::Indent {
                    keys: vec!["d0".into()]
                }
            ),
            Err(Reason::DepthCap)
        );
        // Merging d0 into x adopts d1's 126-level subtree at depth 1: it fits.
        assert!(run(
            &[p.clone()],
            Verb::Merge {
                survivor: "x".into(),
                absorbed: "d0".into(),
                absorbed_referenced: false
            }
        )
        .is_ok());
        // Merging d1 into the leaf of a second deep chain does not.
        let mut deep = chain(OUTLINE_MAX_DEPTH - 1);
        for (k, b) in deep.blocks.drain() {
            p.blocks.insert(
                format!("e{k}"),
                Block {
                    raw: b.raw,
                    children: b.children.iter().map(|c| format!("e{c}")).collect(),
                },
            );
        }
        p.roots.push("ed0".into());
        let survivor = format!("ed{}", OUTLINE_MAX_DEPTH - 2);
        assert_eq!(
            run(
                &[p],
                Verb::Merge {
                    survivor,
                    absorbed: "d0".into(),
                    absorbed_referenced: false
                }
            ),
            Err(Reason::DepthCap)
        );
    }

    #[test]
    fn merge_refuses_across_pages_and_into_its_own_subtree() {
        let a = page("A", &[("a", "one", &[])], &["a"]);
        let b = page("B", &[("b", "two", &[])], &["b"]);
        let merge = |s: &str, x: &str| Verb::Merge {
            survivor: s.into(),
            absorbed: x.into(),
            absorbed_referenced: false,
        };
        assert_eq!(
            run(&[a.clone(), b], merge("a", "b")),
            Err(Reason::CrossPageMerge)
        );
        let nested = page("N", &[("p", "parent", &["c"]), ("c", "child", &[])], &["p"]);
        assert_eq!(run(&[nested.clone()], merge("c", "p")), Err(Reason::Cycle));
        assert_eq!(run(&[nested], merge("p", "p")), Err(Reason::Cycle));
        assert_eq!(run(&[a], merge("a", "missing")), Err(Reason::Incomplete));
    }

    #[test]
    fn frozen_and_read_only_pages_take_no_edit() {
        let mut p = page("P", &[("a", "one", &[]), ("b", "two", &[])], &["a", "b"]);
        p.frozen = true;
        assert_eq!(
            run(
                &[p.clone()],
                Verb::Delete {
                    keys: vec!["a".into()]
                }
            ),
            Err(Reason::Frozen)
        );
        p.read_only = true;
        assert_eq!(
            run(
                &[p],
                Verb::Indent {
                    keys: vec!["b".into()]
                }
            ),
            Err(Reason::ReadOnly)
        );
    }

    #[test]
    fn a_change_is_a_patch_of_touched_blocks_only() {
        let p = page(
            "P",
            &[("a", "one", &[]), ("b", "two", &[]), ("c", "three", &[])],
            &["a", "b", "c"],
        );
        let change = run(
            &[p],
            Verb::Indent {
                keys: vec!["b".into()],
            },
        )
        .unwrap();
        let patch = &change.pages[0];
        assert_eq!(
            patch.roots.as_deref(),
            Some(&["a".to_string(), "c".to_string()][..])
        );
        assert_eq!(
            patch
                .upserts
                .iter()
                .map(|u| u.key.as_str())
                .collect::<Vec<_>>(),
            ["a"]
        );
        assert!(patch.deleted.is_empty());
        assert_eq!(change.keys_used, 0);
    }

    fn move_to(keys: &[&str], to: Place, inherit_from: Option<&str>) -> Verb {
        Verb::MoveTo {
            keys: keys.iter().map(|k| k.to_string()).collect(),
            to,
            inherit_from: inherit_from.map(str::to_string),
        }
    }

    #[test]
    fn a_move_between_pages_carries_the_subtree_and_names_the_transfer() {
        let a = page(
            "A",
            &[("x", "x", &["x1"]), ("x1", "kid", &[]), ("y", "y", &[])],
            &["x", "y"],
        );
        let b = page(
            "B",
            &[("t", "t\nlogseq.order-list-type:: number", &[])],
            &["t"],
        );
        let change = run(
            &[a.clone(), b.clone()],
            move_to(&["x"], Place::After { key: "t".into() }, Some("t")),
        )
        .unwrap();
        assert_eq!(change.transfers, vec![("A".to_string(), "B".to_string())]);
        let (mut a2, mut b2) = (a, b);
        for patch in &change.pages {
            let target = if patch.name == "A" { &mut a2 } else { &mut b2 };
            patch.apply_to(target);
        }
        assert_eq!(a2.roots, ["y"]);
        assert!(!a2.blocks.contains_key("x") && !a2.blocks.contains_key("x1"));
        assert_eq!(b2.roots, ["t", "x"]);
        assert_eq!(b2.blocks["x"].children, ["x1"]);
        assert_eq!(b2.blocks["x"].raw, "x\nlogseq.order-list-type:: number");
        assert_eq!(b2.blocks["x1"].raw, "kid");
        let kinds = |name: &str| {
            change
                .pages
                .iter()
                .find(|p| p.name == name)
                .unwrap()
                .kinds
                .clone()
        };
        assert_eq!(kinds("A"), [EditKind::MoveBlocks]);
        assert_eq!(kinds("B"), [EditKind::MoveBlocks, EditKind::SaveBlock]);
    }

    #[test]
    fn a_feed_move_lands_at_a_root_slot_of_the_other_day() {
        let a = page("Mon", &[("m", "m", &[]), ("n", "n", &[])], &["m", "n"]);
        let b = page("Sun", &[("s", "s", &[])], &["s"]);
        let slot = |index| Place::Slot {
            parent: None,
            page: "Sun".into(),
            index,
        };
        for (index, roots) in [(0, vec!["m", "n", "s"]), (1, vec!["s", "m", "n"])] {
            let change = run(
                &[a.clone(), b.clone()],
                move_to(&["m", "n"], slot(index), None),
            )
            .unwrap();
            let sun = change.pages.iter().find(|p| p.name == "Sun").unwrap();
            assert_eq!(sun.roots.as_deref().unwrap(), roots);
            assert_eq!(sun.kinds, [EditKind::MoveBlocks]);
            let mon = change.pages.iter().find(|p| p.name == "Mon").unwrap();
            assert_eq!(mon.roots.as_deref().unwrap(), Vec::<String>::new());
            assert_eq!(mon.deleted.len(), 2);
        }
        // A page not given is incomplete, never "empty".
        assert_eq!(
            run(&[a], move_to(&["m"], slot(0), None)),
            Err(Reason::Incomplete)
        );
    }

    #[test]
    fn a_move_checks_every_page_it_touches() {
        let a = page("A", &[("x", "x", &[])], &["x"]);
        let mut b = page("B", &[("t", "t", &[])], &["t"]);
        b.frozen = true;
        let after_t = || move_to(&["x"], Place::After { key: "t".into() }, Some("t"));
        assert_eq!(run(&[a.clone(), b.clone()], after_t()), Err(Reason::Frozen));
        b.frozen = false;
        let mut ro = a.clone();
        ro.read_only = true;
        assert_eq!(run(&[ro, b.clone()], after_t()), Err(Reason::ReadOnly));
        assert!(run(&[a, b], after_t()).is_ok());
    }

    #[test]
    fn moves_and_selections_check_the_depth_cap() {
        // x beside a 127-level chain: nesting the chain under x overflows.
        let mut p = chain(OUTLINE_MAX_DEPTH);
        p.blocks.insert(
            "x".into(),
            Block {
                raw: "x".into(),
                children: vec![],
            },
        );
        p.blocks.insert(
            "y".into(),
            Block {
                raw: "y".into(),
                children: vec![],
            },
        );
        p.roots = vec!["x".into(), "d0".into(), "y".into()];
        assert_eq!(
            run(
                &[p.clone()],
                move_to(&["d0"], Place::LastChild { key: "x".into() }, Some("x"))
            ),
            Err(Reason::DepthCap)
        );
        assert_eq!(
            run(
                &[p.clone()],
                Verb::Indent {
                    keys: vec!["d0".into(), "y".into()]
                }
            ),
            Err(Reason::DepthCap)
        );
        assert!(run(
            &[p],
            move_to(&["d0"], Place::After { key: "y".into() }, Some("y"))
        )
        .is_ok());
    }

    #[test]
    fn a_malformed_arena_refuses_instead_of_looping() {
        // b listed under a and as a root.
        let twice = page("P", &[("a", "a", &["b"]), ("b", "b", &[])], &["a", "b"]);
        assert_eq!(
            run(
                &[twice],
                Verb::Delete {
                    keys: vec!["a".into()]
                }
            ),
            Err(Reason::Incomplete)
        );
        // A parent cycle no root reaches: walks stop after one lap.
        let mut lap = page(
            "P",
            &[("r", "r", &[]), ("c", "c", &["d"]), ("d", "d", &["c"])],
            &["r"],
        );
        lap.blocks.get_mut("r").unwrap().children = vec![];
        assert_eq!(
            run(
                &[lap.clone()],
                move_to(&["r"], Place::LastChild { key: "c".into() }, None)
            ),
            Err(Reason::Cycle)
        );
        assert_eq!(
            run(
                &[lap],
                move_to(&["c"], Place::After { key: "r".into() }, None)
            ),
            Err(Reason::Incomplete)
        );
    }

    #[test]
    fn a_step_and_a_selection_differ_at_a_child_list_edge() {
        let p = page(
            "P",
            &[
                ("a", "a", &["a1"]),
                ("a1", "a1", &[]),
                ("b", "b", &["b1"]),
                ("b1", "b1", &[]),
            ],
            &["a", "b"],
        );
        let step = |within| Verb::MoveUpDown {
            keys: vec!["a1".into()],
            up: false,
            within_siblings: within,
        };
        let change = run(&[p.clone()], step(false)).unwrap();
        let b = change.pages[0]
            .upserts
            .iter()
            .find(|u| u.key == "b")
            .unwrap();
        assert_eq!(b.children, ["a1", "b1"]);
        assert_eq!(run(&[p], step(true)), Err(Reason::NoOutlineStep));
    }

    /// M1 review F2: a selection through an embed or a reference list can list
    /// a block and a deeper descendant of it, or one block twice; the old
    /// `deleteSelection` deleted the outer block and skipped the rest.
    #[test]
    fn delete_takes_nested_and_repeated_keys_with_the_outer_block() {
        let keys = |ks: &[&str]| Verb::Delete {
            keys: ks.iter().map(|k| k.to_string()).collect(),
        };
        for ks in [&["d0", "d2"][..], &["d2", "d0"], &["d1", "d1"]] {
            let change = run(&[chain(4)], keys(ks)).unwrap();
            let mut deleted = change.pages[0].deleted.clone();
            deleted.sort();
            let expect: &[&str] = if ks[0] == "d1" {
                &["d1", "d2", "d3"]
            } else {
                &["d0", "d1", "d2", "d3"]
            };
            assert_eq!(deleted, expect, "keys {ks:?}");
        }
    }

    /// A partial page (DESIGN §5.3) refuses what it cannot see: a block whose
    /// parent was not given would read as a root, and a subtree cut short
    /// would read as a leaf (the depth check) or be left behind (delete).
    #[test]
    fn a_partial_page_refuses_what_it_was_not_given() {
        let without = |p: &OutlinePage, gone: &[&str]| {
            let mut p = p.clone();
            for k in gone {
                p.blocks.remove(*k);
            }
            p
        };
        let keys = |ks: &[&str]| ks.iter().map(|k| k.to_string()).collect::<Vec<_>>();
        // b > c; d's parent b is not given.
        let p = page(
            "P",
            &[
                ("a", "a", &[]),
                ("b", "b", &["c", "d"]),
                ("c", "c", &[]),
                ("d", "d", &[]),
            ],
            &["a", "b"],
        );
        let indent = |ks: &[&str]| Verb::Indent { keys: keys(ks) };
        assert!(run(&[p.clone()], indent(&["d"])).is_ok());
        assert_eq!(
            run(&[without(&p, &["b"])], indent(&["d"])),
            Err(Reason::Incomplete)
        );
        // The chain under d0 reaches the cap only through its last two blocks:
        // without them it must not read as fitting.
        let mut deep = chain(OUTLINE_MAX_DEPTH);
        deep.roots.insert(0, "top".into());
        deep.blocks.insert(
            "top".into(),
            Block {
                raw: "top".into(),
                children: vec![],
            },
        );
        assert_eq!(run(&[deep.clone()], indent(&["d0"])), Err(Reason::DepthCap));
        let last = format!("d{}", OUTLINE_MAX_DEPTH - 1);
        let above = format!("d{}", OUTLINE_MAX_DEPTH - 2);
        assert_eq!(
            run(&[without(&deep, &[&above, &last])], indent(&["d0"])),
            Err(Reason::Incomplete)
        );
        let delete = Verb::Delete {
            keys: keys(&["d1"]),
        };
        assert_eq!(
            run(&[without(&chain(4), &["d3"])], delete),
            Err(Reason::Incomplete)
        );
    }

    /// M1 review F3: the page-header block refuses children wherever it sits,
    /// not only as the first root.
    #[test]
    fn nothing_indents_under_the_header_block_after_it_moved() {
        let mut p = page(
            "P",
            &[("a", "a", &[]), ("h", "title:: x", &[]), ("b", "b", &[])],
            &["a", "h", "b"],
        );
        p.header_root = Some("h".into());
        let indent = Verb::Indent {
            keys: vec!["b".into()],
        };
        assert_eq!(run(&[p], indent), Err(Reason::HeaderRoot));
    }

    /// GH #210 F1: a page whose first root is its materialized page header.
    fn header_page() -> OutlinePage {
        let mut p = page(
            "P",
            &[
                ("h", "alias:: a\ntags:: b", &[]),
                ("b", "body", &["c"]),
                ("c", "child", &[]),
            ],
            &["h", "b"],
        );
        p.header_root = Some("h".into());
        p
    }

    fn split_at(key: &str, raw_offset: usize, force_child: bool) -> Verb {
        Verb::Split {
            key: key.into(),
            raw_offset,
            force_child,
            children_visible: true,
            keep_start_in_scope: false,
        }
    }

    /// GH #210 F1: Enter inside the page header never moves it below a new
    /// block or carries a property out of it: it is OG's in-field newline
    /// (`dwim-in-properties`). Before the fix offset 0 put an empty block above
    /// the header and offset 10 moved `tags:: b` into the outline, which the
    /// store refuses to save.
    #[test]
    fn enter_inside_the_page_header_is_a_newline_in_it() {
        for (offset, raw, caret) in [
            (0, "\nalias:: a\ntags:: b", 1),
            (3, "ali\nas:: a\ntags:: b", 4),
            (9, "alias:: a\n\ntags:: b", 10),
            (10, "alias:: a\n\ntags:: b", 11),
        ] {
            let change = run(&[header_page()], split_at("h", offset, false)).unwrap();
            let patch = &change.pages[0];
            assert_eq!(patch.roots, None, "offset {offset}: no block is added");
            assert_eq!(patch.upserts.len(), 1, "offset {offset}");
            assert_eq!(patch.upserts[0].key, "h");
            assert_eq!(patch.upserts[0].raw, raw, "offset {offset}");
            assert!(patch.upserts[0].children.is_empty());
            assert_eq!(
                change.focus,
                Some(Focus {
                    key: "h".into(),
                    raw_offset: Some(caret)
                })
            );
        }
    }

    /// Enter at the very end of the header leaves it, into a new block below,
    /// and never a child of it (GH #638), whatever the caller asked.
    #[test]
    fn enter_at_the_end_of_the_page_header_adds_a_block_below_it() {
        for force_child in [false, true] {
            let change = run(&[header_page()], split_at("h", 18, force_child)).unwrap();
            let patch = &change.pages[0];
            assert_eq!(
                patch.roots.as_deref(),
                Some(&["h".to_string(), "n0".to_string(), "b".to_string()][..])
            );
            let h = patch.upserts.iter().find(|u| u.key == "h").unwrap();
            assert_eq!(h.raw, "alias:: a\ntags:: b");
            assert!(h.children.is_empty());
        }
    }

    /// A hidden property of the header stays hidden and in place; the caret
    /// lands after the inserted newline.
    #[test]
    fn enter_in_a_header_with_hidden_properties_keeps_them() {
        let mut p = header_page();
        p.blocks.get_mut("h").unwrap().raw = "alias:: a\ncollapsed:: true".into();
        let change = run(&[p], split_at("h", 0, false)).unwrap();
        let h = &change.pages[0].upserts[0];
        assert_eq!(h.raw, "\nalias:: a\ncollapsed:: true");
        assert_eq!(change.focus.unwrap().raw_offset, Some(1));
    }

    /// No verb puts a block above the page header or moves it: each would
    /// leave page properties in the outline, which the store refuses to save
    /// (GH #210 F1). Before the fix all of these applied.
    #[test]
    fn no_verb_puts_a_block_above_the_page_header_or_under_it() {
        let verbs = [
            Verb::MoveUpDown {
                keys: vec!["b".into()],
                up: true,
                within_siblings: false,
            },
            Verb::MoveUpDown {
                keys: vec!["b".into()],
                up: true,
                within_siblings: true,
            },
            Verb::MoveUpDown {
                keys: vec!["h".into()],
                up: false,
                within_siblings: false,
            },
            Verb::MoveTo {
                keys: vec!["b".into()],
                to: Place::Before { key: "h".into() },
                inherit_from: None,
            },
            Verb::MoveTo {
                keys: vec!["c".into()],
                to: Place::Slot {
                    parent: None,
                    page: "P".into(),
                    index: 0,
                },
                inherit_from: None,
            },
        ];
        for verb in verbs {
            assert_eq!(
                run(&[header_page()], verb.clone()),
                Err(Reason::HeaderRoot),
                "{verb:?}"
            );
        }
        // Ordinary edits below the header, and deleting it, still apply.
        for verb in [
            Verb::Outdent {
                keys: vec!["c".into()],
                logical: false,
            },
            Verb::MoveTo {
                keys: vec!["c".into()],
                to: Place::After { key: "h".into() },
                inherit_from: None,
            },
            Verb::Delete {
                keys: vec!["h".into()],
            },
            split_at("b", 0, false),
        ] {
            assert!(run(&[header_page()], verb.clone()).is_ok(), "{verb:?}");
        }
    }

    /// A header already pushed below another block (an older build's state) is
    /// not held in place, so the user can repair the page by moving or deleting.
    #[test]
    fn a_header_that_is_no_longer_first_is_not_held() {
        let mut p = page(
            "P",
            &[("e", "", &[]), ("h", "alias:: a", &[]), ("b", "b", &[])],
            &["e", "h", "b"],
        );
        p.header_root = Some("h".into());
        assert!(run(
            &[p.clone()],
            Verb::Delete {
                keys: vec!["e".into()]
            }
        )
        .is_ok());
        assert!(run(
            &[p],
            Verb::MoveUpDown {
                keys: vec!["h".into()],
                up: true,
                within_siblings: false
            }
        )
        .is_ok());
    }
}
