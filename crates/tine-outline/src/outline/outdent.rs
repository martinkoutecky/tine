use super::*;
use std::collections::HashSet;

/// Shift+Tab: keys land after their parent. OG `indent-outdent-blocks`
/// (`outliner/core.cljs:835-852`, 6e7afa8eb) takes following siblings along in
/// traditional mode. Tine's editor supplies annotation siblings to retain with
/// their owner (DESIGN §1, GH #891); CLI/default behavior remains unchanged.
pub(super) fn outdent(
    cx: &mut Cx,
    keys: &[Key],
    logical: bool,
    retain: &[Key],
) -> Result<(), Reason> {
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
        let retain: HashSet<_> = retain.iter().collect();
        let (kept, following): (Vec<_>, Vec<_>) =
            following.into_iter().partition(|k| retain.contains(k));
        children.extend(kept);
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
