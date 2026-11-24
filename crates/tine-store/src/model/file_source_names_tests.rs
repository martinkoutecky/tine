//! Tine's reading of "what is at this graph path" from what a filesystem
//! reported ([`by_tine_names`]), fed the answers Windows gives, so Linux
//! checks the decision the hosted Windows run depends on.
use super::{ancestor_spellings, by_tine_names, EntryKind, FileSource};
use std::io::{self, ErrorKind};

fn kind_name(answer: &io::Result<EntryKind>) -> String {
    match answer {
        Ok(EntryKind::File) => "file".into(),
        Ok(EntryKind::Dir) => "dir".into(),
        Ok(EntryKind::Other) => "other".into(),
        Err(error) => format!("{:?}", error.kind()),
    }
}

fn error(kind: ErrorKind) -> io::Result<EntryKind> {
    Err(io::Error::from(kind))
}

/// The answer for `rel` given the OS's `reported` answer and its answers for
/// the ancestors Tine names, nearest first; also how many ancestors were read.
fn decide(
    rel: &str,
    reported: io::Result<EntryKind>,
    ancestors: Vec<io::Result<EntryKind>>,
) -> (String, usize) {
    let read = std::cell::Cell::new(0);
    let spelled = super::names_a_directory(std::path::Path::new(rel));
    let answer = by_tine_names(
        spelled,
        reported,
        |kind| matches!(kind, EntryKind::Dir),
        || ancestors.into_iter().inspect(|_| read.set(read.get() + 1)),
    );
    (kind_name(&answer), read.get())
}

/// The six layouts the hosted Windows run opened while the memory source
/// refused them, with Windows' answers: `x` is a file.
#[test]
fn windows_answers_read_as_tine_reads_them() {
    let file = || Ok(EntryKind::File);
    let not_found = || error(ErrorKind::NotFound);
    // Under a file: ERROR_PATH_NOT_FOUND, read as NotFound.
    for rel in ["x/pages", "x/j", "logseq/config.edn"] {
        assert_eq!(
            decide(rel, not_found(), vec![file()]),
            ("NotADirectory".into(), 1),
            "{rel}"
        );
    }
    // A file spelled as a directory: the trailing `/` and `.` are dropped
    // and the file is found.
    for rel in ["x/", "x/.", "x//./"] {
        let expected = ("NotADirectory".into(), 0);
        assert_eq!(decide(rel, file(), vec![]), expected, "{rel}");
    }
}

/// Every other answer is the filesystem's, and ancestors are read only after
/// a `NotFound`, up to the first that exists (principle 2: nothing on a
/// clean open's path).
#[test]
fn other_answers_are_the_filesystems() {
    let answer = |name: &str| match name {
        "dir" => Ok(EntryKind::Dir),
        "file" => Ok(EntryKind::File),
        "other" => Ok(EntryKind::Other),
        "missing" => error(ErrorKind::NotFound),
        "denied" => error(ErrorKind::PermissionDenied),
        "notdir" => error(ErrorKind::NotADirectory),
        _ => unreachable!("{name}"),
    };
    // (path, the OS's answer, its ancestors' nearest first, expected,
    // ancestors read)
    let cases = [
        ("x", "file", "", "file", 0),
        ("x/", "dir", "", "dir", 0),
        ("x/.", "dir", "", "dir", 0),
        ("x/p", "dir", "file", "dir", 0),
        ("a/p", "missing", "dir file", "NotFound", 1),
        ("a/b/p", "missing", "missing dir", "NotFound", 2),
        ("x/b/p", "missing", "missing file", "NotADirectory", 2),
        ("x/p", "missing", "other", "NotADirectory", 1),
        ("a/p", "missing", "denied file", "NotFound", 1),
        ("p", "missing", "", "NotFound", 0),
        ("a/p", "denied", "file", "PermissionDenied", 0),
        ("x/p", "notdir", "file", "NotADirectory", 0),
    ];
    for (rel, reported, ancestors, expected, read) in cases {
        let ancestors = ancestors.split_whitespace().map(answer).collect();
        let decided = decide(rel, answer(reported), ancestors);
        assert_eq!(decided, (expected.into(), read), "{rel} {reported}");
    }
}

/// Ancestors are Tine's names (split on `/`, empty and `.` dropped), never
/// the platform's normalization of the joined path.
#[test]
fn ancestors_are_tines_names() {
    let cases: [(&str, &[&str]); 6] = [
        ("pages", &[]),
        ("x/", &[]),
        ("x/pages", &["x"]),
        ("x//./pages", &["x"]),
        ("a/b/c", &["a/b", "a"]),
        ("a/./b//c/", &["a/b", "a"]),
    ];
    for (rel, expected) in cases {
        assert_eq!(ancestor_spellings(rel), expected, "{rel}");
    }
}

/// On this platform's disk, the configured spellings of the six cases read
/// as not a directory, and a missing directory stays missing.
#[test]
fn the_disk_reads_configured_spellings_by_tines_names() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    std::fs::write(root.join("x"), "x").unwrap();
    std::fs::create_dir(root.join("a")).unwrap();
    let disk = FileSource::Disk;
    for rel in ["x/pages", "x/j", "x/", "x/.", "x//./", "x/./pages"] {
        let answer = disk.lstat_spelled(root, rel);
        assert_eq!(kind_name(&answer), "NotADirectory", "{rel}");
    }
    assert_eq!(kind_name(&disk.lstat_spelled(root, "x")), "file");
    assert_eq!(kind_name(&disk.lstat_spelled(root, "a/")), "dir");
    assert_eq!(kind_name(&disk.lstat_spelled(root, "a/p")), "NotFound");
    assert_eq!(kind_name(&disk.lstat_spelled(root, "b/p")), "NotFound");
    let config = root.join("x/config.edn");
    let read = std::fs::File::open(&config).unwrap_err();
    let kind = disk.read_error(&config, read).kind();
    assert_eq!(kind, ErrorKind::NotADirectory);
    let config = root.join("a/config.edn");
    let read = std::fs::File::open(&config).unwrap_err();
    assert_eq!(disk.read_error(&config, read).kind(), ErrorKind::NotFound);
}
