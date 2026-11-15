//! Lossless sidecar editing. Reuse the authored EDN span reader and value
//! writer; unchanged records keep their original tokens, including number spelling.
//! New collections follow Fipp 0.6.26 edn/pretty-coll and engine/format-nodes:
//! width 70, aligned children, comma + line between map entries.
//! O(sidecar bytes + output bytes), no I/O. The guarded feature save owns I/O.

use super::{highlight_id, Edn};
use crate::{
    edn,
    query_edn::{self, Form},
};
use std::borrow::Cow;
use std::collections::{HashMap, VecDeque};

// ClojureScript pr-str uses Number.toString. Reuse serde_json's shortest
// decimal digits (ties to even), then apply JavaScript's notation thresholds.
// Rust Display chooses a different shortest-decimal tie in GH #954.
fn number(value: f64) -> String {
    if value == 0.0 {
        return "0".into();
    }
    if !value.is_finite() {
        return if value.is_nan() {
            "##NaN"
        } else if value > 0.0 {
            "##Inf"
        } else {
            "##-Inf"
        }
        .into();
    }
    let raw = serde_json::to_string(&value).unwrap();
    let sign = if value < 0.0 { "-" } else { "" };
    let raw = raw.trim_start_matches('-');
    let (mantissa, exponent) = raw
        .split_once('e')
        .map_or((raw, 0), |(m, e)| (m, e.parse::<i32>().unwrap()));
    let mut point = mantissa.find('.').unwrap_or(mantissa.len()) as i32 + exponent;
    let mut digits = mantissa.replace('.', "");
    let leading = digits.len() - digits.trim_start_matches('0').len();
    point -= leading as i32;
    digits = digits[leading..].trim_end_matches('0').into();
    if value.abs() < 1e-6 || value.abs() >= 1e21 {
        let fraction = if digits.len() > 1 {
            format!(".{}", &digits[1..])
        } else {
            String::new()
        };
        return format!("{sign}{}{fraction}e{:+}", &digits[..1], point - 1);
    }
    if point <= 0 {
        return format!("{sign}0.{}{digits}", "0".repeat((-point) as usize));
    }
    let point = point as usize;
    if point >= digits.len() {
        format!("{sign}{digits}{}", "0".repeat(point - digits.len()))
    } else {
        format!("{sign}{}.{}", &digits[..point], &digits[point..])
    }
}

struct Doc {
    width: usize,
    kind: Layout,
}
enum Layout {
    Text(String),
    Span(Vec<Doc>),
    Collection(&'static str, Vec<Doc>, &'static str, char),
}
impl Doc {
    fn text(text: String) -> Self {
        Self {
            width: text.encode_utf16().count(),
            kind: Layout::Text(text),
        }
    }
    fn span(items: Vec<Self>) -> Self {
        Self {
            width: items.iter().map(|d| d.width).sum(),
            kind: Layout::Span(items),
        }
    }
    fn value(value: &Edn) -> Self {
        let (open, items, sep, close) = match value {
            Edn::Map(pairs) => (
                "{",
                pairs
                    .iter()
                    .map(|(k, v)| {
                        Self::span(vec![Self::value(k), Self::text(" ".into()), Self::value(v)])
                    })
                    .collect::<Vec<_>>(),
                ",",
                '}',
            ),
            Edn::Vec(items) => ("[", items.iter().map(Self::value).collect(), "", ']'),
            Edn::List(items) => ("(", items.iter().map(Self::value).collect(), "", ')'),
            Edn::Set(items) => ("#{", items.iter().map(Self::value).collect(), "", '}'),
            Edn::Tagged(tag, value) => {
                return Self::span(vec![Self::text(format!("#{tag} ")), Self::value(value)])
            }
            Edn::Float(value) => return Self::text(number(*value)),
            _ => return Self::text(edn::to_string(value)),
        };
        Self {
            width: open.len()
                + 1
                + items.iter().map(|d| d.width).sum::<usize>()
                + items.len().saturating_sub(1) * (sep.len() + 1),
            kind: Layout::Collection(open, items, sep, close),
        }
    }
    fn render(&self, out: &mut String, column: &mut usize, flat: bool, newline: &str) {
        match &self.kind {
            Layout::Text(text) => {
                out.push_str(text);
                *column += self.width;
            }
            Layout::Span(items) => {
                for item in items {
                    item.render(out, column, flat, newline);
                }
            }
            Layout::Collection(open, items, sep, close) => {
                let flat = flat || *column + self.width <= 70;
                out.push_str(open);
                *column += open.len();
                let indent = *column;
                for (i, item) in items.iter().enumerate() {
                    if i > 0 {
                        out.push_str(sep);
                        if flat {
                            out.push(' ');
                            *column += sep.len() + 1;
                        } else {
                            out.push_str(newline);
                            out.extend(std::iter::repeat_n(' ', indent));
                            *column = indent;
                        }
                    }
                    item.render(out, column, flat, newline);
                }
                out.push(*close);
                *column += 1;
            }
        }
    }
}

fn pretty(value: &Edn, mut column: usize, newline: &str) -> String {
    let mut out = String::new();
    Doc::value(value).render(&mut out, &mut column, false, newline);
    out
}

