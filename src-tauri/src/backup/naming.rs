//! Snapshot names (REVIEW-backup-zip R4): one validated grammar for both
//! namespaces, a total order, and name allocation for new zips.
//!
//! A name is `<YYYY-MM-DD_HH-MM-SS>[-<word>]*[-<k>]`: a UTC stamp, an
//! optional suffix of lowercase words (`pre-restore`, `pre-journal-rename`),
//! and an optional counter `k >= 2` (written without leading zeros) for the
//! k-th snapshot of that second. A zip is `<name>.zip`, a zip being written is
//! `.partial-<name>.zip`; a legacy snapshot directory is `<name>` and its
//! partial `.partial-<name>`. Anything else is unknown: never counted, never
//! deleted, never mapped to "old".

use std::cmp::Ordering;

pub(super) const ZIP_EXT: &str = ".zip";
pub(super) const PARTIAL_PREFIX: &str = ".partial-";
/// The namespace lock file inside `<id>.zips/` (persistent, never unlinked).
pub(super) const LOCK_FILE: &str = "lock";

/// Which namespace a name was found in. Legacy sorts first among equal
/// (stamp, counter) keys, so a same-second zip counts as the newer one.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(super) enum Namespace {
    Legacy,
    Zip,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct SnapshotName {
    pub(super) stamp: String,
    pub(super) suffix: String,
    pub(super) counter: u32,
}

impl SnapshotName {
    /// Parse a bare name (no extension, no partial prefix).
    pub(super) fn parse(name: &str) -> Option<Self> {
        let stamp = name.get(..19)?;
        if !is_stamp(stamp) {
            return None;
        }
        let rest = &name[19..];
        if rest.is_empty() {
            return Some(Self {
                stamp: stamp.into(),
                suffix: String::new(),
                counter: 1,
            });
        }
        let mut parts: Vec<&str> = rest.strip_prefix('-')?.split('-').collect();
        let mut counter = 1;
        if let Some(last) = parts.last() {
            if last.bytes().all(|b| b.is_ascii_digit()) {
                if last.starts_with('0') {
                    return None;
                }
                counter = last.parse::<u32>().ok().filter(|k| *k >= 2)?;
                parts.pop();
            }
        }
        if parts
            .iter()
            .any(|word| word.is_empty() || !word.bytes().all(|b| b.is_ascii_lowercase()))
        {
            return None;
        }
        Some(Self {
            stamp: stamp.into(),
            suffix: parts.join("-"),
            counter,
        })
    }

    /// The snapshot's name without extension or partial prefix. Not named
    /// `render`: the A-K3 host-slot guard resolves callees by name, and the
    /// CLI's `owner::terminal::render` is on a host-lock path.
    pub(super) fn snapshot_name(&self) -> String {
        let mut name = self.stamp.clone();
        if !self.suffix.is_empty() {
            name.push('-');
            name.push_str(&self.suffix);
        }
        if self.counter > 1 {
            name.push_str(&format!("-{}", self.counter));
        }
        name
    }

    /// The keep-count's only permanent exemption (today's rule).
    pub(super) fn is_pre_restore(&self) -> bool {
        self.suffix == "pre-restore"
    }

    /// Chronological order between snapshots: (stamp, counter).
    pub(super) fn chrono_cmp(&self, other: &Self) -> Ordering {
        (&self.stamp, self.counter).cmp(&(&other.stamp, other.counter))
    }
}

/// One directory entry in either namespace.
#[derive(Debug, PartialEq, Eq)]
pub(super) enum Entry {
    Snapshot(SnapshotName),
    Partial(SnapshotName),
    Lock,
    Unknown,
}

