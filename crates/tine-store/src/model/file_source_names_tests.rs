//! Tine's one reading of "what kind of entry does this graph-relative
//! spelling name" ([`spelled_kind`] over a source's lookup): fed the answers
//! Windows gives, so Linux checks the decision the hosted Windows run
//! depends on; counted, so its cost stays one probe on a clean path and
//! linear in the depth; and run over the memory and the disk source for the
//! same spellings, which must answer alike.
use super::{
    locate_deepest, spelled_kind, spelled_path, tine_names, EntryKind, FileSource, Files,
    MEMORY_ROOT,
};
use std::cell::{Cell, RefCell};
use std::io::{self, ErrorKind};
use std::path::{Path, PathBuf};
use std::sync::Arc;

thread_local! {
    /// How many probes the disk source made on this thread ([`locate_deepest`]).
    pub(super) static DISK_PROBES: Cell<usize> = const { Cell::new(0) };
    /// How many names a lookup visited on this thread: a memory walk's step,
    /// or a name the disk wrote into a probed path.
    pub(super) static NAME_VISITS: Cell<usize> = const { Cell::new(0) };
    /// How many path components below the graph root the disk source
    /// submitted to the filesystem on this thread: a whole path's names, or
    /// the one name of a step relative to a directory handle.
    pub(super) static COMPONENTS: Cell<usize> = const { Cell::new(0) };
}

/// A missing suffix costs a linear number of components submitted to the
/// filesystem, not one full prefix per missing name: the review's cases
/// (depth 128 and 256 with the first name absent, 8,256 and 32,896
/// components; half the names present, 6,240 and 24,768) and depths 1 and
/// 512. The bound is 3 per name: the whole path once (`depth`), then at most
/// two per existing prefix (its probe and the opening of its directory) and
/// one for the first missing name. Run with `--nocapture` for the counts.
#[test]
fn a_missing_suffix_costs_linear_components() {
    let mut wrong = Vec::new();
    for depth in [1usize, 128, 256, 512] {
        let spelling = vec!["n"; depth].join("/");
        let mut present = vec![0, depth / 2];
        present.dedup();
        for present in present {
            let temp = tempfile::tempdir().unwrap();
            if present > 0 {
                std::fs::create_dir_all(temp.path().join(vec!["n"; present].join("/"))).unwrap();
            }
            let (components, probes) = (COMPONENTS.with(Cell::get), DISK_PROBES.with(Cell::get));
            let started = std::time::Instant::now();
            let answer = kind_name(&FileSource::Disk.lstat_spelled(temp.path(), &spelling));
            let took = started.elapsed();
            let components = COMPONENTS.with(Cell::get) - components;
            let probes = DISK_PROBES.with(Cell::get) - probes;
            eprintln!(
                "depth {depth}, {present} present, disk: {answer}, {probes} probes, \
                 {components} components, {took:?}"
            );
            if answer != "NotFound" || components > 3 * depth {
                wrong.push(format!(
                    "depth {depth}, {present} present: {answer}, {components} components"
                ));
            }
        }
    }
    assert!(wrong.is_empty(), "{wrong:#?}");
}

