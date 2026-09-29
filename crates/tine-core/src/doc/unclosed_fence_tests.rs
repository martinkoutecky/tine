use super::*;

#[test]
fn unclosed_fence_does_not_swallow_a_following_bullet() {
    // mldoc 1.5.7 treats the unclosed opener as a paragraph; the following
    // bullet remains an outline block. This also covers an inserted block
    // after a pre-existing unclosed fence on the next save/reparse.
    let source = "- open\n  ```js\n  code\n- after\n";
    let parsed = parse(source);
    assert_eq!(parsed.roots.len(), 2, "{parsed:?}");
    assert_eq!(parsed.roots[1].raw(), "after");
}
