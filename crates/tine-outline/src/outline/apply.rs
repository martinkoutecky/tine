use super::*;

// Merging visible text must not discard properties newly removed from that
// text. Identity/fold/list/comment metadata retain their existing merge rules.
pub(super) fn retained_hidden_properties(raw: &str, org: bool, hide: &HidePolicy) -> String {
    let regions = block_regions::parse(raw, org);
    let comment = editor_text::comment_quote(
        regions
            .properties
            .iter()
            .filter(|p| p.primary)
            .map(|p| (p.key.as_str(), p.value.as_str())),
    )
    .is_some();
    regions
        .properties
        .iter()
        .filter(|p| {
            p.primary
                && hide.hides_for_comment(&p.key, comment)
                && !HidePolicy::Structural.hides_for_comment(&p.key, comment)
        })
        .map(|p| p.line.slice(raw).trim_end_matches('\n'))
        .collect::<Vec<_>>()
        .join("\n")
}

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
