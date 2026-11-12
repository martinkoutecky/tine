//! Native outline shape and origin counts, without copying body text.
use tine_core::{model::BlockDto, DocBlock};

/// Count blocks when native child shapes agree; `None` on mismatch.
/// O(blocks), no text clones or I/O.
pub(super) fn same_shape<'a>(
    expected: impl IntoIterator<Item = &'a BlockDto>,
    actual: &[DocBlock],
) -> Option<usize> {
    let mut expected = expected.into_iter();
    let mut count = 0;
    for block in actual {
        let original = expected.next()?;
        count += 1 + same_shape(&original.children, &block.children)?;
    }
    expected.next().is_none().then_some(count)
}
