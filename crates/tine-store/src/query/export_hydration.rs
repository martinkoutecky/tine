//! Source ancestry for detached export roots; no index or DTO tree is built.
use tine_core::doc::DocBlock;

/// Preorder hydration walk with parser-owned thread membership. O(visited
/// blocks), O(depth) borrowed traversal state, no I/O or document mutation.
pub(super) fn blocks_with_comment_context(
    blocks: &[DocBlock],
) -> impl Iterator<Item = (&DocBlock, bool)> {
    let mut stack = vec![(blocks.iter(), false, false)];
    std::iter::from_fn(move || loop {
        let (siblings, thread, has_parent) = stack.last_mut()?;
        let Some(block) = siblings.next() else {
            stack.pop();
            continue;
        };
        let comment = *thread
            || (*has_parent
                && tine_core::editor_text::comment_quote(
                    block
                        .projection()
                        .properties()
                        .iter()
                        .map(|(k, v)| (k.as_str(), v.as_str())),
                )
                .is_some());
        if !block.children.is_empty() {
            stack.push((block.children.iter(), comment, true));
        }
        return Some((block, comment));
    })
}