fn advance(column: usize, text: &str) -> usize {
    match text.rsplit_once('\n') {
        Some((_, line)) => line.encode_utf16().count(),
        None => column + text.encode_utf16().count(),
    }
}

fn patch<'a>(
    source: &'a str,
    form: &Form,
    old: &Edn,
    new: &Edn,
    column: usize,
    newline: &str,
) -> Cow<'a, str> {
    if old == new {
        return Cow::Borrowed(&source[form.span.clone()]);
    }
    match (old, new) {
        (Edn::Map(old), Edn::Map(new))
            if old.len() <= new.len() && old.iter().zip(new).all(|(a, b)| a.0 == b.0) =>
        {
            let mut out = String::new();
            let mut at = form.span.start;
            let mut current_column = column;
            for ((old, new), pair) in old.iter().zip(new).zip(form.children.chunks_exact(2)) {
                let value = &pair[1];
                out.push_str(&source[at..value.span.start]);
                current_column = advance(current_column, &source[at..value.span.start]);
                out.push_str(&patch(
                    source,
                    value,
                    &old.1,
                    &new.1,
                    current_column,
                    newline,
                ));
                current_column = advance(current_column, &source[value.span.clone()]);
                at = value.span.end;
            }
            out.push_str(&source[at..form.span.end - 1]);
            for (i, (key, value)) in new.iter().enumerate().skip(old.len()) {
                if i > 0 {
                    out.push(',');
                    out.push_str(newline);
                    out.push_str(&" ".repeat(column + 1));
                }
                out.push_str(&edn::to_string(key));
                out.push(' ');
                out.push_str(&pretty(
                    value,
                    column + 2 + edn::to_string(key).len(),
                    newline,
                ));
            }
            out.push('}');
            out.into()
        }
        (Edn::Vec(old), Edn::Vec(new)) => {
            let by_id: HashMap<_, _> = old
                .iter()
                .enumerate()
                .filter_map(|(i, h)| Some((highlight_id(h.get("id")?)?, i)))
                .collect();
            let mut unmodelled: HashMap<String, VecDeque<usize>> = HashMap::new();
            for (i, entry) in old
                .iter()
                .enumerate()
                .filter(|(_, h)| h.get("id").and_then(highlight_id).is_none())
            {
                unmodelled
                    .entry(edn::to_string(entry))
                    .or_default()
                    .push_back(i);
            }
            let indent = column + 1;
            let separator = format!("{newline}{}", " ".repeat(indent));
            let mut out = String::from("[");
            let first = form
                .children
                .first()
                .map_or(form.span.end - 1, |child| child.span.start);
            out.push_str(&source[form.span.start + 1..first]);
            for (i, item) in new.iter().enumerate() {
                let index = item
                    .get("id")
                    .and_then(highlight_id)
                    .and_then(|id| by_id.get(id).copied())
                    .or_else(|| {
                        unmodelled
                            .get_mut(&edn::to_string(item))
                            .and_then(VecDeque::pop_front)
                    });
                if i > 0 {
                    if let Some(index) = index.filter(|index| *index > 0) {
                        out.push_str(
                            &source[form.children[index - 1].span.end
                                ..form.children[index].span.start],
                        );
                    } else {
                        out.push_str(&separator);
                    }
                }
                if let Some(index) = index {
                    out.push_str(&patch(
                        source,
                        &form.children[index],
                        &old[index],
                        item,
                        indent,
                        newline,
                    ));
                } else {
                    out.push_str(&pretty(item, indent, newline));
                }
            }
            if let Some(last) = form.children.last() {
                out.push_str(&source[last.span.end..form.span.end - 1]);
            }
            out.push(']');
            out.into()
        }
        _ => pretty(new, column, newline).into(),
    }
}

pub(super) fn write(new: &Edn, old: Option<&Edn>, source: &str) -> String {
    let newline = if source.contains("\r\n") {
        "\r\n"
    } else {
        "\n"
    };
    if let (Some(old), Some(form)) = (
        old,
        query_edn::read_with_limit(source, edn::MAX_INPUT_BYTES),
    ) {
        return format!(
            "{}{}{}",
            &source[..form.span.start],
            patch(
                source,
                &form,
                old,
                new,
                advance(0, &source[..form.span.start]),
                newline
            ),
            &source[form.span.end..]
        );
    }
    format!("{}{newline}", pretty(new, 0, newline))
}

#[cfg(test)]
mod tests {
    use super::super::*;

    // Synthesized from Fipp 0.6.26 pretty-coll/visit-map (width 70), the
    // serializer used by Logseq persist-hls-data$ at 6e7afa8eb.
    const RECORD: &str = "{:id \"old\",\n               :page 1,\n               :position {:bounding {:x1 10,\n                                     :y1 20,\n                                     :x2 763.5791625976562,\n                                     :y2 40,\n                                     :width 800,\n                                     :height 1000},\n                          :rects (),\n                          :page 1},\n               :content {:text \"First line\\nSecond line\"},\n               :properties {:color \"yellow\"}}";

