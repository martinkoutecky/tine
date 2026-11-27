//! tine#985: a `file:` target outside the graph is never opened or read before
//! the user agrees, nothing outside the graph is probed before then, and no
//! link develop opens is refused.
use super::*;
use std::collections::HashMap;
use std::sync::atomic::AtomicBool;

/// Records every path probed, around the real filesystem.
#[derive(Default)]
struct Recording {
    probed: Vec<PathBuf>,
    read_links: usize,
}

impl Probe for Recording {
    fn kind(&mut self, path: &Path) -> std::io::Result<Kind> {
        self.probed.push(path.to_path_buf());
        Fs.kind(path)
    }
    fn read_link(&mut self, path: &Path) -> std::io::Result<PathBuf> {
        self.probed.push(path.to_path_buf());
        self.read_links += 1;
        Fs.read_link(path)
    }
}

impl Recording {
    fn outside(&self, root: &Path) -> Vec<PathBuf> {
        self.probed
            .iter()
            .filter(|path| !path.starts_with(root))
            .cloned()
            .collect()
    }
}

fn is_ask(result: &Result<impl std::fmt::Debug, String>) -> bool {
    matches!(result, Err(error) if error.starts_with(FILE_CONSENT))
}

fn ask_display(error: &str) -> serde_json::Value {
    serde_json::from_str(error.strip_prefix(FILE_CONSENT).unwrap()).unwrap()
}

/// A fake filesystem with Windows path semantics, so the Windows rules run on
/// every host. Keys are the walker's own `\`-joined paths.
#[derive(Default)]
struct WinFs {
    entries: HashMap<String, Entry>,
    probed: Vec<String>,
}

