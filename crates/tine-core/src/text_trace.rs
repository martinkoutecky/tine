//! Ephemeral complete-text evidence for the merge writer (model O6 and P4).
//!
//! A writer partitions its original files at parser-owned edit/placement
//! boundaries, then places originals or explicit inserted syntax. `check`
//! verifies coverage, multiplicity, scoped consumption and the complete
//! rendered result in O(input + output bytes + pieces). No I/O or durable state.
use std::{
    collections::{HashMap, HashSet},
    ops::Range,
};

/// The fixed input classifier. P4 lets a merge drop only an alias member that
/// reads as a kept one, the syntax of combined alias declarations, and bytes
/// the parser does not report as content (layout); each has a narrow
/// consumption rule, and ordinary text is always retained.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Role {
    Opaque,
    AliasKey,
    AliasDelimiter,
    AliasName,
    /// A blank line, a structural prefix or a property-drawer frame line.
    Layout,
}

/// One disjoint interval of an original physical file.
#[derive(Clone, Debug)]
pub struct Piece {
    pub file: usize,
    pub range: Range<usize>,
    pub editable: bool,
    pub role: Role,
}

/// A retained input piece or explicitly inserted serialization syntax.
#[derive(Clone, Debug)]
pub enum Placement {
    Original(usize),
    Insert(String),
}

/// A malformed writer proposal; rejected before any durable operation.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TraceError {
    Partition,
    Substitution,
    Output,
    Multiplicity,
    Consumption,
    TitleIdentity,
}

impl std::fmt::Display for TraceError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if *self == Self::TitleIdentity {
            f.write_str("Q-TITLE")
        } else {
            write!(f, "complete-text certificate: {self:?}")
        }
    }
}
impl std::error::Error for TraceError {}

/// Input ranges and candidate placements, kept only while preparing a write.
#[derive(Clone, Debug, Default)]
pub struct TextTrace {
    pub inputs: Vec<String>,
    pub pieces: Vec<Piece>,
    pub output: Vec<Placement>,
    pub consumed: Vec<usize>,
    pub scope: HashSet<usize>,
    /// The legal span finder's rendering under this candidate's substitution.
    pub substitution: HashMap<usize, String>,
    /// Paired native link inserts around a changed semantic name. Wrappers
    /// already present in the input remain protected original pieces.
    pub wrappers: HashSet<usize>,
    /// The page each alias-name piece reads as (the writer's name identity).
    pub readings: HashMap<usize, String>,
    starts: HashMap<(usize, usize), usize>,
}

impl TextTrace {
    /// Partition one file at the given structural boundaries and legal edits.
    /// `cuts` are source ordered; edits must be disjoint, source ordered and
    /// not cross a structural boundary. Every other byte is protected.
    pub fn add_input(
        &mut self,
        text: &str,
        edits: &[(Range<usize>, String)],
        cuts: &[usize],
    ) -> Result<usize, TraceError> {
        let file = self.inputs.len();
        if edits.iter().any(|(r, _)| r.is_empty())
            || edits.windows(2).any(|w| w[0].0.end > w[1].0.start)
        {
            return Err(TraceError::Partition);
        }
        // A transient byte per input byte avoids sorting the partition's cuts:
        // construction is linear even for a header with one token per byte.
        let mut boundary = vec![false; text.len() + 1];
        for at in cuts
            .iter()
            .copied()
            .chain([0, text.len()])
            .chain(edits.iter().flat_map(|(r, _)| [r.start, r.end]))
        {
            *boundary.get_mut(at).ok_or(TraceError::Partition)? = true;
        }
        let mut edit = 0;
        let mut start = 0;
        for end in (1..=text.len()).filter(|at| boundary[*at]) {
            let range = start..end;
            start = end;
            if text.get(range.clone()).is_none() {
                return Err(TraceError::Partition);
            }
            while edit < edits.len() && edits[edit].0.end <= range.start {
                edit += 1;
            }
            let replacement = edits.get(edit).filter(|(r, _)| *r == range);
            if edits
                .get(edit)
                .is_some_and(|(r, _)| r.start < range.end && r.end > range.start)
                && replacement.is_none()
            {
                return Err(TraceError::Partition);
            }
            let id = self.pieces.len();
            self.starts.insert((file, range.start), id);
            self.pieces.push(Piece {
                file,
                range,
                editable: replacement.is_some(),
                role: Role::Opaque,
            });
            if let Some((_, to)) = replacement {
                self.substitution.insert(id, to.clone());
            }
        }
        self.inputs.push(text.to_owned());
        Ok(file)
    }

