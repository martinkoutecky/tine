//! Ephemeral complete-text evidence (conceptual model §5.2, Alias14.Valid).
//!
//! A writer partitions its original files at parser-owned edit/placement
//! boundaries, then places originals or explicit inserted syntax. `check`
//! verifies coverage, multiplicity, scoped alias consumption and the complete
//! rendered result in O(input + output bytes + pieces). No I/O or durable state.
use std::{
    collections::{HashMap, HashSet},
    ops::Range,
};
mod drawers;
pub use drawers::SourceDrawer;

/// The fixed input classifier. Alias syntax, exact duplicate rendered names
/// and independently owned source drawer boundaries have narrow consumption
/// rules; ordinary text is always retained.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Role {
    Opaque,
    AliasKey,
    AliasDelimiter,
    AliasName,
    DrawerBoundary,
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
    /// Native source head drawers, independently cut before assembly.
    pub source_drawers: Vec<SourceDrawer>,
    /// Physical output header extent; only placements wholly inside it join.
    pub header_end: usize,
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
        if self.header_end > output.len() {
            return Err(TraceError::Partition);
        }
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
        let mut key = false;
        let mut names = HashSet::new();
        let boundaries = self.approved_drawer_boundaries(m)?;
        for placement in &self.output {
            let text = match placement {
                Placement::Insert(s) => s.as_str(),
                Placement::Original(id) => {
                    let count = counts.get_mut(*id).ok_or(TraceError::Partition)?;
                    *count = count.saturating_add(1);
                    let text = self.rendered(*id, m)?;
                    match self.pieces[*id].role {
                        Role::AliasKey => key |= consumable_alias_key(self.original(*id)),
                        Role::AliasName => {
                            names.insert(text);
                        }
                        _ => {}
                    }
                    text
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
            let allowed = match p.role {
                Role::AliasKey => key && consumable_alias_key(self.original(*id)),
                Role::AliasDelimiter => {
                    key && matches!(self.original(*id), ", " | "," | "\n" | "\r\n" | "\r")
                }
                Role::AliasName => names.contains(self.rendered(*id, m)?),
                Role::Opaque => false,
                Role::DrawerBoundary => boundaries.contains(id),
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

/// The frozen AL key whitelist, applied only to an independently native-owned
/// declaration prefix. O(1), bounded tokens; this does not recognize properties.
pub fn consumable_alias_key(s: &str) -> bool {
    matches!(
        s,
        "alias:: " | "aliases:: " | "#+alias: " | "#+aliases: " | ":ALIAS: " | ":ALIASES: "
    )
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

    #[test]
    fn consumption_requires_scoped_native_syntax_and_an_identical_retained_name() {
        let mut t = TextTrace::default();
        for _ in 0..2 {
            t.add_input("alias:: a\n", &[], &[8, 9]).unwrap();
        }
        for (id, p) in t.pieces.iter_mut().enumerate() {
            p.role = match id % 3 {
                0 => Role::AliasKey,
                1 => Role::AliasName,
                _ => Role::AliasDelimiter,
            };
        }
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
        let mut bad = t.clone();
        bad.pieces[0].role = Role::Opaque;
        assert_eq!(
            bad.check("alias:: a\n", &bad.substitution),
            Err(TraceError::Consumption)
        );
        let mut bad = t.clone();
        bad.inputs[1] = "alias:: b\n".into();
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
    }
}
