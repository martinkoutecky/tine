use super::*;

/// Run a verb with the host's graph-configured editor projection. No graph
/// reads or mutable global policy: every offset and mutation sees one snapshot.
pub fn apply_with_policy(
    pages: &[OutlinePage],
    verb: &Verb,
    keys: &KeySource,
    hide: &HidePolicy,
) -> Result<Change, Reason> {
    let mut cx = Cx {
        pages,
        hide,
        txs: Vec::new(),
        keys: keys.clone(),
        used: 0,
        focus: None,
        notices: Vec::new(),
        transfers: Vec::new(),
    };
    match verb {
        Verb::InsertOutline {
            place,
            blocks,
            live_ids,
            reuse,
        } => insert_outline(&mut cx, place, blocks, live_ids, reuse.as_deref())?,
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
            survivor,
            absorbed,
            absorbed_referenced,
        } => merge(&mut cx, survivor, absorbed, *absorbed_referenced)?,
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