    /// Source-ordered piece ids for an interval whose endpoints were cuts.
    pub fn interval(&self, file: usize, range: Range<usize>) -> Result<Vec<usize>, TraceError> {
        if range.is_empty() {
            return Ok(Vec::new());
        }
        let start = *self
            .starts
            .get(&(file, range.start))
            .ok_or(TraceError::Partition)?;
        let mut end = start;
        while self
            .pieces
            .get(end)
            .is_some_and(|p| p.file == file && p.range.end <= range.end)
        {
            end += 1;
        }
        let ids: Vec<_> = (start..end).collect();
        if ids
            .first()
            .is_none_or(|id| self.pieces[*id].range.start != range.start)
            || ids
                .last()
                .is_none_or(|id| self.pieces[*id].range.end != range.end)
        {
            return Err(TraceError::Partition);
        }
        Ok(ids)
    }

    /// Original bytes of a piece, without applying a candidate substitution.
    pub fn original(&self, id: usize) -> &str {
        let p = &self.pieces[id];
        &self.inputs[p.file][p.range.clone()]
    }

    fn rendered<'a>(
        &'a self,
        id: usize,
        m: &'a HashMap<usize, String>,
    ) -> Result<&'a str, TraceError> {
        let p = self.pieces.get(id).ok_or(TraceError::Partition)?;
        if p.editable {
            m.get(&id)
                .map(String::as_str)
                .ok_or(TraceError::Substitution)
        } else if m.contains_key(&id) {
            Err(TraceError::Substitution)
        } else {
            Ok(self.original(id))
        }
    }

    /// Render this proposal using the legal edit spans' substitution.
    pub fn render(&self) -> Result<String, TraceError> {
        let mut out = String::new();
        for placement in &self.output {
            match placement {
                Placement::Original(id) => out.push_str(self.rendered(*id, &self.substitution)?),
                Placement::Insert(s) => out.push_str(s),
            }
        }
        Ok(out)
    }

    /// Build an ordered rewrite image, distinct from merge assembly. O(pieces).
    pub fn ordered_output(&mut self) {
        self.output.clear();
        for id in 0..self.pieces.len() {
            if self.wrappers.contains(&id) {
                self.output.push(Placement::Insert("[[".into()));
            }
            self.output.push(Placement::Original(id));
            if self.wrappers.contains(&id) {
                self.output.push(Placement::Insert("]]".into()));
            }
        }
    }

    /// Certify an ordered name rewrite. No relocation, consumption or assembly
    /// punctuation is licensed here; only paired links around changed names.
    /// O(input + output bytes + pieces), no I/O.
    pub fn check_shadow(&self, output: &str) -> Result<(), TraceError> {
        if !self.consumed.is_empty() {
            return Err(TraceError::Consumption);
        }
        let mut at = 0;
        for id in 0..self.pieces.len() {
            let wrapped = matches!(self.output.get(at), Some(Placement::Insert(s)) if s == "[[");
            at += usize::from(wrapped);
            if !matches!(self.output.get(at), Some(Placement::Original(n)) if *n == id) {
                return Err(TraceError::Partition);
            }
            at += 1;
            if wrapped {
                if !self.pieces[id].editable
                    || self
                        .substitution
                        .get(&id)
                        .is_none_or(|s| s == self.original(id))
                    || !matches!(self.output.get(at), Some(Placement::Insert(s)) if s == "]]")
                {
                    return Err(TraceError::Substitution);
                }
                at += 1;
            }
        }
        if at != self.output.len() {
            return Err(TraceError::Output);
        }
        self.check(output, &self.substitution)
    }

    /// Check `TraceValid` against the actual proposed output and substitution.
    pub fn check(&self, output: &str, m: &HashMap<usize, String>) -> Result<(), TraceError> {
        #[cfg(feature = "test-counters")]
        work::certificate_check();
        let mut offsets = vec![0; self.inputs.len()];
        for p in &self.pieces {
            let at = offsets.get_mut(p.file).ok_or(TraceError::Partition)?;
            if p.range.start != *at
                || p.range.is_empty()
                || self.inputs[p.file].get(p.range.clone()).is_none()
            {
                return Err(TraceError::Partition);
            }
            *at = p.range.end;
        }
        if offsets
            .iter()
            .zip(&self.inputs)
            .any(|(at, s)| *at != s.len())
        {
            return Err(TraceError::Partition);
        }
        let mut counts = vec![0u8; self.pieces.len()];
        let mut at = 0;
        let mut names = HashSet::new();
        for placement in &self.output {
            let text = match placement {
                Placement::Insert(s) => s.as_str(),
                Placement::Original(id) => {
                    let count = counts.get_mut(*id).ok_or(TraceError::Partition)?;
                    *count = count.saturating_add(1);
                    if self.pieces[*id].role == Role::AliasName {
                        names.extend(self.readings.get(id).map(String::as_str));
                    }
                    self.rendered(*id, m)?
                }
            };
            if output.get(at..at + text.len()) != Some(text) {
                return Err(TraceError::Output);
            }
            at += text.len();
        }
        if at != output.len() {
            return Err(TraceError::Output);
        }
        for id in &self.consumed {
            let p = self.pieces.get(*id).ok_or(TraceError::Consumption)?;
            counts[*id] = counts[*id].saturating_add(1);
            if !self.scope.contains(&p.file) {
                return Err(TraceError::Consumption);
            }
            // Syntax goes only where a declaration with members remains; a
            // name only where a kept name reads as the same page.
            let text = self.original(*id);
            let allowed = match p.role {
                Role::AliasKey => !names.is_empty() && alias_key(text),
                Role::AliasDelimiter => !names.is_empty() && alias_syntax(text),
                Role::AliasName => self
                    .readings
                    .get(id)
                    .is_some_and(|r| names.contains(r.as_str())),
                Role::Opaque => false,
                Role::Layout => layout(text),
            };
            if !allowed {
                return Err(TraceError::Consumption);
            }
        }
        if counts.iter().any(|n| *n != 1) {
            return Err(TraceError::Multiplicity);
        }
        Ok(())
    }
}