/// A lookup's cost is linear in its spelling's depth on both sources, for a
/// spelling that exists, one under a file halfway down, and one missing
/// halfway down (the review's 65 → 4,224 visits at depth 128): at most one
/// visit per name built into the probed path, and on disk one probe, plus
/// one step per existing prefix and one for the first missing name when
/// the platform answers `NotFound` (components submitted:
/// [`a_missing_suffix_costs_linear_components`]). Run with `--nocapture` for the
/// counts.
#[test]
fn a_lookup_visits_each_name_once() {
    let mut wrong = Vec::new();
    for depth in [1usize, 128, 512] {
        let half = depth.div_ceil(2);
        let spelling = vec!["n"; depth].join("/");
        let upto = |names: usize| vec!["n"; names].join("/");
        let temp = tempfile::tempdir().unwrap();
        let (whole, under, missing) = (
            temp.path().join("whole"),
            temp.path().join("under"),
            temp.path().join("missing"),
        );
        std::fs::create_dir_all(whole.join(&spelling)).unwrap();
        std::fs::create_dir_all(under.join(upto(half - 1))).unwrap();
        std::fs::write(under.join(upto(half)), "x").unwrap();
        std::fs::create_dir_all(missing.join(upto(half - 1))).unwrap();
        let memory_root = PathBuf::from(MEMORY_ROOT);
        let held = |rel: String| (rel, super::Content::Bytes(Arc::default()), None);
        let memory = |files: Vec<String>| {
            FileSource::Memory(Arc::new(
                Files::new(&memory_root, files.into_iter().map(held)).unwrap(),
            ))
        };
        let below = |names: usize| {
            if names == 0 {
                "f".to_owned()
            } else {
                format!("{}/f", upto(names))
            }
        };
        let cases = [
            ("exists", whole, memory(vec![format!("{spelling}/f")])),
            ("under a file", under, memory(vec![upto(half)])),
            ("missing", missing, memory(vec![below(half - 1)])),
        ];
        for (case, root, held) in cases {
            for (source, files, at) in [
                ("disk", FileSource::Disk, root.as_path()),
                ("memory", held, memory_root.as_path()),
            ] {
                let (visits, probes) = (NAME_VISITS.with(Cell::get), DISK_PROBES.with(Cell::get));
                let answer = kind_name(&files.lstat_spelled(at, &spelling));
                let visits = NAME_VISITS.with(Cell::get) - visits;
                let probes = DISK_PROBES.with(Cell::get) - probes;
                eprintln!("depth {depth}, {case}, {source}: {answer}, {visits} name visits, {probes} probes");
                if visits > depth + 1 {
                    wrong.push(format!("depth {depth}, {case}, {source}: {visits} visits"));
                }
            }
        }
    }
    assert!(wrong.is_empty(), "{wrong:#?}");
}

fn kind_name(answer: &io::Result<EntryKind>) -> String {
    match answer {
        Ok(EntryKind::File) => "file".into(),
        Ok(EntryKind::Dir) => "dir".into(),
        Ok(EntryKind::Other) => "other".into(),
        Err(error) => format!("{:?}", error.kind()),
    }
}

fn answer(name: &str) -> io::Result<EntryKind> {
    match name {
        "dir" => Ok(EntryKind::Dir),
        "file" => Ok(EntryKind::File),
        "other" => Ok(EntryKind::Other),
        "missing" => Err(ErrorKind::NotFound.into()),
        "denied" => Err(ErrorKind::PermissionDenied.into()),
        "notdir" => Err(ErrorKind::NotADirectory.into()),
        _ => unreachable!("{name}"),
    }
}

/// The decision for `rel` given what a filesystem answers at each prefix
/// (`at`, by clean names joined with `/`; an unlisted prefix is missing),
/// and the prefixes probed, in order: the disk's lookup of the one path
/// built for `rel`, then its walk down from the root, each step one name
/// below the prefix the step before found.
fn decide(rel: &str, at: &[(&str, &str)]) -> (String, Vec<String>) {
    let root = Path::new("r");
    let spelled = tine_names(rel).unwrap();
    let (path, names) = spelled_path(root, spelled.clone());
    let probed = RefCell::new(Vec::new());
    let probe = |clean: String| {
        probed.borrow_mut().push(clean.clone());
        let found = at.iter().find(|(path, _)| *path == clean);
        answer(found.map_or("missing", |(_, kind)| kind))
    };
    let whole = path.strip_prefix(root).unwrap().components();
    let whole = whole.map(|name| name.as_os_str().to_str().unwrap());
    let whole = probe(whole.collect::<Vec<_>>().join("/"));
    let mut below = String::new();
    let step = |name: &str| {
        if !below.is_empty() {
            below.push('/');
        }
        below.push_str(name);
        probe(below.clone())
    };
    let is_dir = |kind: &EntryKind| matches!(kind, EntryKind::Dir);
    let located = locate_deepest(spelled, names, whole, is_dir, step);
    let as_directory = super::spelled_as_directory(rel);
    let decided = spelled_kind(as_directory, located, is_dir, Path::new(rel));
    (kind_name(&decided), probed.into_inner())
}

