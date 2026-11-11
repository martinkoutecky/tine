//! `physical_lines` and `line_terminator`: the physical writers' one answer
//! to "which bytes end a line" agrees with the reader's normalization.
use super::{line_terminator, normalize_line_endings, physical_lines};

#[test]
fn physical_lines_end_where_the_reader_ends_a_line() {
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
        let lines: Vec<_> = physical_lines(text).collect();
        assert_eq!(lines.concat(), text, "{text:?}");
        // Every line but the last is terminated, by exactly one terminator.
        for (n, line) in lines.iter().enumerate() {
            let end = line_terminator(line);
            assert_eq!(
                end.is_empty(),
                n + 1 == lines.len() && !text.ends_with(['\r', '\n'])
            );
            assert!(
                !line[..line.len() - end.len()].contains(['\r', '\n']),
                "{line:?}"
            );
        }
        // The reader's LF form has the same lines.
        let lf = normalize_line_endings(text);
        assert_eq!(lines.len(), lf.split_inclusive('\n').count(), "{text:?}");
    }
    assert_eq!(line_terminator("x\r"), "\r");
    assert_eq!(line_terminator("x\r\n"), "\r\n");
    assert_eq!(line_terminator("x\n"), "\n");
    assert_eq!(line_terminator("x"), "");
}
