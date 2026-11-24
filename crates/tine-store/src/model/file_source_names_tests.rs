//! Tine's one reading of "what kind of entry does this graph-relative
//! spelling name" ([`spelled_kind`]): fed the answers Windows gives, so Linux
//! checks the decision the hosted Windows run depends on; counted, so its
//! cost stays one probe on a clean path; and run over the memory and the
//! disk source for the same spellings, which must answer alike.
use super::{parent_spelling, spelled_kind, tine_names, EntryKind, FileSource, Files, MEMORY_ROOT};
use std::cell::{Cell, RefCell};
use std::io::{self, ErrorKind};
use std::path::{Path, PathBuf};
use std::sync::Arc;

thread_local! {
    /// How many probes the disk source made on this thread ([`super::disk_probe`]).
    pub(super) static DISK_PROBES: Cell<usize> = const { Cell::new(0) };
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
/// and the prefixes probed, in order.
fn decide(rel: &str, at: &[(&str, &str)]) -> (String, Vec<String>) {
    let probed = RefCell::new(Vec::new());
    let probe = |prefix: &str, _whole| {
        let clean = tine_names(prefix).collect::<Vec<_>>().join("/");
        probed.borrow_mut().push(clean.clone());
        let found = at.iter().find(|(path, _)| *path == clean);
        answer(found.map_or("missing", |(_, kind)| kind))
    };
    let is_dir = |kind: &EntryKind| matches!(kind, EntryKind::Dir);
    let as_directory = super::spelled_as_directory(rel);
    let decided = spelled_kind(rel, as_directory, probe, is_dir, Path::new(rel));
    (kind_name(&decided), probed.into_inner())
}

/// The six layouts the hosted Windows run opened while the memory source
/// refused them, with Windows' answers: `x` is a file.
#[test]
fn windows_answers_read_as_tine_reads_them() {
    // Under a file: ERROR_PATH_NOT_FOUND, read as NotFound; the file is
    // the nearest ancestor, so one more probe.
    for rel in ["x/pages", "x/j"] {
        let (kind, probed) = decide(rel, &[("x", "file")]);
        assert_eq!((kind.as_str(), probed.len()), ("NotADirectory", 2), "{rel}");
    }
    let (kind, probed) = decide("logseq/config.edn", &[("logseq", "file")]);
    assert_eq!((kind.as_str(), probed.len()), ("NotADirectory", 2));
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

/// Every other answer is the filesystem's, and ancestors are probed only
/// after a `NotFound`, nearest first, up to the first that exists
/// (principle 2: nothing more on a clean path).
#[test]
fn other_answers_are_the_filesystems() {
    // (spelling, what is there, expected, prefixes probed)
    let cases: &[(&str, &[(&str, &str)], &str, &[&str])] = &[
        ("x", &[("x", "file")], "file", &["x"]),
        ("x/", &[("x", "dir")], "dir", &["x"]),
        ("x/.", &[("x", "dir")], "dir", &["x"]),
        ("x/p", &[("x/p", "dir"), ("x", "file")], "dir", &["x/p"]),
        ("a/p", &[("a", "dir")], "NotFound", &["a/p", "a"]),
        ("a/b/p", &[("a", "dir")], "NotFound", &["a/b/p", "a/b", "a"]),
        (
            "x/b/p",
            &[("x", "file")],
            "NotADirectory",
            &["x/b/p", "x/b", "x"],
        ),
        ("x/p", &[("x", "other")], "NotADirectory", &["x/p", "x"]),
        ("a/p", &[("a", "denied")], "NotFound", &["a/p", "a"]),
        ("p", &[], "NotFound", &["p"]),
        ("a/p", &[("a/p", "denied")], "PermissionDenied", &["a/p"]),
        ("x/p", &[("x/p", "notdir")], "NotADirectory", &["x/p"]),
        (
            "a/./b//c/",
            &[("a", "dir")],
            "NotFound",
            &["a/b/c", "a/b", "a"],
        ),
    ];
    for (rel, at, expected, probed) in cases {
        let (kind, made) = decide(rel, at);
        let probed: Vec<String> = probed.iter().map(|prefix| prefix.to_string()).collect();
        assert_eq!((kind.as_str(), made), (*expected, probed), "{rel}");
    }
}

/// Ancestors are borrowed prefixes of the spelling as Tine reads it (empty
/// and `.` names dropped), never the platform's normalization of the joined
/// path, and nothing is built to hold them.
#[test]
fn ancestors_are_borrowed_prefixes_of_tines_names() {
    let cases: [(&str, &[&str]); 7] = [
        ("pages", &[]),
        ("x/", &[]),
        ("x/pages", &["x"]),
        ("x//./pages", &["x"]),
        ("a/b/c", &["a/b", "a"]),
        ("a/./b//c/", &["a/./b", "a"]),
        ("./a/b", &["./a"]),
    ];
    for (rel, expected) in cases {
        let mut ancestors = Vec::new();
        let mut at = parent_spelling(rel);
        while let Some(prefix) = at {
            let start = prefix.as_ptr() as usize - rel.as_ptr() as usize;
            assert_eq!(start, 0, "{rel}: {prefix} is a prefix of the spelling");
            ancestors.push(prefix);
            at = parent_spelling(prefix);
        }
        assert_eq!(ancestors, expected, "{rel}");
    }
}

/// On this platform's disk, the configured spellings of the six cases read
/// as not a directory, a missing directory stays missing, and the probe
/// count is one on a clean path and one per ancestor up to the first that
/// exists after a `NotFound`.
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
        ("a/b/c/d/p", "NotFound", 5),
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
    let read = |rel: &str| {
        let error = std::fs::File::open(root.join(rel)).err().unwrap();
        disk.read_error(root, rel, error).kind()
    };
    assert_eq!(read("x/config.edn"), ErrorKind::NotADirectory);
    assert_eq!(read("a/config.edn"), ErrorKind::NotFound);
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