/// The key of a parser-owned alias declaration (line start through the value
/// start), in any case and spacing, on one line. O(bytes); this does not
/// recognize properties, the parser did.
pub fn alias_key(s: &str) -> bool {
    !s.contains(['\n', '\r'])
        && matches!(
            s.trim().to_ascii_lowercase().as_str(),
            "alias::" | "aliases::" | "#+alias:" | "#+aliases:" | ":alias:" | ":aliases:"
        )
}

/// Bytes of an alias value outside its member names: separators, link and
/// tag wrappers, spacing and the line terminator. O(bytes).
pub fn alias_syntax(s: &str) -> bool {
    s.chars().all(|c| {
        c.is_whitespace()
            || crate::refs::is_linkable_property_separator(c)
            || matches!(c, '#' | '[' | ']')
    })
}

/// The fragments of an alias declaration that this certificate cannot accept
/// as declaration syntax, so a merge that combines the declaration must keep
/// them: before the `value`, unless it is an alias key; between member
/// `tokens` and after the last, unless alias syntax. Native ranges of `text`;
/// `line` is the declaration's line without its structural prefix. O(bytes).
pub fn alias_nonsyntax(
    text: &str,
    line: Range<usize>,
    value: usize,
    tokens: &[Range<usize>],
) -> Vec<Range<usize>> {
    let mut out = Vec::new();
    if !alias_key(&text[line.start..value]) {
        out.push(line.start..value);
    }
    let mut at = value;
    for token in tokens.iter().chain([&(line.end..line.end)]) {
        if !alias_syntax(&text[at..token.start]) {
            out.push(at..token.start);
        }
        at = token.end;
    }
    out
}

/// The whitespace a blank line or an indentation may hold here: the mldoc
/// indentation alphabet (parsers.ml:8,17), never other Unicode spaces.
const SPACE: [char; 4] = [' ', '\t', '\u{c}', '\u{1a}'];

/// A line holding only spacing and its terminator. O(bytes).
pub fn blank(s: &str) -> bool {
    s.trim_end_matches(['\r', '\n'])
        .trim_matches(SPACE)
        .is_empty()
}

