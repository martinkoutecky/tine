//! Ordered-list glyphs, shared by the live app and static export (I-12).
//! Context traversal stays with each tree adapter; punctuation is presentation.

/// `1 → a`, `2 → b`, `27 → aa` (`toLetters`).
pub fn letters(mut n: u32) -> String {
    let mut s = Vec::new();
    while n > 0 {
        s.insert(0, (b'a' + ((n - 1) % 26) as u8) as char);
        n = (n - 1) / 26;
    }
    s.into_iter().collect()
}

/// `1 → i`, `4 → iv`, … (`toRoman`).
pub fn roman(mut n: u32) -> String {
    const MAP: [(u32, &str); 13] = [
        (1000, "m"),
        (900, "cm"),
        (500, "d"),
        (400, "cd"),
        (100, "c"),
        (90, "xc"),
        (50, "l"),
        (40, "xl"),
        (10, "x"),
        (9, "ix"),
        (5, "v"),
        (4, "iv"),
        (1, "i"),
    ];
    let mut s = String::new();
    for (v, sym) in MAP {
        while n >= v {
            s.push_str(sym);
            n -= v;
        }
    }
    s
}

/// Positive sibling-run index and consecutive ordered-parent depth.
pub fn glyph(index: u32, depth: u32) -> String {
    match depth % 3 {
        0 => index.to_string(),
        1 => letters(index),
        _ => roman(index),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn ordered_depth_cycle_and_long_runs() {
        assert_eq!(glyph(27, 1), "aa");
        assert_eq!(glyph(1994, 2), "mcmxciv");
        assert_eq!(glyph(4, 3), "4");
    }
}