#[derive(Clone)]
enum Entry {
    Dir,
    File,
    Link(&'static str),
}

impl WinFs {
    fn with(entries: &[(&str, Entry)]) -> Self {
        Self {
            entries: entries
                .iter()
                .map(|(path, entry)| (path.to_string(), entry.clone()))
                .collect(),
            probed: Vec::new(),
        }
    }
}

impl Probe for WinFs {
    fn kind(&mut self, path: &Path) -> std::io::Result<Kind> {
        let key = path.to_str().unwrap().to_string();
        self.probed.push(key.clone());
        match self.entries.get(&key) {
            Some(Entry::Dir) => Ok(Kind::Dir),
            Some(Entry::File) => Ok(Kind::File),
            Some(Entry::Link(_)) => Ok(Kind::Link),
            None => Err(std::io::ErrorKind::NotFound.into()),
        }
    }
    fn read_link(&mut self, path: &Path) -> std::io::Result<PathBuf> {
        let key = path.to_str().unwrap().to_string();
        self.probed.push(key.clone());
        match self.entries.get(&key) {
            Some(Entry::Link(target)) => Ok(PathBuf::from(target)),
            _ => Err(std::io::ErrorKind::InvalidInput.into()),
        }
    }
}

fn win(root: &str, path: &str, fs: &mut WinFs) -> Containment {
    containment(Semantics::Windows, Path::new(root), Path::new(path), fs)
}

fn unproven(display: &str, admitted: &str) -> Containment {
    Containment::Unproven {
        admitted: PathBuf::from(admitted),
        display: display.into(),
    }
}

#[test]
fn windows_alias_dotdot_is_normalized_as_win32_does_before_any_probe() {
    // Regression: `C:\G\alias\..\sample.pdf` (the decoded form of a
    // `file:` URL with encoded separators). Win32 removes `alias\..` before
    // following anything, so the file opened is `C:\G\sample.pdf`, a link out.
    let mut fs = WinFs::with(&[
        ("C:\\G\\alias", Entry::Link("C:\\G\\deep\\leaf")),
        ("C:\\G\\deep", Entry::Dir),
        ("C:\\G\\deep\\leaf", Entry::Dir),
        ("C:\\G\\deep\\sample.pdf", Entry::File),
        ("C:\\G\\sample.pdf", Entry::Link("D:\\out\\s.pdf")),
    ]);
    assert_eq!(
        win("\\\\?\\C:\\G", "C:\\G\\alias\\..\\sample.pdf", &mut fs),
        unproven("D:\\out\\s.pdf", "C:\\G\\sample.pdf"),
        "the admitted path is the one Win32 opens, and it leaves the graph"
    );
    assert_eq!(fs.probed, ["C:\\G\\sample.pdf", "C:\\G\\sample.pdf"]);
}

#[test]
fn windows_foreign_roots_and_case_siblings_ask_with_zero_probes() {
    for path in [
        "D:\\x",
        "\\\\srv\\s\\x",
        "C:\\g\\x",
        "C:\\G\\..\\T\\x",
        "C:\\G.\\x",
    ] {
        let mut fs = WinFs::default();
        assert!(
            matches!(win("C:\\G", path, &mut fs), Containment::Unproven { .. }),
            "{path}"
        );
        assert!(fs.probed.is_empty(), "{path} probed {:?}", fs.probed);
    }
    // The case sibling is handed on exactly as spelled.
    assert_eq!(
        win("C:\\G", "C:\\g\\x", &mut WinFs::default()),
        unproven("C:\\g\\x", "C:\\g\\x")
    );
}

#[test]
fn windows_inside_paths_open_including_drive_case_and_unc_graphs() {
    let mut fs = WinFs::with(&[
        ("C:\\G\\files", Entry::Dir),
        ("C:\\G\\files\\x.txt", Entry::File),
    ]);
    assert_eq!(
        win("\\\\?\\C:\\G", "c:/G/files/x.txt", &mut fs),
        Containment::Inside("C:\\G\\files\\x.txt".into())
    );
    let mut fs = WinFs::with(&[
        ("\\\\srv\\s\\G\\files", Entry::Dir),
        ("\\\\srv\\s\\G\\files\\x.txt", Entry::File),
    ]);
    assert_eq!(
        win(
            "\\\\?\\UNC\\srv\\s\\G",
            "\\\\srv\\s\\G\\files\\x.txt",
            &mut fs
        ),
        Containment::Inside("\\\\srv\\s\\G\\files\\x.txt".into())
    );
    assert!(fs
        .probed
        .iter()
        .all(|path| path.starts_with("\\\\srv\\s\\G\\")));
}

#[test]
fn windows_payloads_are_classified_by_platform_meaning() {
    let links = |target: &'static str| {
        WinFs::with(&[
            ("C:\\G\\r", Entry::Link(target)),
            ("C:\\G\\files", Entry::Dir),
            ("C:\\G\\files\\s.txt", Entry::File),
        ])
    };
    // Rooted `\T`: the root of the link's own volume.
    let mut fs = links("\\T");
    assert_eq!(
        win("C:\\G", "C:\\G\\r\\s.txt", &mut fs),
        unproven("C:\\T\\s.txt", "C:\\G\\r\\s.txt")
    );
    assert!(fs.probed.iter().all(|path| path.starts_with("C:\\G\\")));
    // Drive-relative `D:T` depends on a per-process folder: ask.
    let mut fs = links("D:T");
    assert_eq!(
        win("C:\\G", "C:\\G\\r\\s.txt", &mut fs),
        unproven("D:T\\s.txt", "C:\\G\\r\\s.txt")
    );
    // Junction payloads (`\??\`), and UNC: lexically outside, never probed.
    for target in ["\\??\\D:\\T", "\\\\?\\UNC\\srv\\s", "\\\\srv\\s\\T"] {
        let mut fs = links(target);
        assert!(matches!(
            win("C:\\G", "C:\\G\\r\\s.txt", &mut fs),
            Containment::Unproven { .. }
        ));
        assert!(fs.probed.iter().all(|path| path.starts_with("C:\\G\\")));
    }
    // A rooted payload back inside is Inside.
    let mut fs = links("\\G\\files");
    assert_eq!(
        win("C:\\G", "C:\\G\\r\\s.txt", &mut fs),
        Containment::Inside("C:\\G\\r\\s.txt".into())
    );
    // On a share, `\T` is the share's root.
    let mut fs = WinFs::with(&[("\\\\srv\\s\\G\\r", Entry::Link("\\T"))]);
    assert_eq!(
        win("\\\\srv\\s\\G", "\\\\srv\\s\\G\\r\\s.txt", &mut fs),
        unproven("\\\\srv\\s\\T\\s.txt", "\\\\srv\\s\\G\\r\\s.txt")
    );
    // Relative payloads: `x\..` inside the payload is removed lexically.
    let mut fs = links("files\\nope\\..\\");
    assert_eq!(
        win("C:\\G", "C:\\G\\r\\s.txt", &mut fs),
        Containment::Inside("C:\\G\\r\\s.txt".into())
    );
    assert!(!fs.probed.iter().any(|path| path.contains("nope")));
}

