//! `line_terminator` reads a line's terminator where the one line lexer
//! (`tine_text::lines`) ends the line, and the reader's normalization keeps
//! the same lines.
use super::{line_terminator, normalize_line_endings};

#[test]
fn line_terminators_are_the_lexers_breaks() {
    for text in [
        "",
        "a",
        "a\n",
        "a\r",
        "a\r\n",
        "a\rb\nc\r\nd",
        "\r\r\n",
        "\n\r",
        "\r\n\r\n",
        "a\r\rb",
        "- x\r  y\r- z",
    ] {
        let lines: Vec<_> = tine_text::lines(text).collect();
        let mut at = 0;
        for (body, brk) in &lines {
            assert_eq!(body.start, at, "{text:?}");
            at = body.end + brk.as_str().len();
            assert_eq!(line_terminator(&text[body.start..at]), brk.as_str());
        }
        assert_eq!(at, text.len(), "{text:?}");
        // The reader's LF form has the same lines.
        let lf = normalize_line_endings(text);
        assert_eq!(lines.len(), lf.split_inclusive('\n').count(), "{text:?}");
    }
    assert_eq!(line_terminator("x\r"), "\r");
    assert_eq!(line_terminator("x\r\n"), "\r\n");
    assert_eq!(line_terminator("x\n"), "\n");
    assert_eq!(line_terminator("x"), "");
}