/// Bytes the parser does not report as content (P4): a blank line, a
/// structural prefix (indentation and a bullet) or a property-drawer frame
/// line. O(bytes).
pub fn layout(s: &str) -> bool {
    let body = s.trim_end_matches(['\r', '\n']);
    let t = body.trim_matches(SPACE);
    t.is_empty()
        || t == "-"
        || t == "+"
        || t.bytes().all(|b| b == b'*')
        || crate::block_regions::is_property_drawer_marker(body, true)
        || crate::block_regions::is_property_drawer_marker(body, false)
}

/// Process-global counts of rename text work, for downstream cost tests of a
/// whole host operation (its work runs on the host's threads): reference
/// span walks of one page's text (`refs` rewrite and proposal), and
/// certificate checks ([`TextTrace::check`]). Callers serialize measurements.
#[cfg(feature = "test-counters")]
pub mod work {
    use std::sync::atomic::{AtomicU64, Ordering::Relaxed};
    static PAGE_WALKS: AtomicU64 = AtomicU64::new(0);
    static CERTIFICATE_CHECKS: AtomicU64 = AtomicU64::new(0);
    pub(crate) fn page_walk() {
        PAGE_WALKS.fetch_add(1, Relaxed);
    }
    pub(super) fn certificate_check() {
        CERTIFICATE_CHECKS.fetch_add(1, Relaxed);
    }
    /// Take and reset (reference span walks, certificate checks).
    pub fn take() -> (u64, u64) {
        (
            PAGE_WALKS.swap(0, Relaxed),
            CERTIFICATE_CHECKS.swap(0, Relaxed),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn trace() -> TextTrace {
        let mut t = TextTrace::default();
        t.add_input("before Old after", &[(7..10, "New".into())], &[])
            .unwrap();
        t.output = (0..t.pieces.len()).map(Placement::Original).collect();
        t
    }
    #[test]
    fn shadow_rejects_reversal_rotation_and_unpaired_or_nonname_insertions() {
        let mut t = TextTrace::default();
        t.add_input("owner", &[], &[1, 2, 3, 4]).unwrap();
        for ids in [vec![4, 3, 2, 1, 0], vec![1, 2, 3, 4, 0]] {
            t.output = ids.into_iter().map(Placement::Original).collect();
            let output = t.render().unwrap();
            assert!(
                t.check(&output, &t.substitution).is_ok(),
                "merge relocation is distinct"
            );
            assert!(t.check_shadow(&output).is_err());
        }
        t.ordered_output();
        t.output.insert(0, Placement::Insert("_".into()));
        assert!(t.check_shadow(&t.render().unwrap()).is_err());
        t.ordered_output();
        t.output.insert(0, Placement::Insert("[[".into()));
        t.output.insert(2, Placement::Insert("]]".into()));
        assert!(t.check_shadow(&t.render().unwrap()).is_err());
        let mut t = trace();
        t.wrappers.insert(1);
        t.ordered_output();
        assert_eq!(t.check_shadow("before [[New]] after"), Ok(()));
        t.output.pop();
        assert!(t.check_shadow(&t.render().unwrap()).is_err());
    }
    #[test]
    fn trace_valid_rejects_byte_loss_duplication_protected_edits_and_missing_substitution() {
        let t = trace();
        let s = t.render().unwrap();
        assert_eq!(t.check(&s, &t.substitution), Ok(()));
        assert!(t.check(&s[1..], &t.substitution).is_err());
        let mut duplicate = t.clone();
        duplicate.output.push(Placement::Original(0));
        assert_eq!(
            duplicate.check(&duplicate.render().unwrap(), &duplicate.substitution),
            Err(TraceError::Multiplicity)
        );
        assert!(t
            .check(&s.replace("before", "BEFORE"), &t.substitution)
            .is_err());
        assert_eq!(t.check(&s, &HashMap::new()), Err(TraceError::Substitution));
        let mut changed = t.substitution.clone();
        changed.insert(0, "BEFORE ".into());
        assert_eq!(t.check(&s, &changed), Err(TraceError::Substitution));
    }

    /// P4: a merge drops only a member that reads as a kept one and the
    /// syntax of a combined declaration, and only while a declaration with
    /// members remains. `A` reads as `a` here (the writer's reading).
    #[test]
    fn consumption_requires_scoped_alias_syntax_and_a_kept_name_of_the_same_reading() {
        let mut t = TextTrace::default();
        t.add_input("alias:: a\n", &[], &[8, 9]).unwrap();
        t.add_input("Alias::  A\n", &[], &[9, 10]).unwrap();
        for (id, p) in t.pieces.iter_mut().enumerate() {
            p.role = match id % 3 {
                0 => Role::AliasKey,
                1 => Role::AliasName,
                _ => Role::AliasDelimiter,
            };
        }
        t.readings.insert(1, "a".into());
        t.readings.insert(4, "a".into());
        t.output = (0..3).map(Placement::Original).collect();
        t.consumed = (3..6).collect();
        t.scope.insert(1);
        assert_eq!(t.check("alias:: a\n", &t.substitution), Ok(()));
        let mut bad = t.clone();
        bad.scope.clear();
        assert_eq!(
            bad.check("alias:: a\n", &bad.substitution),
            Err(TraceError::Consumption)
        );
        // No kept name: the syntax and the name have nowhere to go.
        let mut bad = t.clone();
        bad.pieces[1].role = Role::Opaque;
        assert_eq!(
            bad.check("alias:: a\n", &bad.substitution),
            Err(TraceError::Consumption)
        );
        // A name that reads as another page is never dropped.
        let mut bad = t.clone();
        bad.readings.insert(4, "b".into());
        assert_eq!(
            bad.check("alias:: a\n", &bad.substitution),
            Err(TraceError::Consumption)
        );
        let mut bad = t.clone();
        bad.readings.remove(&4);
        assert_eq!(
            bad.check("alias:: a\n", &bad.substitution),
            Err(TraceError::Consumption)
        );
        let mut bad = t.clone();
        bad.pieces[5].role = Role::Opaque;
        assert_eq!(
            bad.check("alias:: a\n", &bad.substitution),
            Err(TraceError::Consumption)
        );
        // Text that is not a declaration key or alias syntax stays.
        for (text, cut) in [("tags:: A\n", 7), ("alias:: **A**\n", 8)] {
            let mut bad = TextTrace::default();
            bad.add_input("alias:: a\n", &[], &[8, 9]).unwrap();
            bad.add_input(text, &[], &[cut]).unwrap();
            bad.pieces[1].role = Role::AliasName;
            bad.readings.insert(1, "a".into());
            bad.pieces[3].role = Role::AliasKey;
            bad.pieces[4].role = Role::AliasDelimiter;
            bad.output = (0..3).map(Placement::Original).collect();
            bad.consumed = vec![3, 4];
            bad.scope.insert(1);
            assert_eq!(
                bad.check("alias:: a\n", &bad.substitution),
                Err(TraceError::Consumption),
                "{text:?}"
            );
        }
    }

    /// P4: blank lines, structural prefixes and drawer frames are not
    /// content; a drawer marker is the mldoc indentation alphabet
    /// (parsers.ml:8,17; drawer.ml:55,60) and any physical ending. A content
    /// line marked as layout is refused.
    #[test]
    fn layout_consumption_is_limited_to_bytes_that_are_not_content() {
        let consumed = |text: &str| {
            let mut t = TextTrace::default();
            t.add_input(text, &[], &[]).unwrap();
            t.pieces[0].role = Role::Layout;
            t.consumed = vec![0];
            t.scope.insert(0);
            t.check("", &t.substitution)
        };
        for indent in ["", " \t", "\u{c}", "\u{1a}"] {
            for ending in ["\n", "\r\n", "\r", ""] {
                for line in [":pRoPeRtIeS:", ":EnD:", "", "-", "* ", "  - "] {
                    let text = format!("{indent}{line}{ending}");
                    if !text.is_empty() {
                        assert_eq!(consumed(&text), Ok(()), "{text:?}");
                    }
                }
            }
        }
        for text in [
            "\u{b}:END:\n",
            "\u{a0}\n",
            ":K: value\n",
            "- x",
            "title:: x\n",
            ":END: x\n",
        ] {
            assert_eq!(consumed(text), Err(TraceError::Consumption), "{text:?}");
        }
    }
}