#[test]
fn windows_hop_bound_is_the_platform_bound() {
    // 63 relative links under C:\G ending at x.pdf open; 64 ask.
    let names: Vec<&'static str> = (0..70)
        .map(|index| &*Box::leak(format!("C:\\G\\l{index}").into_boxed_str()))
        .collect();
    let targets: Vec<&'static str> = (0..70)
        .map(|index| &*Box::leak(format!("l{}", index + 1).into_boxed_str()))
        .collect();
    let chain = |length: usize| {
        let mut entries: Vec<(&str, Entry)> = (0..length - 1)
            .map(|index| (names[index], Entry::Link(targets[index])))
            .collect();
        entries.push((names[length - 1], Entry::Link("x.pdf")));
        entries.push(("C:\\G\\x.pdf", Entry::File));
        WinFs::with(&entries)
    };
    let mut fs = chain(63);
    assert_eq!(
        win("C:\\G", "C:\\G\\l0", &mut fs),
        Containment::Inside("C:\\G\\l0".into())
    );
    let mut fs = chain(64);
    assert_eq!(
        win("C:\\G", "C:\\G\\l0", &mut fs),
        unproven("C:\\G\\l0", "C:\\G\\l0")
    );
}

/// The POSIX fixture: graph `G` holding `files/x.txt`, `files/in.pdf`, links
/// `out → T`, `back → G/files`, `loop → loop`; outside it `T/s.txt`, `T/s.pdf`
/// and `T/back → G/files`.
#[cfg(unix)]
struct Fixture {
    _dir: tempfile::TempDir,
    graph: PathBuf,
    outside: PathBuf,
}

#[cfg(unix)]
fn fixture() -> Fixture {
    use std::os::unix::fs::symlink;
    let dir = tempfile::tempdir().unwrap();
    let base = dir.path().canonicalize().unwrap();
    let graph = base.join("G");
    let outside = base.join("T");
    std::fs::create_dir_all(graph.join("files")).unwrap();
    std::fs::create_dir_all(&outside).unwrap();
    std::fs::write(graph.join("files/x.txt"), "inside").unwrap();
    std::fs::write(graph.join("files/in.pdf"), b"%PDF-1.7 inside").unwrap();
    std::fs::write(outside.join("s.txt"), "outside").unwrap();
    std::fs::write(outside.join("s.pdf"), b"%PDF-1.7 outside").unwrap();
    symlink(&outside, graph.join("out")).unwrap();
    symlink(graph.join("files"), graph.join("back")).unwrap();
    symlink("loop", graph.join("loop")).unwrap();
    symlink(graph.join("files"), outside.join("back")).unwrap();
    Fixture {
        _dir: dir,
        graph,
        outside,
    }
}

#[cfg(unix)]
fn graph_binding<'a>(fixture: &'a Fixture, granted: &'a AtomicBool) -> Binding<'a> {
    Binding::Graph {
        root_key: &fixture.graph,
        granted,
    }
}

/// P1 with a recording opener: the result and every path handed to the OS.
#[cfg(unix)]
fn p1(
    binding: &Binding<'_>,
    path: &Path,
    purpose: Purpose,
    probe: &mut Recording,
) -> (Result<(), String>, Vec<PathBuf>) {
    let mut opened = Vec::new();
    let result = open_local(binding, path, purpose, probe, |path| {
        opened.push(path.to_path_buf());
        Ok(())
    });
    (result, opened)
}

#[cfg(unix)]
fn p3(
    fixture: &Fixture,
    granted: &AtomicBool,
    relative: &str,
    purpose: Purpose,
    probe: &mut Recording,
) -> (Result<(), String>, Vec<PathBuf>) {
    let mut opened = Vec::new();
    let result = open_graph_relative(&fixture.graph, granted, relative, purpose, probe, |path| {
        opened.push(path.to_path_buf());
        Ok(())
    });
    (result, opened)
}