/// Classify a file name inside `<id>.zips/`.
pub(super) fn classify_zip(file_name: &str) -> Entry {
    if file_name == LOCK_FILE {
        return Entry::Lock;
    }
    let Some(stem) = file_name.strip_suffix(ZIP_EXT) else {
        return Entry::Unknown;
    };
    match stem.strip_prefix(PARTIAL_PREFIX) {
        Some(name) => SnapshotName::parse(name).map_or(Entry::Unknown, Entry::Partial),
        None => SnapshotName::parse(stem).map_or(Entry::Unknown, Entry::Snapshot),
    }
}

/// Classify a file name inside the legacy `<id>/` directory.
pub(super) fn classify_legacy(file_name: &str) -> Entry {
    match file_name.strip_prefix(PARTIAL_PREFIX) {
        Some(name) => SnapshotName::parse(name).map_or(Entry::Unknown, Entry::Partial),
        None => SnapshotName::parse(file_name).map_or(Entry::Unknown, Entry::Snapshot),
    }
}

/// The name for a new zip: stamp `max(now, greatest recognized stamp)` (a
/// clock that moved backward cannot sort a new snapshot before an old one),
/// counter one above every recognized counter at that stamp, of any suffix,
/// partial or namespace. `None` only on counter overflow. Runs under the
/// namespace lock, so cooperating zip writers never allocate the same name.
pub(super) fn allocate<'a>(
    now: &str,
    suffix: &str,
    existing: impl IntoIterator<Item = &'a SnapshotName> + Clone,
) -> Option<SnapshotName> {
    let stamp = existing
        .clone()
        .into_iter()
        .map(|name| name.stamp.as_str())
        .fold(now, |max, stamp| if stamp > max { stamp } else { max })
        .to_owned();
    let counter = match existing
        .into_iter()
        .filter(|name| name.stamp == stamp)
        .map(|name| name.counter)
        .max()
    {
        Some(highest) => highest.checked_add(1)?,
        None => 1,
    };
    Some(SnapshotName {
        stamp,
        suffix: suffix.into(),
        counter,
    })
}

