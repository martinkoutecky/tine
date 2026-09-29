//! Fence-marker recognition used by document parsing and projections.

/// Parse a file's contents into a [`Document`].
/// The fence marker at the start of a line: `(char, run-length)` for a run of >=3
/// backticks or tildes (leading whitespace ignored); else `None`.
pub(crate) fn fence_marker(text: &str) -> Option<(char, usize)> {
    let t = text.trim_start();
    let c = t.chars().next()?;
    if c != '`' && c != '~' {
        return None;
    }
    let n = t.chars().take_while(|&x| x == c).count();
    (n >= 3).then_some((c, n))
}

/// Given the current open fence (if any) and a line, return the new fence state:
/// open on the first valid marker, close only on a matching one (same char, >=
/// the opener's length). Shared by the block parser, `property_lines`, and
/// `visible_lines` so "inside a code fence?" is decided one way.
pub(crate) fn next_fence(cur: Option<(char, usize)>, line: &str) -> Option<(char, usize)> {
    match cur {
        None => fence_marker(line),
        Some((c, n)) => match fence_marker(line) {
            Some((c2, n2)) if c2 == c && n2 >= n => None, // closing fence
            _ => Some((c, n)),                            // still inside
        },
    }
}