#[cfg(unix)]
fn url(path: &Path) -> String {
    tauri::Url::from_file_path(path).unwrap().into()
}

#[cfg(unix)]
#[test]
fn inside_links_open_with_no_prompt_and_probe_only_inside() {
    let f = fixture();
    let granted = AtomicBool::new(false);
    for path in [f.graph.join("files/x.txt"), f.graph.join("back/x.txt")] {
        let mut probe = Recording::default();
        let (result, opened) = p1(
            &graph_binding(&f, &granted),
            &path,
            Purpose::Link,
            &mut probe,
        );
        assert_eq!(result, Ok(()));
        assert_eq!(opened, [path.clone()], "P1 hands on the parsed path");
        assert!(probe.outside(&f.graph).is_empty(), "{:?}", probe.probed);
    }
    let path = crate::platform::file_url_to_path(&url(&f.graph.join("files/x.txt"))).unwrap();
    assert_eq!(
        admit(
            &graph_binding(&f, &granted),
            &path,
            Purpose::Link,
            &mut Recording::default()
        ),
        Ok(Admit::Go(path))
    );
}

#[cfg(unix)]
#[test]
fn outside_links_ask_with_zero_probes_then_open_once_on_agree() {
    let f = fixture();
    let granted = AtomicBool::new(false);
    let target = f.outside.join("s.txt");
    let dotdot = format!("{}/%2e%2e/T/s.txt", url(&f.graph));
    for path in [
        target.clone(),
        f.graph.join("../T/s.txt"),
        crate::platform::file_url_to_path(&dotdot).unwrap(),
    ] {
        let mut probe = Recording::default();
        let (result, opened) = p1(
            &graph_binding(&f, &granted),
            &path,
            Purpose::Link,
            &mut probe,
        );
        assert!(is_ask(&result), "{path:?}: {result:?}");
        assert!(
            probe.probed.is_empty(),
            "{path:?} probed {:?}",
            probe.probed
        );
        assert!(opened.is_empty(), "nothing is opened before consent");
        assert_eq!(
            ask_display(&result.unwrap_err())["display"],
            target.to_str().unwrap(),
            "the prompt names the full path"
        );
        let (result, opened) = p1(
            &graph_binding(&f, &granted),
            &path,
            Purpose::LinkAgreed,
            &mut Recording::default(),
        );
        assert_eq!(result, Ok(()));
        assert_eq!(opened.len(), 1);
    }
    assert!(
        !granted.load(Ordering::Acquire),
        "plain Open remembers nothing"
    );
}

#[cfg(unix)]
#[test]
fn a_link_out_through_a_symlink_names_the_file_open_acts_on() {
    let f = fixture();
    let granted = AtomicBool::new(false);
    let mut probe = Recording::default();
    let (result, opened) = p1(
        &graph_binding(&f, &granted),
        &f.graph.join("out/s.txt"),
        Purpose::Link,
        &mut probe,
    );
    assert!(opened.is_empty());
    assert_eq!(
        ask_display(&result.unwrap_err())["display"],
        f.outside.join("s.txt").to_str().unwrap()
    );
    assert!(probe.outside(&f.graph).is_empty(), "{:?}", probe.probed);
}

