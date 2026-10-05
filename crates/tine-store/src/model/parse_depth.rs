//! Pure source depth admission for Markdown and Org page text.

use super::PARSE_INPUT_MAX_DEPTH;
use tine_core::doc;

/// Whether source text is within the parser and renderer nesting ceiling
/// (`PARSE_INPUT_MAX_DEPTH` = 128 levels, inclusive). List-item columns define
/// outline depth; callouts, quotes, and paired inline delimiters have separate
/// ceilings and do not consume outline levels. Fenced code and literal
/// src/example/export/comment bodies are excluded. Org headline levels are
/// checked separately by the path-aware reader. Pure, O(n).
pub(super) fn parse_input_depth_within_limit(input: &str) -> bool {
    let input: &str = &doc::normalize_line_endings(input); // a lone `\r` ends a line (K01a)
    let lines: Vec<_> = input.lines().collect();
    // lsdoc's fence rule, transcribed (`v2/source.rs::is_fence_marker_line`,
    // `block_common::find_matching_fence`): any line whose trimmed text starts
    // with three backticks OR three tildes is a fence marker; a marker line with
    // a LATER marker line opens a fence, and the very next marker line closes it
    // whatever its character or length. (CommonMark's "same character, at least
    // as long" is NOT the parser's rule: a four-backtick opener is closed by a
    // three-backtick line, and the outline after it is real structure.)
    let last_marker = lines.iter().rposition(|line| is_fence_marker_line(line));
    let mut bullet_columns = Vec::new();
    let mut containers = Vec::<String>::new();
    let mut literal: Option<String> = None;
    let mut in_fence = false;
    for (line_index, line) in lines.into_iter().enumerate() {
        let indent = line
            .bytes()
            .take_while(|byte| matches!(byte, b' ' | b'\t'))
            .count();
        let body = &line[indent..];
        let marker_line = is_fence_marker_line(line);
        if in_fence {
            in_fence = !marker_line;
            continue;
        }
        if let Some(open) = literal.as_deref() {
            if body
                .get(..6)
                .is_some_and(|prefix| prefix.eq_ignore_ascii_case("#+END_"))
                .then(|| &body[6..])
                .is_some_and(|name| {
                    name.split_whitespace()
                        .next()
                        .is_some_and(|name| name.eq_ignore_ascii_case(open))
                })
            {
                literal = None;
            }
            continue;
        }
        if marker_line && last_marker.is_some_and(|last| line_index < last) {
            in_fence = true;
            continue;
        }
        let list_item = body == "-"
            || body.starts_with("- ")
            || body == "+"
            || body.starts_with("+ ")
            || body == "*"
            || body.starts_with("* ")
            || body
                .bytes()
                .take_while(|byte| byte.is_ascii_digit())
                .count()
                .checked_add(1)
                .is_some_and(|marker_end| {
                    marker_end > 1
                        && matches!(body.as_bytes().get(marker_end - 1), Some(b'.' | b')'))
                        && matches!(body.as_bytes().get(marker_end), Some(b' '))
                });
        if list_item {
            while bullet_columns
                .last()
                .is_some_and(|column| *column >= indent)
            {
                bullet_columns.pop();
            }
            bullet_columns.push(indent);
            if bullet_columns.len() > PARSE_INPUT_MAX_DEPTH {
                return false;
            }
        }
        // lsdoc builds closed custom/quote callouts as nested Block values,
        // even when every physical line has only two spaces of indentation.
        if body
            .get(..8)
            .is_some_and(|prefix| prefix.eq_ignore_ascii_case("#+BEGIN_"))
        {
            let name = &body[8..];
            let name = name
                .split_whitespace()
                .next()
                .unwrap_or("")
                .to_ascii_lowercase();
            if literal.is_none()
                && matches!(name.as_str(), "src" | "example" | "export" | "comment")
            {
                literal = Some(name);
            } else if literal.is_none() && !name.is_empty() {
                containers.push(name);
                if containers.len() > PARSE_INPUT_MAX_DEPTH {
                    return false;
                }
            }
        } else if body
            .get(..6)
            .is_some_and(|prefix| prefix.eq_ignore_ascii_case("#+END_"))
        {
            let name = &body[6..];
            let name = name
                .split_whitespace()
                .next()
                .unwrap_or("")
                .to_ascii_lowercase();
            if literal.as_deref() == Some(name.as_str()) {
                literal = None;
            } else if literal.is_none() && containers.last().is_some_and(|open| *open == name) {
                containers.pop();
            }
        }
        // Spaced (`> > >`) and list-prefixed (`- > >`) staircases nest too.
        if tine_core::render::line_quote_depth(body) > PARSE_INPUT_MAX_DEPTH {
            return false;
        }
        // Inline parsing starts afresh for each source line. Only paired
        // delimiters can form a recursive inline value; unmatched punctuation
        // is ordinary text, even if it occurs thousands of times in a page.
        let mut opens = Vec::new();
        let mut paired = vec![false; line.len()];
        for (index, byte) in line.bytes().enumerate() {
            match byte {
                b'[' | b'{' | b'(' => opens.push((byte, index)),
                b']' | b'}' | b')' => {
                    let expected = match byte {
                        b']' => b'[',
                        b'}' => b'{',
                        _ => b'(',
                    };
                    if opens.last().is_some_and(|(open, _)| *open == expected) {
                        let (_, start) = opens.pop().unwrap();
                        paired[start] = true;
                        paired[index] = true;
                    } else {
                        opens.clear();
                    }
                }
                _ => {}
            }
        }
        let mut depth = 0usize;
        for (index, byte) in line.bytes().enumerate() {
            if !paired[index] {
                continue;
            }
            if matches!(byte, b'[' | b'{' | b'(') {
                depth += 1;
                if depth > PARSE_INPUT_MAX_DEPTH {
                    return false;
                }
            } else {
                depth -= 1;
            }
        }
    }
    true
}

/// `trimmed.starts_with("```") || trimmed.starts_with("~~~")` after lsdoc's
/// OCaml-style trim (space, tab, CR, LF, FF).
fn is_fence_marker_line(line: &str) -> bool {
    let trimmed = line.trim_start_matches([' ', '\t', '\r', '\n', '\x0c']);
    trimmed.starts_with("```") || trimmed.starts_with("~~~")
}
