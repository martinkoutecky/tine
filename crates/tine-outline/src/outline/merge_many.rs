//! Merge a run of selected blocks into the first one (GH #764).
use super::*;
use std::collections::HashSet;

/// Append each of `absorbed`, in the order given, to `survivor`, one line break
/// between the texts, and remove them. It is [`merge`] applied to every absorbed
/// block, with the same rules, but as one verb: a merge reads parents from the
/// page as it stood before the verb, so a selection that holds a block and its
/// own child cannot be done as a chain of single merges.
///
/// * Text: the survivor's visible text, then each absorbed block's, a line break
///   between two non-empty texts. The survivor keeps its key and hidden
///   properties; an absorbed block's hidden rows and clocks are dropped.
/// * Identity: a referenced `id::` moves to the survivor while it has none;
///   once it has one, the next referenced `id::` refuses the whole verb
///   ([`Reason::ReferencedIdentity`]), so no reference target is ever lost.
///   An unreferenced `id::` goes with its block, as in Backspace.
/// * Children: the survivor's own children first, then each absorbed block's
///   children in turn, skipping absorbed blocks. A child that is not absorbed
///   keeps its whole subtree.
/// * Refuses: blocks on two pages ([`Reason::CrossPageMerge`]), a survivor
///   inside an absorbed block ([`Reason::Cycle`]), a moved subtree past the
///   depth cap, a read-only page.
///
/// The focus is the survivor, at the end of the joined text.
pub(super) fn merge_many(
    cx: &mut Cx,
    survivor: &str,
    absorbed: &[Key],
    referenced: &[Key],
) -> Result<(), Reason> {
    let mut order: Vec<Key> = Vec::with_capacity(absorbed.len());
    let mut gone: HashSet<&str> = HashSet::new();
    for key in absorbed {
        if gone.insert(key.as_str()) {
            order.push(key.clone());
        }
    }
    if order.is_empty() {
        return Err(Reason::Incomplete);
    }
    if gone.contains(survivor) {
        return Err(Reason::Cycle);
    }
    let t = cx.tx_of(survivor)?;
    for key in &order {
        if cx.tx_of(key)? != t {
            return Err(Reason::CrossPageMerge);
        }
    }
    cx.writable(t)?;
    let format = cx.txs[t].base.format;
    let tx = &mut cx.txs[t];
    // The survivor may not sit inside a block that goes.
    if order.iter().any(|key| tx.in_subtree(key, survivor)) {
        return Err(Reason::Cycle);
    }
    // Children that move under the survivor, in reading order.
    let mut moved: Vec<Key> = Vec::new();
    for key in &order {
        let block = tx.block(key).ok_or(Reason::Incomplete)?;
        moved.extend(
            block
                .children
                .iter()
                .filter(|c| !gone.contains(c.as_str()))
                .cloned(),
        );
    }
    let first = tx.depth_of(survivor) + 1;
    if !all_fit(&moved, |c| tx.fits_from(c, first))? {
        return Err(Reason::DepthCap);
    }

    let mut raw = tx.block(survivor).ok_or(Reason::Incomplete)?.raw.clone();
    let mut visible_len = 0;
    let mut transfers: Vec<Key> = Vec::new();
    for key in &order {
        let lost_raw = &tx.block(key).ok_or(Reason::Incomplete)?.raw;
        let keep = editor_text::split(&raw, format.is_org(), cx.hide);
        let lost = editor_text::split(lost_raw, format.is_org(), cx.hide);
        let survivor_has_id = block_regions::parse(&raw, format.is_org())
            .id
            .is_some_and(|p| !p.value.trim().is_empty());
        let absorbed_id = block_regions::parse(lost_raw, format.is_org())
            .id
            .filter(|p| !p.value.trim().is_empty())
            .map(|p| js_trim(p.line.slice(lost_raw)).to_string());
        let is_referenced = referenced.contains(key);
        if is_referenced && absorbed_id.is_some() && survivor_has_id {
            return Err(Reason::ReferencedIdentity);
        }
        let mut hidden = keep.hidden.clone();
        if let (true, Some(id)) = (is_referenced, &absorbed_id) {
            hidden = if hidden.is_empty() {
                id.clone()
            } else {
                format!("{hidden}\n{id}")
            };
            transfers.push(key.clone());
        }
        let line_break = if keep.visible.is_empty() || lost.visible.is_empty() {
            ""
        } else {
            "\n"
        };
        let visible = format!("{}{line_break}{}", keep.visible, lost.visible);
        visible_len = visible.len();
        raw = join(&visible, &hidden, format)?;
    }
    for key in transfers {
        cx.notices.push(Notice::IdTransferred {
            from: key,
            to: survivor.to_string(),
        });
    }
    let focus = raw_offset_of_visible(&raw, format, visible_len, cx.hide);

    // Detach every absorbed block from a parent that stays, then rebuild the
    // survivor's children.
    let parents: Vec<Option<&str>> = order.iter().map(|key| tx.parent_of(key)).collect();
    for parent in parents {
        if parent.is_some_and(|p| gone.contains(p)) {
            continue;
        }
        tx.siblings_mut(parent)?
            .retain(|k| !gone.contains(k.as_str()));
    }
    let b = tx.block_mut(survivor)?;
    b.raw = raw;
    b.children.retain(|k| !gone.contains(k.as_str()));
    b.children.extend(moved);
    for key in &order {
        tx.blocks.insert(key.clone(), None);
    }
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