#[cfg(unix)]
#[test]
fn graph_relative_links_ask_and_after_open_run_develops_canonical_check() {
    let f = fixture();
    let granted = AtomicBool::new(false);
    // Out to a final outside target: ask; Open gets develop's refusal.
    let mut probe = Recording::default();
    let (result, opened) = p3(&f, &granted, "out/s.txt", Purpose::Link, &mut probe);
    assert!(is_ask(&result) && opened.is_empty() && probe.outside(&f.graph).is_empty());
    let (result, opened) = p3(
        &f,
        &granted,
        "out/s.txt",
        Purpose::LinkAgreed,
        &mut Recording::default(),
    );
    assert!(result.unwrap_err().contains("outside the graph folder"));
    assert!(opened.is_empty());
    // Leave and return: ask, then Open opens the inside file, as develop does.
    let mut probe = Recording::default();
    let (result, opened) = p3(&f, &granted, "out/back/x.txt", Purpose::Link, &mut probe);
    assert!(is_ask(&result) && opened.is_empty() && probe.outside(&f.graph).is_empty());
    let (result, opened) = p3(
        &f,
        &granted,
        "out/back/x.txt",
        Purpose::LinkAgreed,
        &mut Recording::default(),
    );
    assert_eq!(result, Ok(()));
    assert_eq!(opened, [f.graph.join("files/x.txt")]);
    // Inside through an in-graph alias: canonical inside, no prompt.
    let (result, opened) = p3(
        &f,
        &granted,
        "back/x.txt",
        Purpose::Link,
        &mut Recording::default(),
    );
    assert_eq!(result, Ok(()));
    assert_eq!(opened, [f.graph.join("files/x.txt")]);
    // Develop's lexical refusals and missing target are unchanged, zero probes.
    for escaping in [
        "../T/s.txt",
        "files/../../T/s.txt",
        "/etc/passwd",
        "C:/Windows/win.ini",
    ] {
        let mut probe = Recording::default();
        let (result, _) = p3(&f, &granted, escaping, Purpose::Link, &mut probe);
        assert!(
            result.unwrap_err().contains("outside the graph folder"),
            "{escaping}"
        );
        assert!(probe.probed.is_empty());
    }
    let (result, _) = p3(
        &f,
        &granted,
        "files/missing.txt",
        Purpose::Link,
        &mut Recording::default(),
    );
    assert!(result.unwrap_err().contains("could not be found"));
}

#[cfg(unix)]
#[test]
fn pdfs_outside_read_nothing_before_open_and_inside_pdfs_read_directly() {
    use crate::external_pdf::read_pdf;
    let f = fixture();
    let granted = AtomicBool::new(false);
    let outside = url(&f.outside.join("s.pdf"));
    let mut probe = Recording::default();
    let result = read_pdf(
        &graph_binding(&f, &granted),
        &outside,
        Purpose::Link,
        &mut probe,
    );
    assert!(is_ask(&result), "{result:?}");
    assert!(probe.probed.is_empty());
    assert_eq!(
        read_pdf(
            &graph_binding(&f, &granted),
            &outside,
            Purpose::LinkAgreed,
            &mut Fs
        )
        .unwrap(),
        b"%PDF-1.7 outside"
    );
    for inside in ["files/in.pdf", "back/in.pdf"] {
        let result = read_pdf(
            &graph_binding(&f, &granted),
            &url(&f.graph.join(inside)),
            Purpose::Link,
            &mut Recording::default(),
        );
        assert_eq!(result.unwrap(), b"%PDF-1.7 inside", "{inside}");
    }
}

#[cfg(unix)]
#[test]
fn posix_alias_dotdot_twin_reads_the_physically_resolved_inside_file() {
    // The POSIX twin of the Windows blocker: `G/alias/../x.pdf` with
    // `alias → G/deep/leaf`. POSIX resolves `..` physically, so the file is
    // `G/deep/x.pdf`; `G/x.pdf`, a link out, is never read.
    use std::os::unix::fs::symlink;
    let f = fixture();
    std::fs::create_dir_all(f.graph.join("deep/leaf")).unwrap();
    std::fs::write(f.graph.join("deep/x.pdf"), b"%PDF-1.7 deep").unwrap();
    symlink(f.graph.join("deep/leaf"), f.graph.join("alias")).unwrap();
    symlink(f.outside.join("s.pdf"), f.graph.join("x.pdf")).unwrap();
    let encoded = format!("{}/alias%2F..%2Fx.pdf", url(&f.graph));
    assert!(
        crate::platform::file_url_to_path(&encoded)
            .unwrap()
            .ends_with("alias/../x.pdf"),
        "the URL decoder keeps the encoded `..` segment"
    );
    let granted = AtomicBool::new(false);
    let mut probe = Recording::default();
    let bytes = crate::external_pdf::read_pdf(
        &graph_binding(&f, &granted),
        &encoded,
        Purpose::Link,
        &mut probe,
    )
    .unwrap();
    assert_eq!(bytes, b"%PDF-1.7 deep");
    assert!(probe.outside(&f.graph).is_empty());
}