    fn fixture() -> String {
        format!("{{:highlights [{RECORD}],\n :extra {{:page 1, :scale 1}}}}\n")
    }

    #[test]
    fn new_sidecar_uses_logseq_fipp_layout() {
        let source = fixture();
        let highlights = parse_highlights(&source);
        let expected = source.replace(":extra {:page 1, :scale 1}", ":extra {}");
        assert_eq!(write_highlights(&highlights, ""), expected);
    }

    #[test]
    fn new_numbers_use_clojurescript_printing_thresholds() {
        for (value, expected) in [
            (763.5791625976562, "763.5791625976562"),
            (1e20, "100000000000000000000"),
            (-1e-7, "-1e-7"),
            (1.0000000000000002, "1.0000000000000002"),
        ] {
            assert_eq!(super::number(value), expected);
        }
        assert_eq!(
            super::pretty(
                &Edn::Vec(vec![
                    Edn::Float(-0.0),
                    Edn::Float(1e-6),
                    Edn::Float(1e-7),
                    Edn::Float(1e21)
                ]),
                0,
                "\n"
            ),
            "[0 0.000001 1e-7 1e+21]"
        );
    }

    #[test]
    fn logseq_sidecar_append_changes_only_the_new_record() {
        for newline in ["\n", "\r\n"] {
            let source = fixture().replace('\n', newline);
            let mut highlights = parse_highlights(&source);
            assert_eq!(write_highlights(&highlights, &source), source);
            let mut added = highlights[0].clone();
            added.id = "new".into();
            highlights.push(added);
            let out = write_highlights(&highlights, &source);
            let record = RECORD.replace('\n', newline);
            assert!(out.contains(&record), "untouched record changed: {out}");
            let before = crate::query_edn::read(&source).unwrap();
            let after = crate::query_edn::read(&out).unwrap();
            let old_vec = &before.children[1];
            let new_vec = &after.children[1];
            assert_eq!(
                &out[..new_vec.children[0].span.end],
                &source[..old_vec.children[0].span.end]
            );
            assert_eq!(
                &out[new_vec.span.end - 1..],
                &source[old_vec.span.end - 1..]
            );
            assert_eq!(parse_highlights(&out), highlights);
        }
    }

    #[test]
    fn recolor_preserves_all_other_authored_values_and_layout() {
        let source = fixture();
        let mut highlights = parse_highlights(&source);
        highlights[0].color = "blue".into();
        let out = write_highlights(&highlights, &source);
        assert_eq!(out, source.replace("\"yellow\"", "\"blue\""));
    }

    #[test]
    fn authored_numeric_tokens_and_literal_string_newlines_survive() {
        let source = fixture()
            .replace("763.5791625976562", "7.635791625976562e+2")
            .replace("First line\\nSecond line", "First line\nSecond line");
        let mut highlights = parse_highlights(&source);
        highlights[0].color = "blue".into();
        let out = write_highlights(&highlights, &source);
        assert_eq!(
            parse_highlights(&out)[0].text.as_deref(),
            Some("First line\nSecond line")
        );
        assert_eq!(out, source.replace("\"yellow\"", "\"blue\""));
    }

    #[test]
    fn delete_keeps_the_surviving_record_and_root_metadata() {
        let source = fixture().replace(
            RECORD,
            &format!(
                "{RECORD}\n              {}",
                RECORD.replace("\"old\"", "\"other\"")
            ),
        );
        let mut highlights = parse_highlights(&source);
        highlights.pop();
        assert_eq!(write_highlights(&highlights, &source), fixture());
    }

    #[test]
    fn sidecars_larger_than_query_payloads_keep_foreign_bytes() {
        let source = fixture().replace(
            ":extra {",
            &format!(":foreign \"{}\",\n :extra {{", "z".repeat(1024 * 1024)),
        );
        let mut highlights = parse_highlights(&source);
        highlights[0].color = "blue".into();
        assert!(
            write_highlights(&highlights, &source) == source.replace("\"yellow\"", "\"blue\""),
            "large sidecar changed outside the color edit"
        );
    }

    #[test]
    fn sidecar_edit_cost_and_fidelity_at_one_and_sixty_highlights() {
        let mut changed_outside_edit = Vec::new();
        for count in [1, 60] {
            let records = (0..count)
                .map(|i| RECORD.replace("\"old\"", &format!("\"record-{i}\"")))
                .collect::<Vec<_>>()
                .join("\n              ");
            let source = format!("{{:highlights [{records}],\n :extra {{:page 1, :scale 1}}}}\n");
            let mut highlights = parse_highlights(&source);
            highlights[0].color = "blue".into();
            let start = std::time::Instant::now();
            let out = write_highlights(&highlights, &source);
            println!("unit cost: {count} highlights; input={} bytes, output={} bytes, serialization={:?}", source.len(), out.len(), start.elapsed());
            assert_eq!(parse_highlights(&out).len(), count);
            if out != source.replacen("\"yellow\"", "\"blue\"", 1) {
                changed_outside_edit.push(count);
            }
        }
        assert!(
            changed_outside_edit.is_empty(),
            "unrelated bytes changed at {changed_outside_edit:?} highlights"
        );
    }
}
