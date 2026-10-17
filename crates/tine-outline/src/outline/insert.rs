//! Imported-forest insertion, shared by the window and command line.
use super::*;

#[cfg(test)]
mod tests {
    use super::*;

    fn page() -> OutlinePage {
        OutlinePage {
            name: "P".into(),
            format: Format::Md,
            pre_block: None,
            roots: vec!["a".into()],
            blocks: HashMap::from([(
                "a".into(),
                Block {
                    raw: "anchor".into(),
                    children: vec![],
                },
            )]),
            header_root: None,
            read_only: false,
            frozen: false,
        }
    }

    #[test]
    fn imported_forests_check_depth_before_creating_any_blocks() {
        let mut node = OutlineNode {
            raw: "leaf".into(),
            children: vec![],
        };
        for _ in 1..OUTLINE_MAX_DEPTH {
            node = OutlineNode {
                raw: "parent".into(),
                children: vec![node],
            };
        }
        let mut verb = Verb::InsertOutline {
            place: Place::After { key: "a".into() },
            blocks: vec![node],
            live_ids: vec![],
            reuse: None,
        };
        assert_eq!(
            apply(&[page()], &verb, &Default::default())
                .unwrap()
                .keys_used,
            128
        );
        if let Verb::InsertOutline { place, .. } = &mut verb {
            *place = Place::LastChild { key: "a".into() };
        }
        assert_eq!(
            apply(&[page()], &verb, &Default::default()),
            Err(Reason::DepthCap)
        );
    }

    #[test]
    fn insertion_checks_writability_and_graph_identity_context() {
        let uuid = "6512bd43-d9ca-4e3b-9f7a-1a2b3c4d5e6f";
        let verb = Verb::InsertOutline {
            place: Place::After { key: "a".into() },
            blocks: vec![OutlineNode {
                raw: format!("new\nid:: {uuid}"),
                children: vec![],
            }],
            live_ids: vec![uuid.to_uppercase()],
            reuse: None,
        };
        let mut p = page();
        p.frozen = true;
        assert_eq!(
            apply(&[p.clone()], &verb, &Default::default()),
            Err(Reason::Frozen)
        );
        p.frozen = false;
        p.read_only = true;
        assert_eq!(
            apply(&[p.clone()], &verb, &Default::default()),
            Err(Reason::ReadOnly)
        );
        p.read_only = false;
        let change = apply(&[p], &verb, &Default::default()).unwrap();
        assert!(change.pages[0].upserts.iter().any(|b| b.raw == "new"));
    }
}