/// The six layouts the hosted Windows run opened while the memory source
/// refused them, with Windows' answers: `x` is a file.
#[test]
fn windows_answers_read_as_tine_reads_them() {
    // Under a file: ERROR_PATH_NOT_FOUND, read as NotFound; the file is
    // the first step down from the root, so one more probe.
    for rel in ["x/pages", "x/j"] {
        let (kind, probed) = decide(rel, &[("x", "file")]);
        assert_eq!((kind.as_str(), probed.len()), ("NotADirectory", 2), "{rel}");
    }
    // A file spelled as a directory: probed by its clean names, found.
    for rel in ["x/", "x/.", "x//./"] {
        let (kind, probed) = decide(rel, &[("x", "file")]);
        assert_eq!(
            (kind.as_str(), probed),
            ("NotADirectory", vec!["x".into()]),
            "{rel}"
        );
    }
}

/// Every other answer is the filesystem's, and prefixes are probed only
/// after a `NotFound`, top down from the root, up to the first that is
/// missing or not a directory, never the whole path again (principle 2:
/// nothing more on a clean path).
#[test]
fn other_answers_are_the_filesystems() {
    // (spelling, what is there, expected, prefixes probed)
    let cases: &[(&str, &[(&str, &str)], &str, &[&str])] = &[
        ("x", &[("x", "file")], "file", &["x"]),
        ("x/", &[("x", "dir")], "dir", &["x"]),
        ("x/.", &[("x", "dir")], "dir", &["x"]),
        ("x/p", &[("x/p", "dir"), ("x", "file")], "dir", &["x/p"]),
        ("a/p", &[("a", "dir")], "NotFound", &["a/p", "a"]),
        ("a/b/p", &[("a", "dir")], "NotFound", &["a/b/p", "a", "a/b"]),
        (
            "a/b/p",
            &[("a", "dir"), ("a/b", "dir")],
            "NotFound",
            &["a/b/p", "a", "a/b"],
        ),
        ("x/b/p", &[("x", "file")], "NotADirectory", &["x/b/p", "x"]),
        (
            "a/x/b/p",
            &[("a", "dir"), ("a/x", "file")],
            "NotADirectory",
            &["a/x/b/p", "a", "a/x"],
        ),
        ("x/p", &[("x", "other")], "NotADirectory", &["x/p", "x"]),
        ("a/p", &[("a", "denied")], "NotFound", &["a/p", "a"]),
        (
            "a/b/c/p",
            &[("a", "dir"), ("a/b", "denied")],
            "NotFound",
            &["a/b/c/p", "a", "a/b"],
        ),
        ("p", &[], "NotFound", &["p"]),
        ("a/p", &[("a/p", "denied")], "PermissionDenied", &["a/p"]),
        ("x/p", &[("x/p", "notdir")], "NotADirectory", &["x/p"]),
        (
            "a/./b//c/",
            &[("a", "dir")],
            "NotFound",
            &["a/b/c", "a", "a/b"],
        ),
    ];
    for (rel, at, expected, probed) in cases {
        let (kind, made) = decide(rel, at);
        let probed: Vec<String> = probed.iter().map(|prefix| prefix.to_string()).collect();
        assert_eq!((kind.as_str(), made), (*expected, probed), "{rel}");
    }
}

/// A probed path is the root's own spelling, then Tine's names (empty and
/// `.` names dropped) after the platform separator, built once; never the
/// platform's normalization of the joined spelling.
#[test]
fn a_probed_path_is_the_roots_spelling_then_tines_names() {
    let sep = std::path::MAIN_SEPARATOR_STR;
    let cases: [(&str, &str, &[&str], usize); 6] = [
        ("r", "pages", &["r", "pages"], 1),
        ("r", "x/", &["r", "x"], 1),
        ("r", "x//./pages", &["r", "x", "pages"], 2),
        ("r", "a/./b//c/", &["r", "a", "b", "c"], 3),
        ("r/", "./a/b", &["r", "a", "b"], 2),
        ("", "a/b", &["a", "b"], 2),
    ];
    for (root, rel, expected, count) in cases {
        let (path, names) = spelled_path(Path::new(root), tine_names(rel).unwrap());
        let mut want = expected.join(sep);
        if root == "r/" {
            want = format!("r/{}", expected[1..].join(sep));
        }
        assert_eq!(
            (path.to_str().unwrap(), names),
            (want.as_str(), count),
            "{root} {rel}"
        );
    }
}

