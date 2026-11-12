use super::{Placement, Role, TextTrace, TraceError};
use std::collections::{HashMap, HashSet};

/// One parser-owned source drawer: complete physical marker originals and
/// every intervening original, in input order. No I/O or persistent state.
#[derive(Clone, Debug)]
pub struct SourceDrawer {
    pub org: bool,
    pub opening: usize,
    pub entries: Vec<usize>,
    pub closing: usize,
}

/// Validate an already native-owned complete physical marker line. This is
/// the mldoc indentation alphabet (parsers.ml:8,17; drawer.ml:55,60), not a
/// syntax recognizer or a source of drawer ownership.
fn marker(text: &str, opening: bool) -> bool {
    let ending = crate::doc::line_terminator(text);
    if opening && ending.is_empty() {
        return false;
    }
    let content = &text[..text.len() - ending.len()];
    crate::block_regions::is_property_drawer_marker(content, opening)
}

impl TextTrace {
    /// Source-only boundary exception, separate from AL. O(pieces + output).
    pub(super) fn approved_drawer_boundaries(
        &self,
        m: &HashMap<usize, String>,
    ) -> Result<HashSet<usize>, TraceError> {
        if self.source_drawers.is_empty() {
            return Ok(HashSet::new());
        }
        let mut head = vec![false; self.pieces.len()];
        let mut at = 0;
        for placement in &self.output {
            match placement {
                Placement::Insert(s) => at += s.len(),
                Placement::Original(id) => {
                    at += self.rendered(*id, m)?.len();
                    if at <= self.header_end {
                        head[*id] = true;
                    }
                }
            }
        }
        let mut consumed = vec![false; self.pieces.len()];
        for id in &self.consumed {
            *consumed.get_mut(*id).ok_or(TraceError::Consumption)? = true;
        }
        let mut allowed = HashSet::new();
        for drawer in &self.source_drawers {
            let open = self
                .pieces
                .get(drawer.opening)
                .ok_or(TraceError::Partition)?;
            let close = self
                .pieces
                .get(drawer.closing)
                .ok_or(TraceError::Partition)?;
            if !drawer.org
                || self.inputs.len() != 2
                || open.file != 1
                || close.file != 1
                || open.range.end > close.range.start
                || open.editable
                || close.editable
                || open.role != Role::DrawerBoundary
                || close.role != Role::DrawerBoundary
                || !marker(self.original(drawer.opening), true)
                || !marker(self.original(drawer.closing), false)
            {
                return Err(TraceError::Consumption);
            }
            if drawer
                .entries
                .iter()
                .copied()
                .ne(drawer.opening + 1..drawer.closing)
                || drawer.entries.iter().any(|id| self.pieces[*id].file != 1)
            {
                return Err(TraceError::Partition);
            }
            if drawer.entries.iter().all(|id| head[*id] || consumed[*id]) {
                allowed.extend([drawer.opening, drawer.closing]);
            }
        }
        Ok(allowed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn empty_source(indent: &str, eol: &str) -> TextTrace {
        let mut trace = TextTrace::default();
        trace.add_input("", &[], &[]).unwrap();
        let open = format!("{indent}:pRoPeRtIeS:{eol}");
        let text = format!("{open}{indent}:EnD:{eol}");
        trace.add_input(&text, &[], &[open.len()]).unwrap();
        for piece in &mut trace.pieces {
            piece.role = Role::DrawerBoundary;
        }
        trace.source_drawers.push(SourceDrawer {
            org: true,
            opening: 0,
            entries: vec![],
            closing: 1,
        });
        trace.scope.insert(1);
        trace.consumed = vec![0, 1];
        trace
    }

    #[test]
    fn native_source_markers_have_exact_indentation_and_physical_endings() {
        for indent in ["", " \t", "\u{c}", "\u{1a}"] {
            for ending in ["\n", "\r\n", "\r"] {
                let trace = empty_source(indent, ending);
                assert_eq!(trace.check("", &trace.substitution), Ok(()));
            }
        }
        for indent in ["\u{b}", "\u{a0}"] {
            let trace = empty_source(indent, "\n");
            assert_eq!(
                trace.check("", &trace.substitution),
                Err(TraceError::Consumption)
            );
        }
        let mut non_org = empty_source("", "\n");
        non_org.source_drawers[0].org = false;
        assert_eq!(
            non_org.check("", &non_org.substitution),
            Err(TraceError::Consumption)
        );
        let mut trace = empty_source("", "\n");
        for piece in &mut trace.pieces {
            piece.file = 0;
        }
        assert!(
            trace.check("", &trace.substitution).is_err(),
            "target markers never qualify"
        );
    }

    #[test]
    fn a_source_entry_in_the_body_does_not_license_marker_consumption() {
        let mut trace = TextTrace::default();
        trace.add_input("", &[], &[]).unwrap();
        let text = ":PROPERTIES:\n:K: value\n:END:\n";
        trace.add_input(text, &[], &[13, 23]).unwrap();
        trace.pieces[0].role = Role::DrawerBoundary;
        trace.pieces[2].role = Role::DrawerBoundary;
        trace.source_drawers.push(SourceDrawer {
            org: true,
            opening: 0,
            entries: vec![1],
            closing: 2,
        });
        trace.scope.insert(1);
        trace.consumed = vec![0, 2];
        trace.output = vec![Placement::Original(1)];
        assert_eq!(
            trace.check(":K: value\n", &trace.substitution),
            Err(TraceError::Consumption)
        );
        trace.header_end = 10;
        assert_eq!(trace.check(":K: value\n", &trace.substitution), Ok(()));
    }
}