#[cfg(unix)]
#[test]
fn loops_and_chains_over_the_bound_ask_and_in_bound_chains_open() {
    use std::os::unix::fs::symlink;
    let f = fixture();
    let granted = AtomicBool::new(false);
    let looped = f.graph.join("loop/x");
    let mut probe = Recording::default();
    assert_eq!(
        containment(SEMANTICS, &f.graph, &looped, &mut probe),
        Containment::Unproven {
            admitted: looped.clone(),
            display: looped.clone().into_os_string()
        }
    );
    assert_eq!(probe.read_links, POSIX_MAX_HOPS);
    assert!(probe.outside(&f.graph).is_empty());
    // A chain of exactly the bound opens with no prompt; one more asks. The
    // kernel draws the same line.
    for (length, inside) in [(POSIX_MAX_HOPS, true), (POSIX_MAX_HOPS + 1, false)] {
        let chain = f.graph.join(format!("chain{length}"));
        std::fs::create_dir_all(&chain).unwrap();
        for index in 0..length {
            let target = if index + 1 == length {
                "../files/x.txt".to_string()
            } else {
                format!("l{}", index + 1)
            };
            symlink(target, chain.join(format!("l{index}"))).unwrap();
        }
        let head = chain.join("l0");
        let answer = containment(SEMANTICS, &f.graph, &head, &mut Recording::default());
        assert_eq!(
            matches!(answer, Containment::Inside(_)),
            inside,
            "{length}: {answer:?}"
        );
        assert_eq!(
            std::fs::read(&head).is_ok(),
            inside,
            "the kernel's own bound"
        );
        let (result, _) = p1(
            &graph_binding(&f, &granted),
            &head,
            Purpose::Link,
            &mut Recording::default(),
        );
        assert_eq!(is_ask(&result), !inside);
    }
    // A non-directory followed by more components is Missing.
    let through_file = f.graph.join("files/x.txt/../x.txt");
    assert_eq!(
        containment(
            SEMANTICS,
            &f.graph,
            &through_file,
            &mut Recording::default()
        ),
        Containment::Missing(through_file)
    );
}

#[cfg(unix)]
#[test]
fn the_grant_covers_this_binding_only_and_never_settings() {
    let f = fixture();
    let granted = AtomicBool::new(false);
    let target = f.outside.join("s.txt");
    let (result, opened) = p1(
        &graph_binding(&f, &granted),
        &target,
        Purpose::LinkAgreedRemember,
        &mut Recording::default(),
    );
    assert_eq!((result, opened.len()), (Ok(()), 1));
    assert!(granted.load(Ordering::Acquire));
    let mut probe = Recording::default();
    let (result, opened) = p1(
        &graph_binding(&f, &granted),
        &target,
        Purpose::Link,
        &mut probe,
    );
    assert_eq!(
        (result, opened.len()),
        (Ok(()), 1),
        "granted: opens directly"
    );
    // Settings' manifest buttons never use the grant.
    let (result, opened) = p1(
        &graph_binding(&f, &granted),
        &target,
        Purpose::Settings,
        &mut Recording::default(),
    );
    assert_eq!(result, Err(OUTSIDE_FILE.into()));
    assert!(opened.is_empty());
    // Inside, Settings opens as before.
    let (result, _) = p1(
        &graph_binding(&f, &granted),
        &f.graph.join("files/x.txt"),
        Purpose::Settings,
        &mut Recording::default(),
    );
    assert_eq!(result, Ok(()));
    // Another binding (switch, window, restart) starts ungranted.
    let fresh = AtomicBool::new(false);
    let (result, _) = p1(
        &graph_binding(&f, &fresh),
        &target,
        Purpose::Link,
        &mut Recording::default(),
    );
    assert!(is_ask(&result));
}

#[cfg(unix)]
#[test]
fn quick_capture_asks_for_every_file_and_remembers_nothing() {
    let f = fixture();
    for path in [f.graph.join("files/x.txt"), f.outside.join("s.txt")] {
        let mut probe = Recording::default();
        let (result, opened) = p1(&Binding::Capture, &path, Purpose::Link, &mut probe);
        assert!(is_ask(&result) && opened.is_empty() && probe.probed.is_empty());
        let (result, opened) = p1(
            &Binding::Capture,
            &path,
            Purpose::LinkAgreedRemember,
            &mut Recording::default(),
        );
        assert_eq!((result, opened.len()), (Ok(()), 1));
        let (result, _) = p1(
            &Binding::Capture,
            &path,
            Purpose::Link,
            &mut Recording::default(),
        );
        assert!(is_ask(&result), "nothing was remembered");
    }
}

