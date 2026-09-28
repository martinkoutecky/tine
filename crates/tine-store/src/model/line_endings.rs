//! Line terminators at the Markdown write boundary (K01a, I-4).
//!
//! The model is LF-canonical: `doc::parse` ends a line at `\r\n`, `\n` or a
//! lone `\r` (mldoc's `eol_chars`). A save must still write the file's own
//! terminators back: a reused line keeps its original terminator
//! (`layout_retention`), and a new line takes the file's convention.

/// Split `source` into lines and the terminator after each line, as the LF
/// form's `body.split('\n')` would (one trailing terminator is not a line of
/// its own). The last line's terminator is that trailing terminator, or `""`.
/// Lines never contain `\r` or `\n`. O(n), no copy of the text.
pub(super) fn split(source: &str) -> (Vec<&str>, Vec<&'static str>) {
    let (mut lines, mut ends) = (Vec::new(), Vec::new());
    let bytes = source.as_bytes();
    let (mut start, mut i) = (0, 0);
    while i < bytes.len() {
        let end = match bytes[i] {
            b'\n' => "\n",
            b'\r' if bytes.get(i + 1) == Some(&b'\n') => "\r\n",
            b'\r' => "\r",
            _ => {
                i += 1;
                continue;
            }
        };
        lines.push(&source[start..i]);
        ends.push(end);
        i += end.len();
        start = i;
    }
    if start < bytes.len() || lines.is_empty() {
        lines.push(&source[start..]);
        ends.push("");
    }
    if lines == [""] {
        // An empty body (`""`, or one lone terminator) has no lines.
        lines.clear();
        ends.clear();
    }
    (lines, ends)
}

/// The terminator a new line gets in `existing`: CRLF when the file has any
/// (as before K01a), else `\r` when it has a lone one, else LF (new files too).
pub(super) fn convention(existing: Option<&str>) -> &'static str {
    match existing {
        Some(e) if e.contains("\r\n") => "\r\n",
        Some(e) if e.contains('\r') => "\r",
        _ => "\n",
    }
}

/// Give LF `content` the existing file's convention, unless it already carries
/// its own terminators (reused disk bytes, `layout_retention` output). A
/// whole-page rewrite of a mixed-ending file therefore uses one convention.
pub(super) fn restore(content: String, existing: Option<&str>) -> String {
    let ending = convention(existing);
    if ending != "\n" && !content.contains('\r') {
        content.replace('\n', ending)
    } else {
        content
    }
}

/// Put back the lone `\r` terminators of an org file (K01a). The org model
/// reads a lone `\r` as `\n` and keeps a CRLF's `\r` in the line text, so
/// `serialized` is LF-separated. Lines equal to the old file's leading and
/// trailing lines keep their own terminators; a changed stretch takes the old
/// terminators position by position (an edited block keeps its own), and
/// lines beyond it the file's convention. Files without a lone `\r` are
/// returned unchanged. O(lines), one output copy.
pub(super) fn restore_org(serialized: String, existing: Option<&str>) -> String {
    let Some(existing) = existing.filter(|e| e.replace("\r\n", "").contains('\r')) else {
        return serialized;
    };
    let (mut old, mut ends, mut start) = (Vec::new(), Vec::new(), 0);
    let bytes = existing.as_bytes();
    for (i, &byte) in bytes.iter().enumerate() {
        let lone = byte == b'\r' && bytes.get(i + 1) != Some(&b'\n');
        if byte == b'\n' || lone {
            old.push(&existing[start..i]);
            ends.push(if lone { "\r" } else { "\n" });
            start = i + 1;
        }
    }
    old.push(&existing[start..]);
    let new: Vec<&str> = serialized.split('\n').collect();
    let fallback = if existing.contains("\r\n") {
        "\r\n"
    } else {
        "\r"
    };
    let prefix = old.iter().zip(&new).take_while(|(a, b)| a == b).count();
    let room = old.len().min(new.len()) - prefix;
    let suffix = (old.iter().rev().zip(new.iter().rev()))
        .take(room)
        .take_while(|(a, b)| a == b)
        .count();
    let mut out = String::with_capacity(serialized.len());
    for (j, line) in new.iter().enumerate() {
        out.push_str(line);
        if j + 1 == new.len() {
            break;
        }
        let k = if j >= new.len() - suffix {
            Some(j + old.len() - new.len())
        } else {
            (j < old.len() - suffix).then_some(j)
        };
        out.push_str(k.and_then(|k| ends.get(k).copied()).unwrap_or(fallback));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn split_matches_the_lf_body_split() {
        for lf in [
            "",
            "a",
            "a\n",
            "a\nb",
            "a\nb\n",
            "a\n\n",
            "\n",
            "a\n\nb\n\n\n",
        ] {
            let body = lf.strip_suffix('\n').unwrap_or(lf);
            let expected: Vec<&str> = if body.is_empty() {
                Vec::new()
            } else {
                body.split('\n').collect()
            };
            for ending in ["\n", "\r\n", "\r"] {
                let source = lf.replace('\n', ending);
                let (lines, ends) = split(&source);
                assert_eq!(lines, expected, "{source:?}");
                let rebuilt: String = lines
                    .iter()
                    .zip(&ends)
                    .map(|(l, e)| format!("{l}{e}"))
                    .collect();
                if !(lf.is_empty() || lf == "\n") {
                    assert_eq!(rebuilt, source, "{source:?}");
                }
            }
        }
        assert_eq!(
            split("a\r\nb\rc\n"),
            (vec!["a", "b", "c"], vec!["\r\n", "\r", "\n"])
        );
    }

    #[test]
    fn convention_and_restore() {
        assert_eq!(convention(None), "\n");
        assert_eq!(convention(Some("a\rb\r\n")), "\r\n");
        assert_eq!(convention(Some("a\rb\n")), "\r");
        assert_eq!(restore("a\nb\n".into(), Some("x\ry")), "a\rb\r");
        assert_eq!(restore("a\nb\n".into(), Some("x\r\ny")), "a\r\nb\r\n");
        assert_eq!(restore("a\rb\n".into(), Some("x\ry")), "a\rb\n");
    }

    #[test]
    fn restore_org_keeps_each_terminator() {
        let lf = |s: &str| s.to_string();
        assert_eq!(restore_org(lf("* a\n* b\n"), Some("* a\r\n")), "* a\n* b\n");
        assert_eq!(
            restore_org(lf("* a\n* b\n"), Some("* a\r* b\r")),
            "* a\r* b\r"
        );
        assert_eq!(restore_org(lf("* a!\n* b"), Some("* a\r* b")), "* a!\r* b");
        assert_eq!(
            restore_org(lf("* a\n* n\n* b\n"), Some("* a\n* b\r")),
            "* a\n* n\r* b\r",
            "a new line takes the convention"
        );
        assert_eq!(
            restore_org(lf("* x\n* b\n"), Some("* a\n* b\r")),
            "* x\n* b\r",
            "an edited line keeps its own terminator"
        );
    }
}