fn is_stamp(stamp: &str) -> bool {
    let b = stamp.as_bytes();
    b.len() == 19
        && b.iter().enumerate().all(|(i, c)| match i {
            4 | 7 | 13 | 16 => *c == b'-',
            10 => *c == b'_',
            _ => c.is_ascii_digit(),
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn name(raw: &str) -> SnapshotName {
        SnapshotName::parse(raw).unwrap_or_else(|| panic!("{raw} parses"))
    }

    #[test]
    fn the_grammar_accepts_todays_names_and_round_trips() {
        for raw in [
            "2026-10-11_09-00-00",
            "2026-10-11_09-00-00-2",
            "2026-10-11_09-00-00-10",
            "2026-10-11_09-00-00-pre-restore",
            "2026-10-11_09-00-00-pre-restore-3",
            "2026-10-11_09-00-00-pre-journal-rename",
        ] {
            assert_eq!(name(raw).snapshot_name(), raw);
        }
        assert_eq!(
            name("2026-10-11_09-00-00-pre-restore-3").suffix,
            "pre-restore"
        );
        assert_eq!(name("2026-10-11_09-00-00-pre-restore-3").counter, 3);
        assert!(name("2026-10-11_09-00-00-pre-restore").is_pre_restore());
        assert!(!name("2026-10-11_09-00-00-pre-journal-rename").is_pre_restore());
    }

    #[test]
    fn the_grammar_rejects_everything_else() {
        for raw in [
            "",
            "2026-10-11_09-00-0",
            "2026-10-11T09-00-00",
            "2026-10-11_09-00-00-",
            "2026-10-11_09-00-00-1",
            "2026-10-11_09-00-00-02",
            "2026-10-11_09-00-00--x",
            "2026-10-11_09-00-00-Pre",
            "2026-10-11_09-00-00-pre_restore",
            "2026-10-11_09-00-00-99999999999",
            "2026-10-11_09-00-00x",
            "notes",
        ] {
            assert!(SnapshotName::parse(raw).is_none(), "{raw:?}");
        }
    }

    /// The extension and the partial prefix are stripped exactly once.
    #[test]
    fn entries_classify_by_namespace() {
        let stamp = "2026-10-11_09-00-00";
        assert_eq!(
            classify_zip(&format!("{stamp}.zip")),
            Entry::Snapshot(name(stamp))
        );
        assert_eq!(
            classify_zip(&format!(".partial-{stamp}.zip")),
            Entry::Partial(name(stamp))
        );
        assert_eq!(classify_zip("lock"), Entry::Lock);
        for unknown in [
            format!("{stamp}.zip.zip"),
            format!("{stamp}"),
            format!(".partial-.partial-{stamp}.zip"),
            format!("{stamp}.ZIP"),
            "tine-backup.json".to_owned(),
        ] {
            assert_eq!(classify_zip(&unknown), Entry::Unknown, "{unknown}");
        }
        assert_eq!(classify_legacy(stamp), Entry::Snapshot(name(stamp)));
        assert_eq!(
            classify_legacy(&format!(".partial-{stamp}-2")),
            Entry::Partial(name(&format!("{stamp}-2")))
        );
        assert_eq!(classify_legacy(&format!("{stamp}.zip")), Entry::Unknown);
        assert_eq!(classify_legacy("lock"), Entry::Unknown);
    }

    /// The 9 -> 10 bug: order is numeric on the counter, not lexical.
    #[test]
    fn order_is_stamp_then_numeric_counter() {
        let nine = name("2026-10-11_09-00-00-9");
        let ten = name("2026-10-11_09-00-00-10");
        assert_eq!(nine.chrono_cmp(&ten), Ordering::Less);
        let restore = name("2026-10-11_09-00-00-pre-restore-4");
        assert_eq!(restore.chrono_cmp(&ten), Ordering::Less);
        assert_eq!(
            name("2026-10-11_09-00-00").chrono_cmp(&name("2026-10-11_09-00-01")),
            Ordering::Less
        );
    }

    /// Counters are reserved across suffixes, partials and namespaces: a
    /// launch snapshot in the same second as a pre-restore one, or as a
    /// legacy directory, gets the next counter, never a shared order key.
    #[test]
    fn allocation_reserves_above_every_counter_of_that_second() {
        let now = "2026-10-11_09-00-00";
        assert_eq!(allocate(now, "", &[]).unwrap().snapshot_name(), now);
        let existing = [
            name("2026-10-11_09-00-00-pre-restore"),
            name("2026-10-11_09-00-00-9"),
            name("2026-10-10_23-59-59-40"),
        ];
        assert_eq!(
            allocate(now, "", &existing).unwrap().snapshot_name(),
            "2026-10-11_09-00-00-10"
        );
        assert_eq!(
            allocate(now, "pre-restore", &existing)
                .unwrap()
                .snapshot_name(),
            "2026-10-11_09-00-00-pre-restore-10"
        );
        let later = "2026-10-11_09-00-01";
        assert_eq!(
            allocate(later, "", &existing).unwrap().snapshot_name(),
            later
        );
    }

    /// A clock that moved backward: the new name still sorts last.
    #[test]
    fn a_backward_clock_never_sorts_a_new_snapshot_first() {
        let existing = [name("2026-10-11_09-00-00-3")];
        let allocated = allocate("2025-01-01_00-00-00", "pre-restore", &existing).unwrap();
        assert_eq!(
            allocated.snapshot_name(),
            "2026-10-11_09-00-00-pre-restore-4"
        );
        assert_eq!(allocated.chrono_cmp(&existing[0]), Ordering::Greater);
    }

    #[test]
    fn counter_overflow_is_refused_not_wrapped() {
        let existing = [SnapshotName {
            stamp: "2026-10-11_09-00-00".into(),
            suffix: String::new(),
            counter: u32::MAX,
        }];
        assert!(allocate("2026-10-11_09-00-00", "", &existing).is_none());
    }
}
