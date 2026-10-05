//! Facet chrome of the static HTML export: what surrounds a block's body text.
//!
//! Header facets (task checkbox + marker, priority) precede the body; trailer
//! facets (SCHEDULED / DEADLINE, the LOGBOOK time badge, visible properties)
//! follow it. Own-numbered blocks (`logseq.order-list-type:: number`) also get
//! an ordinal marker, computed here from the same run + depth-cycle rule as the
//! app's `orderedListMarker` (`src/document/edits/properties.ts`).

use super::{checkbox_state, esc};
use tine_core::doc::DocBlock;

/// The header-line facet chrome that precedes a block's body text: the task
/// checkbox + marker badge and the `[#A]` priority badge (matches the app's Block header).
pub(super) fn emit_header_facets(marker: Option<&str>, priority: Option<&str>, out: &mut String) {
    if let Some(m) = marker {
        match checkbox_state(m) {
            Some(true) => out.push_str("<span class=\"task-checkbox checked\"></span>"),
            Some(false) => out.push_str("<span class=\"task-checkbox\"></span>"),
            None => {}
        }
        out.push_str(&format!(
            "<span class=\"task-marker m-{}\">{}</span> ",
            m.to_ascii_lowercase(),
            esc(m)
        ));
    }
    if let Some(p) = priority {
        out.push_str(&format!(
            "<span class=\"priority p-{}\">[#{}]</span> ",
            p.to_ascii_lowercase(),
            esc(p)
        ));
    }
}

/// The trailing facet chrome shown BELOW a block's body: SCHEDULED / DEADLINE
/// planning lines, the time-tracking summary, and the block's visible
/// `key:: value` properties. The LOGBOOK drawer itself stays hidden, but the
/// app shows its clocked total as a badge, so the export carries the badge.
pub(super) fn emit_trailer_facets(
    block: &DocBlock,
    raw: &str,
    props: &[(String, String)],
    hidden: &[String],
    out: &mut String,
) {
    if let Some(s) = block.scheduled() {
        out.push_str(&format!(
            "<div class=\"planning scheduled\"><span class=\"pk\">SCHEDULED:</span> {}</div>",
            esc(s)
        ));
    }
    if let Some(d) = block.deadline() {
        out.push_str(&format!(
            "<div class=\"planning deadline\"><span class=\"pk\">DEADLINE:</span> {}</div>",
            esc(d)
        ));
    }
    let clocked = tine_core::logbook::clock_summary_seconds(raw, block.is_org());
    if clocked > 0 {
        out.push_str(&format!(
            "<div class=\"planning logbook\"><span class=\"pk\">CLOCK:</span> {:02}:{:02}:{:02}</div>",
            clocked / 3600,
            (clocked / 60) % 60,
            clocked % 60
        ));
    }
    // The app's own predicate (`tine_core::render_facets`): the graph's
    // `:block-hidden-properties` hide chips here exactly as they do live.
    let visible: Vec<&(String, String)> = props
        .iter()
        .filter(|(k, _)| !tine_core::render_facets::is_render_hidden_prop(k, hidden))
        .collect();
    if !visible.is_empty() {
        out.push_str("<div class=\"block-props\">");
        for (k, v) in visible {
            out.push_str(&format!(
                "<div class=\"prop\"><span class=\"pk\">{}::</span> <span class=\"pv\">{}</span></div>",
                esc(k),
                esc(v)
            ));
        }
        out.push_str("</div>");
    }
}

/// A block's ordinal position: how many consecutive own-numbered ancestors it
/// has (`parents`, which picks the glyph) and, when the block is itself
/// own-numbered, its 1-based index in the run of own-numbered siblings.
#[derive(Clone, Copy, Default)]
pub(super) struct Ordinal {
    parents: u8,
    index: Option<u32>,
}

/// A block's OWN `logseq.order-list-type:: number` makes its bullet an ordered
/// marker (the app's `isOrdered`; there is no inheritance).
fn own_ordered(b: &DocBlock) -> bool {
    b.property("logseq.order-list-type").as_deref() == Some("number")
}

impl Ordinal {
    /// `"1."`, `"a."`, `"i."` … for an own-numbered block, else `None`. The
    /// glyph cycles number → letter → roman with the ancestor depth (mod 3).
    pub(super) fn marker(self) -> Option<String> {
        let i = self.index?;
        Some(format!(
            "{}.",
            tine_core::ordinal::glyph(i, self.parents as u32)
        ))
    }

    /// The ordinals of `b`'s children, in order.
    pub(super) fn children(self, b: &DocBlock) -> Vec<Ordinal> {
        let parents = if own_ordered(b) { self.parents + 1 } else { 0 };
        siblings(&b.children, parents)
    }
}

/// The ordinals of a run of siblings that all sit under `parents` consecutive
/// own-numbered ancestors (0 for page roots).
pub(super) fn siblings(blocks: &[DocBlock], parents: u8) -> Vec<Ordinal> {
    let mut run = 0u32;
    blocks
        .iter()
        .map(|b| {
            run = if own_ordered(b) { run + 1 } else { 0 };
            Ordinal {
                parents,
                index: (run > 0).then_some(run),
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn glyphs_follow_the_apps_toletters_and_toroman() {
        assert_eq!(tine_core::ordinal::letters(1), "a");
        assert_eq!(tine_core::ordinal::letters(26), "z");
        assert_eq!(tine_core::ordinal::letters(27), "aa");
        assert_eq!(tine_core::ordinal::roman(4), "iv");
        assert_eq!(tine_core::ordinal::roman(1994), "mcmxciv");
    }
}