#[test]
fn a_capture_call_from_an_earlier_show_or_a_hidden_window_is_stale() {
    let state = crate::state::AppState {
        graphs: std::sync::RwLock::new(Default::default()),
        graph_load: std::sync::Mutex::new(()),
        last_focused: std::sync::Mutex::new(None),
        capture_graph: std::sync::Mutex::new(Default::default()),
        #[cfg(desktop)]
        next_window: std::sync::atomic::AtomicU64::new(2),
    };
    state.bind_capture_graph("main".into(), 7);
    let show_a = state.bound_capture_show().unwrap();
    assert_eq!(capture_show_admits(&state, Some(show_a), true), Ok(()));
    assert!(capture_show_admits(&state, None, true).is_err());
    // A close request hides natively; the frontend may not know.
    assert_eq!(
        capture_show_admits(&state, Some(show_a), false),
        Err(crate::state::STALE_BINDING.into())
    );
    state.bind_capture_graph("main".into(), 7);
    let show_b = state.bound_capture_show().unwrap();
    assert_ne!(show_a, show_b);
    assert_eq!(
        capture_show_admits(&state, Some(show_a), true),
        Err(crate::state::STALE_BINDING.into()),
        "show A's click is refused in show B, whether or not capture-shown arrived"
    );
    assert_eq!(capture_show_admits(&state, Some(show_b), true), Ok(()));
}

#[cfg(unix)]
#[test]
fn unreadable_names_get_a_readable_display_and_the_exact_bytes_open() {
    use std::os::unix::ffi::OsStrExt;
    let f = fixture();
    let name = std::ffi::OsStr::from_bytes(b"n\xff\x01.txt");
    let target = f.outside.join(name);
    std::fs::write(&target, "x").unwrap();
    let granted = AtomicBool::new(false);
    let (result, _) = p1(
        &graph_binding(&f, &granted),
        &target,
        Purpose::Link,
        &mut Recording::default(),
    );
    let ask = ask_display(&result.unwrap_err());
    assert_eq!(ask["lossy"], true);
    let display = ask["display"].as_str().unwrap();
    assert!(display.ends_with("n\u{FFFD}\u{FFFD}.txt"), "{display}");
    let (result, opened) = p1(
        &graph_binding(&f, &granted),
        &target,
        Purpose::LinkAgreed,
        &mut Recording::default(),
    );
    assert_eq!(result, Ok(()));
    assert_eq!(opened, [target]);
}

#[cfg(unix)]
#[test]
fn no_link_develop_opens_is_refused() {
    use std::os::unix::fs::symlink;
    let f = fixture();
    symlink("../files/x.txt", f.graph.join("files/alias.txt")).unwrap();
    let granted = AtomicBool::new(false);
    for path in [
        f.graph.join("files/x.txt"),
        f.graph.join("back/x.txt"),
        f.graph.join("out/back/x.txt"),
        f.graph.join("files/alias.txt"),
        f.outside.join("s.txt"),
        f.graph.join("loop/x"),
    ] {
        let answer = admit(&graph_binding(&f, &granted), &path, Purpose::Link, &mut Fs);
        assert!(answer.is_ok(), "{path:?}: {answer:?}");
    }
}

#[test]
fn an_uppercase_file_scheme_never_reaches_the_url_opener() {
    assert!(crate::platform::external_open_plan("FILE:///T/x", Some("file")).is_err());
    assert!(crate::platform::external_open_plan("File:///T/x", None).is_err());
}

#[test]
fn the_ask_wire_is_the_exact_token_the_frontend_decodes() {
    // src/fileConsentWire.ts recognizes only this shape: sorted keys, no spaces.
    let ask = Admit::Ask {
        display: "/T/a \"b\"\\c\u{FFFD}".into(),
        lossy: true,
    };
    assert_eq!(
        ask.into_path(),
        Err(r#"file-consent:{"display":"/T/a \"b\"\\c�","lossy":true}"#.to_string())
    );
}
