//! The materialized Markdown page header ([`OutlinePage::header_root`]) stays
//! the page's header through every verb (GH #210 F1).
use super::*;

/// `key` is the page's header and its first root.
pub(super) fn is_first_root_header(tx: &PageTx, key: &str) -> bool {
    tx.base.header_root.as_deref() == Some(key)
        && tx.base.roots.first().map(String::as_str) == Some(key)
}

/// Enter in the page header (`split`): the header is one text field, as OG's
/// page-properties editor is, where Enter on a property line inserts a newline
/// (`dwim-in-properties`, `handler/editor.cljs:2288`, `(insert "\n")`). A
/// split that would put a block above the header or carry any of its text out
/// of it inserts that newline at the caret instead; only Enter with nothing
/// after the caret leaves the header, into a new block below it (never a
/// child). Differs from OG on a non-property line of a header draft, where OG
/// splits: here the split would leave page properties in the outline (see
/// [`header_root_kept`]). `et` and `offset` are the block's editor text and
/// the caret in it.
pub(super) fn newline_in_header(
    cx: &mut Cx,
    t: usize,
    key: &str,
    et: &editor_text::EditorText,
    offset: usize,
    format: Format,
) -> Result<(), Reason> {
    let text = format!("{}\n{}", &et.visible[..offset], &et.visible[offset..]);
    let raw = join(&text, &et.hidden, format)?;
    let regions = block_regions::parse(&raw, format.is_org());
    let caret = editor_text::visible_to_raw_with(
        &raw,
        format.is_org(),
        &regions,
        &HidePolicy::Builtin,
        offset + 1,
    );
    let tx = &mut cx.txs[t];
    tx.block_mut(key)?.raw = raw;
    tx.kind(&[EditKind::SaveBlock]);
    cx.focus = Some(Focus {
        key: key.to_string(),
        raw_offset: Some(caret),
    });
    Ok(())
}

/// The page header stays the page's header (GH #210 F1). A block materialized
/// from an unbulleted Markdown page header ([`OutlinePage::header_root`]) that
/// is the page's first root before a verb is still its first root, and still
/// on the page, after it, unless the verb deletes it. (Nesting a block under
/// the header is the GH #638 family: indent refuses it, a pasted outline may
/// do it (fixture m1), and the window reports and repairs it.) Any other result is the shape the store's page-header firewall
/// refuses to save ("refusing to move page-header property into outline
/// content", GH #163; threat: a frontend edit that would reclassify page
/// properties as outline content, the malformed-input row): the page would then
/// retry forever and no later edit on it would save. So the verb refuses here,
/// before anything changes, with [`Reason::HeaderRoot`], as indenting under the
/// header already does (GH #638). OG has no such state: its Enter on a property
/// line inserts a newline (`dwim-in-properties`), and its writer re-promotes
/// whatever first block carries properties. O(touched pages).
pub(super) fn header_root_kept(cx: &Cx, verb: &Verb) -> Result<(), Reason> {
    for tx in &cx.txs {
        let Some(header) = tx.base.header_root.as_deref() else {
            continue;
        };
        if tx.base.roots.first().map(String::as_str) != Some(header) {
            continue;
        }
        let roots = tx.roots.as_deref().unwrap_or(&tx.base.roots);
        let deleted = matches!(tx.blocks.get(header), Some(None));
        if deleted && matches!(verb, Verb::Delete { .. }) {
            continue;
        }
        if deleted || roots.first().map(String::as_str) != Some(header) {
            return Err(Reason::HeaderRoot);
        }
    }
    Ok(())
}