/// The window's createOutline / insertOutlineBeside / insertOutlineChildren.
/// Keys are allocated in pre-order; identity collision checks follow the old
/// child-first import order. Every descendant inherits from the same anchor.
pub(super) fn insert_outline(
    cx: &mut Cx,
    place: &Place,
    nodes: &[OutlineNode],
    live_ids: &[String],
    reuse: Option<&str>,
) -> Result<(), Reason> {
    let (t, parent, anchor) = match place {
        Place::Before { key } | Place::After { key } => {
            let t = cx.tx_of(key)?;
            (
                t,
                cx.txs[t].parent_of(key).map(str::to_string),
                Some(key.as_str()),
            )
        }
        Place::LastChild { key } => (cx.tx_of(key)?, Some(key.clone()), Some(key.as_str())),
        Place::Slot { parent, page, .. } => {
            (cx.tx_of_page(page)?, parent.clone(), parent.as_deref())
        }
    };
    cx.writable(t)?;
    let first = parent.as_deref().map_or(0, |p| cx.txs[t].depth_of(p) + 1);
    let mut pending: Vec<_> = nodes.iter().map(|n| (n, first)).collect();
    while let Some((node, depth)) = pending.pop() {
        if depth >= OUTLINE_MAX_DEPTH {
            return Err(Reason::DepthCap);
        }
        pending.extend(node.children.iter().map(|n| (n, depth + 1)));
    }
    let list = cx.txs[t].siblings(parent.as_deref())?;
    let at = match place {
        Place::Before { key } | Place::After { key } => {
            list.iter()
                .position(|k| k == key)
                .ok_or(Reason::Incomplete)?
                + usize::from(matches!(place, Place::After { .. }))
        }
        Place::LastChild { .. } => list.len(),
        Place::Slot { index, .. } if *index <= list.len() => *index,
        _ => return Err(Reason::Incomplete),
    };
    let format = cx.txs[t].base.format;
    let inherited = anchor
        .map(|k| {
            let b = cx.txs[t].block(k).ok_or(Reason::Incomplete)?;
            Ok(own_property(&b.raw, format, ORDER_LIST_KEY, false))
        })
        .transpose()?
        .flatten();
    let hidden = if let Some(key) = reuse {
        let b = cx.txs[t].block(key).ok_or(Reason::Incomplete)?;
        let text = editor_text::split(&b.raw, format.is_org(), &HidePolicy::Builtin);
        if nodes.is_empty()
            || !b.children.is_empty()
            || !js_blank(&text.visible)
            || list.get(at).map(String::as_str) != Some(key)
        {
            return Err(Reason::Incomplete);
        }
        Some(text.hidden)
    } else {
        None
    };
    let mut ids: std::collections::HashSet<String> =
        live_ids.iter().map(|id| id.to_lowercase()).collect();
    for page in cx.pages {
        for block in page.blocks.values() {
            if let Some(id) = block_regions::parse(&block.raw, page.format.is_org()).id {
                ids.insert(id.value.trim().to_lowercase());
            }
        }
    }
    fn create(
        cx: &mut Cx,
        t: usize,
        node: &OutlineNode,
        inherited: Option<&str>,
        ids: &mut std::collections::HashSet<String>,
        host: Option<(&str, &str)>,
    ) -> Result<Key, Reason> {
        let key = host.map_or_else(|| cx.fresh_key(), |(key, _)| key.to_string());
        if host.is_none() && cx.pages.iter().any(|p| p.blocks.contains_key(&key)) {
            return Err(Reason::Incomplete);
        }
        let mut children = Vec::new();
        for child in &node.children {
            children.push(create(cx, t, child, inherited, ids, None)?);
        }
        let format = cx.txs[t].base.format;
        let mut raw = node.raw.clone();
        if let Some(id) = block_regions::parse(&raw, format.is_org()).id {
            if !ids.insert(id.value.trim().to_lowercase()) {
                raw = editor_text::split(
                    &raw,
                    format.is_org(),
                    &HidePolicy::Keys {
                        keys: vec!["id".into()],
                        norm: false,
                    },
                )
                .visible;
            }
        }
        if let Some((_, hidden)) = host {
            raw = join(&raw, hidden, format)?;
        }
        if let Some(value) = inherited {
            if own_property(&raw, format, ORDER_LIST_KEY, true).is_none() {
                raw = raw_with_order_list_type(&raw, Some(value), format)?;
            }
        }
        let collapsed = block_regions::parse(&raw, format.is_org())
            .properties
            .iter()
            .any(|p| p.primary && p.key.eq_ignore_ascii_case("collapsed") && p.value == "true");
        cx.txs[t].insert(key.clone(), Block { raw, children });
        cx.txs[t].collapsed.insert(key.clone(), collapsed);
        Ok(key)
    }
    let mut roots = Vec::new();
    for (index, node) in nodes.iter().enumerate() {
        let host = if index == 0 {
            reuse.zip(hidden.as_deref())
        } else {
            None
        };
        roots.push(create(cx, t, node, inherited.as_deref(), &mut ids, host)?);
    }
    if roots.is_empty() {
        return Ok(());
    }
    let focus = if reuse.is_none() && matches!(place, Place::Before { .. }) {
        roots.first()
    } else {
        roots.last()
    }
    .cloned();
    cx.txs[t]
        .siblings_mut(parent.as_deref())?
        .splice(at..at + usize::from(reuse.is_some()), roots);
    cx.txs[t].kind(if reuse.is_some() {
        &[EditKind::SaveBlock, EditKind::InsertBlocks]
    } else {
        &[EditKind::InsertBlocks]
    });
    cx.focus = focus.map(|key| Focus {
        key,
        raw_offset: None,
    });
    Ok(())
}