/// On Windows, a name holding `:` is refused before anything is probed: a
/// drive-relative `C:` would be read against drive C's current directory,
/// outside the graph, and `name:stream` as an alternate data stream.
#[cfg(windows)]
#[test]
fn windows_refuses_a_name_with_a_colon_before_probing() {
    let temp = tempfile::tempdir().unwrap();
    std::fs::write(temp.path().join("x"), "x").unwrap();
    for rel in ["x/C:", "x/C:notes/", "pages:stream", "a/b:c"] {
        let before = DISK_PROBES.with(Cell::get);
        let answer = FileSource::Disk.lstat_spelled(temp.path(), rel);
        assert_eq!(kind_name(&answer), "InvalidInput", "{rel}");
        assert_eq!(DISK_PROBES.with(Cell::get), before, "{rel}: nothing probed");
    }
}

/// On this platform's disk, the configured spellings of the six cases read
/// as not a directory, a missing directory stays missing, and the probe
/// count is one on a clean path and, after a `NotFound`, one per prefix
/// walked down from the root up to the first missing name.
#[test]
fn the_disk_reads_configured_spellings_by_tines_names() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    std::fs::write(root.join("x"), "x").unwrap();
    std::fs::create_dir(root.join("a")).unwrap();
    let disk = FileSource::Disk;
    let counted = |rel: &str| {
        let before = DISK_PROBES.with(Cell::get);
        let answer = kind_name(&disk.lstat_spelled(root, rel));
        (answer, DISK_PROBES.with(Cell::get) - before)
    };
    let cases = [
        ("x", "file", 1),
        ("a/", "dir", 1),
        ("x/", "NotADirectory", 1),
        ("x/.", "NotADirectory", 1),
        ("x//./", "NotADirectory", 1),
        ("a/p", "NotFound", 2),
        ("b/p", "NotFound", 2),
        ("a/b/c/d/p", "NotFound", 3),
    ];
    for (rel, kind, probes) in cases {
        assert_eq!(counted(rel), (kind.into(), probes), "{rel}");
    }
    // Under a file: Unix answers ENOTDIR itself (one probe); Windows answers
    // NotFound and the ancestor `x` settles it (two).
    let under = if cfg!(unix) { 1 } else { 2 };
    for rel in ["x/pages", "x/j", "x/./pages"] {
        assert_eq!(counted(rel), ("NotADirectory".into(), under), "{rel}");
    }
}

/// The same spellings over the memory and the disk source give the same
/// kinds: both are [`spelled_kind`] over their own probe.
#[test]
fn memory_and_disk_name_the_same_kinds() {
    let files = ["x", "d/f", "d/e/g", "logseq/config.edn"];
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    for rel in files {
        let path = root.join(rel);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, "x").unwrap();
    }
    let memory_root = PathBuf::from(MEMORY_ROOT);
    let held = files.map(|rel| (rel.to_owned(), super::Content::Bytes(Arc::default()), None));
    let memory = FileSource::Memory(Arc::new(Files::new(&memory_root, held).unwrap()));
    let long = "n".repeat(256);
    let spellings = [
        "x",
        "x/",
        "x/.",
        "x//./",
        "x/p",
        "x/p/q",
        "x/./p",
        "d",
        "d/",
        "d/.",
        "d//e/./",
        "d/f",
        "d/f/",
        "d/f/g",
        "d/e/g",
        "d/e/g/h",
        "d/missing",
        "d/missing/p",
        "missing/p",
        "logseq",
        "logseq/config.edn",
        "logseq/config.edn/",
        "logseq/config.edn/x",
    ]
    .map(str::to_owned);
    let mut wrong = Vec::new();
    let all = spellings.iter().cloned().chain([format!("d/{long}")]);
    for rel in all {
        let disk = kind_name(&FileSource::Disk.lstat_spelled(root, &rel));
        let held = kind_name(&memory.lstat_spelled(&memory_root, &rel));
        if disk != held {
            wrong.push(format!("{rel}: disk {disk}, memory {held}"));
        }
    }
    assert!(wrong.is_empty(), "{wrong:#?}");
}
