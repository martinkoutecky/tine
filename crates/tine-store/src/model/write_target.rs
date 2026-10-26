//! Bounded containment checks shared by transactions and actual host I/O.
use super::*;

#[cfg(test)]
thread_local! {
    static LAST_CHECKED_TARGET: std::cell::RefCell<Option<PathBuf>> = const { std::cell::RefCell::new(None) };
}

/// RG7: graph mutation primitives must follow a successful containment check
/// on this thread. Private app-data I/O uses the lower-level primitives.
#[cfg(test)]
pub(crate) fn assert_checked_target(target: &Path) {
    LAST_CHECKED_TARGET.with(|checked| {
        assert!(
            checked
                .borrow()
                .as_ref()
                .is_some_and(|path| target.starts_with(path)),
            "RG7: graph mutation target {} has no matching containment check (last: {:?})",
            target.display(),
            checked.borrow(),
        );
    });
}

/// `Graph::ensure_write_target`'s answer for `root` (named `label`).
pub(crate) fn checked_write_target(
    root: &Path,
    target: &Path,
    label: &str,
    binding: Option<&crate::root_binding::RootBinding>,
) -> io::Result<()> {
    #[cfg(test)]
    LAST_CHECKED_TARGET.with(|checked| *checked.borrow_mut() = None);
    if let Some(binding) = binding {
        binding.check()?;
    }
    match write_target_contained(root, target) {
        Ok(true) => {
            #[cfg(test)]
            LAST_CHECKED_TARGET.with(|checked| *checked.borrow_mut() = Some(target.to_path_buf()));
            Ok(())
        }
        Ok(false) => Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("write target escapes {label}: {}", target.display()),
        )),
        Err(error) if error.kind() == io::ErrorKind::InvalidInput => Err(error),
        Err(error) => Err(io::Error::new(
            error.kind(),
            format!(
                "could not inspect write target {}: {error}",
                target.display()
            ),
        )),
    }
}

/// Pauses before re-probing a target that cannot be inspected (under 50 ms in
/// all): Windows refuses to open a page for a moment while a save replaces it
/// (REG-WIN-WRITE-TARGET-TRANSIENT).
const TARGET_PROBE_PAUSES: [std::time::Duration; 3] = [
    std::time::Duration::from_millis(2),
    std::time::Duration::from_millis(8),
    std::time::Duration::from_millis(32),
];

/// A write target's containment: `Ok(false)` when it resolves outside `root`,
/// through an alias of a managed directory (`path_uses_managed_alias`), or
/// through a link that does not resolve. One that cannot be inspected (after
/// `canonical_existing_ancestor`'s re-probes) is answered as that I/O error:
/// unverified, which the caller fails, rather than outside, which it refuses.
fn write_target_contained(root: &Path, target: &Path) -> io::Result<bool> {
    #[cfg(test)]
    TARGET_PROBES.with(|counts| {
        let (r, l, c) = counts.get();
        counts.set((r + 1, l, c));
    });
    let canonical_root = canonical_existing_path(root).unwrap_or_else(|_| root.to_path_buf());
    let (existing, resolved) = match reprobed_existing_ancestor(target) {
        Ok(found) => found,
        Err(Unresolved::Link(_)) => return Ok(false),
        Err(Unresolved::Io(error)) => return Err(error),
    };
    let Ok(relative) = existing.strip_prefix(root) else {
        return Ok(false);
    };
    Ok(resolved == canonical_root.join(relative))
}

/// Why the deepest existing ancestor did not resolve.
enum Unresolved {
    /// A link whose target does not resolve (dangling or looping): it is not
    /// at its own canonical location, and re-probing cannot change that.
    Link(io::Error),
    /// The entry could not be inspected.
    Io(io::Error),
}

/// Resolve the deepest existing ancestor without following a missing suffix.
/// An entry that cannot be inspected is re-probed after each pause, then
/// answered as that error. An existing entry that is not a link but cannot be
/// opened (Windows, while a save replaces it) resolves through its parent: a
/// link-free entry's canonical path is its parent's plus its name.
pub(crate) fn canonical_existing_ancestor(target: &Path) -> io::Result<(&Path, PathBuf)> {
    reprobed_existing_ancestor(target).map_err(|unresolved| match unresolved {
        Unresolved::Link(error) | Unresolved::Io(error) => error,
    })
}

fn reprobed_existing_ancestor(target: &Path) -> Result<(&Path, PathBuf), Unresolved> {
    let mut pauses = TARGET_PROBE_PAUSES.iter();
    loop {
        match existing_ancestor_once(target) {
            Err(Unresolved::Io(error)) if error.kind() != io::ErrorKind::InvalidInput => {
                match pauses.next() {
                    Some(pause) => std::thread::sleep(*pause),
                    None => return Err(Unresolved::Io(error)),
                }
            }
            answer => return answer,
        }
    }
}

fn existing_ancestor_once(target: &Path) -> Result<(&Path, PathBuf), Unresolved> {
    let mut existing = target;
    loop {
        match probe_lstat(existing) {
            Ok(metadata) => {
                let resolved = match probe_canonical(existing) {
                    Ok(resolved) if metadata.file_type().is_symlink() => resolved,
                    Ok(resolved) => in_own_folder(existing, resolved).map_err(Unresolved::Io)?,
                    Err(error) if metadata.file_type().is_symlink() => {
                        return Err(Unresolved::Link(error))
                    }
                    Err(error) => match (existing.parent(), existing.file_name()) {
                        (Some(parent), Some(name)) => {
                            probe_canonical(parent).map_err(Unresolved::Io)?.join(name)
                        }
                        _ => return Err(Unresolved::Io(error)),
                    },
                };
                return Ok((existing, resolved));
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                existing = existing.parent().ok_or_else(|| {
                    Unresolved::Io(io::Error::new(
                        io::ErrorKind::InvalidInput,
                        "target has no existing ancestor",
                    ))
                })?;
            }
            Err(error) => return Err(Unresolved::Io(error)),
        }
    }
}

/// A link-free entry's resolution, checked against its folder. Windows can
/// resolve a page that a save's rename is replacing to where the replaced file
/// went (`\$Extend\$Deleted\…`) rather than where its name is
/// (REG-WIN-WRITE-TARGET-DISPLACED). An entry that resolves under another name
/// is answered as its folder's resolution plus its name, unless it is only a
/// case variant of that name in that same folder, whose resolution stands.
fn in_own_folder(existing: &Path, resolved: PathBuf) -> io::Result<PathBuf> {
    let (Some(parent), Some(name)) = (existing.parent(), existing.file_name()) else {
        return Ok(resolved);
    };
    if resolved.file_name() == Some(name) {
        return Ok(resolved);
    }
    let folder = probe_canonical(parent)?;
    let case_variant = resolved.parent() == Some(folder.as_path())
        && resolved
            .file_name()
            .and_then(|found| found.to_str().zip(name.to_str()))
            .is_some_and(|(found, asked)| found.to_lowercase() == asked.to_lowercase());
    Ok(if case_variant {
        resolved
    } else {
        folder.join(name)
    })
}

// Test faults for the write-target probes: `(path, step, failures)` makes the
// next `failures` probes of `path` at `step` fail as Windows does while a
// save replaces the file (`u32::MAX`: every probe).
#[cfg(test)]
thread_local! {
    /// Root resolutions, ancestor lstats, ancestor resolutions: metadata-only
    /// API calls, measured without counting the platform's internal syscalls.
    pub(crate) static TARGET_PROBES: std::cell::Cell<(u64, u64, u64)> = const { std::cell::Cell::new((0, 0, 0)) };
    static PROBE_FAULTS: std::cell::RefCell<Vec<(PathBuf, ProbeStep, u32)>> = const { std::cell::RefCell::new(Vec::new()) };
    /// `(path, resolution)`: `path` resolves to `resolution`, as a page a
    /// rename is replacing can on Windows.
    static PROBE_DISPLACED: std::cell::RefCell<Vec<(PathBuf, PathBuf)>> = const { std::cell::RefCell::new(Vec::new()) };
}

#[cfg(test)]
#[derive(Clone, Copy, PartialEq, Eq)]
enum ProbeStep {
    Lstat,
    Canonical,
}

#[cfg(test)]
fn probe_fault(path: &Path, step: ProbeStep) -> io::Result<()> {
    PROBE_FAULTS.with(|faults| {
        for (at, kind, left) in faults.borrow_mut().iter_mut() {
            if *at == path && *kind == step && *left > 0 {
                if *left != u32::MAX {
                    *left -= 1;
                }
                return Err(io::Error::from(io::ErrorKind::PermissionDenied));
            }
        }
        Ok(())
    })
}

fn probe_lstat(path: &Path) -> io::Result<fs::Metadata> {
    #[cfg(test)]
    TARGET_PROBES.with(|counts| {
        let (r, l, c) = counts.get();
        counts.set((r, l + 1, c));
    });
    #[cfg(test)]
    probe_fault(path, ProbeStep::Lstat)?;
    fs::symlink_metadata(path)
}

fn probe_canonical(path: &Path) -> io::Result<PathBuf> {
    #[cfg(test)]
    TARGET_PROBES.with(|counts| {
        let (r, l, c) = counts.get();
        counts.set((r, l, c + 1));
    });
    #[cfg(test)]
    probe_fault(path, ProbeStep::Canonical)?;
    #[cfg(test)]
    if let Some(resolution) = PROBE_DISPLACED.with(|displaced| {
        displaced
            .borrow()
            .iter()
            .find_map(|(at, to)| (at == path).then(|| to.clone()))
    }) {
        return Ok(resolution);
    }
    canonical_existing_path(path)
}

/// Managed graph directories must retain their own identity, not merely land
/// somewhere under the graph after canonicalization. An in-graph symlink such as
/// `publish -> assets` passes a plain containment check but redirects generated
/// output onto user assets. Compare the deepest existing ancestor with its
/// expected canonical lexical location to reject any such alias.
pub(super) fn path_uses_managed_alias(root: &Path, target: &Path) -> bool {
    let canonical_root = canonical_existing_path(root).unwrap_or_else(|_| root.to_path_buf());
    let mut existing = target;
    while probe_lstat(existing).is_err() {
        let Some(parent) = existing.parent() else {
            return true;
        };
        existing = parent;
    }
    let Ok(relative) = existing.strip_prefix(root) else {
        return true;
    };
    canonical_existing_path(existing)
        .map(|actual| actual != canonical_root.join(relative))
        .unwrap_or(true)
}

#[cfg(test)]
mod write_target_probe_tests {
    use super::*;

    #[test]
    fn rg7_unchecked_graph_primitives_panic_before_writing() {
        let root = tempfile::tempdir().unwrap();
        let source = root.path().join("source.bin");
        fs::write(&source, b"kept").unwrap();
        for primitive in 0..4 {
            LAST_CHECKED_TARGET.with(|checked| *checked.borrow_mut() = None);
            let target = root.path().join(format!("unchecked-{primitive}"));
            let result = std::panic::catch_unwind(|| match primitive {
                0 => atomic_write_new(&target, b"new"),
                1 => atomic_write(&target, b"new"),
                2 => atomic_copy_new(&source, &target),
                _ => move_file_noreplace(&source, &target),
            });
            assert!(result.is_err(), "primitive {primitive} bypassed RG7");
            assert!(!target.exists(), "the assertion must precede I/O");
            assert_eq!(fs::read(&source).unwrap(), b"kept");
        }
    }

    #[test]
    fn rg7_checked_target_and_descendant_can_be_written() {
        let root = tempfile::tempdir().unwrap();
        checked_write_target(root.path(), root.path(), "fixture", None).unwrap();
        let target = root.path().join("checked.bin");
        atomic_write_new(&target, b"new").unwrap();
        assert_eq!(fs::read(&target).unwrap(), b"new");
        checked_write_target(root.path(), &target, "fixture", None).unwrap();
        atomic_write(&target, b"updated").unwrap();
        assert_eq!(fs::read(&target).unwrap(), b"updated");
        assert!(
            checked_write_target(root.path(), &root.path().join("../escape"), "fixture", None)
                .is_err()
        );
        assert!(std::panic::catch_unwind(|| assert_checked_target(&target)).is_err());
    }

    fn graph(tag: &str) -> (PathBuf, Graph) {
        let dir = std::env::temp_dir().join(format!("tine-probe-{tag}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(dir.join("pages")).unwrap();
        fs::write(dir.join("pages/Page.md"), "- before\n").unwrap();
        let graph = Graph::open(&dir);
        (dir, graph)
    }

    fn fault(path: PathBuf, step: ProbeStep, failures: u32) {
        PROBE_FAULTS.with(|faults| faults.borrow_mut().push((path, step, failures)));
    }

    fn clear() {
        PROBE_FAULTS.with(|faults| faults.borrow_mut().clear());
    }

    /// REG-WIN-WRITE-TARGET-TRANSIENT: Windows briefly refuses to open a page
    /// a save is replacing. Its parent still proves where the write lands.
    #[test]
    fn a_page_that_cannot_be_opened_is_checked_through_its_folder() {
        let (dir, graph) = graph("unopenable");
        let page = graph.root.join("pages/Page.md");
        fault(page.clone(), ProbeStep::Canonical, u32::MAX);
        let checked = graph.ensure_write_target(&page);
        clear();
        assert!(
            checked.is_ok(),
            "an ordinary page is inside its graph: {checked:?}"
        );
        let _ = fs::remove_dir_all(&dir);
    }

    /// REG-WIN-WRITE-TARGET-TRANSIENT: a page that cannot even be inspected is
    /// retried, and then reported as the I/O error it is, never as outside.
    #[test]
    fn a_page_that_cannot_be_inspected_is_retried_then_fails_without_escaping() {
        let (dir, graph) = graph("uninspectable");
        let page = graph.root.join("pages/Page.md");
        fault(page.clone(), ProbeStep::Lstat, 2);
        let brief = graph.ensure_write_target(&page);
        clear();
        assert!(brief.is_ok(), "a brief failure is retried: {brief:?}");

        fault(page.clone(), ProbeStep::Lstat, u32::MAX);
        let lasting = graph.ensure_write_target(&page);
        clear();
        let error = lasting.expect_err("a lasting failure fails the write");
        assert_eq!(error.kind(), io::ErrorKind::PermissionDenied, "{error}");
        assert!(!error.to_string().contains("escapes"), "{error}");
        let _ = fs::remove_dir_all(&dir);
    }

    /// REG-WIN-WRITE-TARGET-TRANSIENT, the submit's first check: a page's OS
    /// path resolves through the same re-probes and folder fallback.
    #[test]
    fn an_unopenable_page_still_resolves_for_its_save() {
        let (dir, graph) = graph("handoff");
        let page = graph.root.join("pages/Page.md");
        let expected = canonical_existing_path(&graph.root.join("pages"))
            .unwrap()
            .join("Page.md");
        fault(page.clone(), ProbeStep::Canonical, u32::MAX);
        let unopenable = canonical_existing_ancestor(&page).map(|(_, resolved)| resolved);
        clear();
        assert_eq!(unopenable.unwrap(), expected);
        fault(page.clone(), ProbeStep::Lstat, 3);
        let brief = canonical_existing_ancestor(&page).map(|(_, resolved)| resolved);
        clear();
        assert_eq!(
            brief.unwrap(),
            expected,
            "three brief failures are re-probed"
        );
        let _ = fs::remove_dir_all(&dir);
    }

    /// REG-WIN-WRITE-TARGET-DISPLACED: a page whose own resolution names where
    /// the file a rename replaced went is still checked where its name is, for
    /// the containment check and the submit's resolution alike; a case variant
    /// in its own folder keeps its resolution.
    #[test]
    fn a_page_resolving_to_its_replaced_file_is_checked_through_its_folder() {
        let (dir, graph) = graph("displaced");
        let page = graph.root.join("pages/Page.md");
        let folder = canonical_existing_path(&graph.root.join("pages")).unwrap();
        let displace = |to: PathBuf| {
            PROBE_DISPLACED.with(|displaced| *displaced.borrow_mut() = vec![(page.clone(), to)])
        };
        let undisplace = || PROBE_DISPLACED.with(|displaced| displaced.borrow_mut().clear());

        displace(
            dir.with_extension("volume")
                .join("$Extend/$Deleted/0100000000000A2B"),
        );
        let checked = graph.ensure_write_target(&page);
        let resolved = canonical_existing_ancestor(&page).map(|(_, resolved)| resolved);
        undisplace();
        assert!(checked.is_ok(), "the page is inside its graph: {checked:?}");
        assert_eq!(resolved.unwrap(), folder.join("Page.md"));

        displace(folder.join("PAGE.md"));
        let variant = canonical_existing_ancestor(&page).map(|(_, resolved)| resolved);
        undisplace();
        assert_eq!(variant.unwrap(), folder.join("PAGE.md"));

        displace(folder.join("Page.md~"));
        let renamed = canonical_existing_ancestor(&page).map(|(_, resolved)| resolved);
        undisplace();
        assert_eq!(
            renamed.unwrap(),
            folder.join("Page.md"),
            "another name is not a case variant"
        );
        let _ = fs::remove_dir_all(&dir);
    }

    /// REG-WIN-WRITE-TARGET-DISPLACED keeps the link guards: a displaced page
    /// under a linked folder is still refused, and a linked page keeps its own
    /// resolution.
    #[cfg(unix)]
    #[test]
    fn a_displaced_page_keeps_the_link_guards() {
        let (dir, graph) = graph("displaced-links");
        let outside = dir.with_extension("outside");
        fs::create_dir_all(&outside).unwrap();
        fs::write(outside.join("Page.md"), "x").unwrap();
        std::os::unix::fs::symlink(&outside, graph.root.join("linked")).unwrap();
        let under_link = graph.root.join("linked/Page.md");
        let link = graph.root.join("pages/escape.md");
        std::os::unix::fs::symlink(outside.join("Page.md"), &link).unwrap();
        PROBE_DISPLACED.with(|displaced| {
            *displaced.borrow_mut() = vec![
                (under_link.clone(), dir.join("$Extend/$Deleted/1")),
                (link.clone(), outside.join("Other.md")),
            ]
        });
        let folder_checked = graph.ensure_write_target(&under_link);
        let link_resolved = canonical_existing_ancestor(&link).map(|(_, resolved)| resolved);
        PROBE_DISPLACED.with(|displaced| displaced.borrow_mut().clear());
        assert!(folder_checked.is_err(), "a linked folder still escapes");
        assert_eq!(link_resolved.unwrap(), outside.join("Other.md"));
        let _ = fs::remove_dir_all(&dir);
        let _ = fs::remove_dir_all(&outside);
    }

    /// The parent fallback is for ordinary entries only: a link that cannot be
    /// resolved is still refused.
    #[cfg(unix)]
    #[test]
    fn an_unresolvable_link_is_still_refused() {
        let (dir, graph) = graph("unresolvable-link");
        let outside = dir.with_extension("outside");
        fs::create_dir_all(&outside).unwrap();
        fs::write(outside.join("foreign.md"), "x").unwrap();
        let link = graph.root.join("pages/escape.md");
        std::os::unix::fs::symlink(outside.join("foreign.md"), &link).unwrap();
        assert!(graph.ensure_write_target(&link).is_err());
        fault(link.clone(), ProbeStep::Canonical, u32::MAX);
        let checked = graph.ensure_write_target(&link);
        clear();
        let error = checked.expect_err("a link is never resolved through its folder");
        assert_eq!(
            error.kind(),
            io::ErrorKind::InvalidInput,
            "refused, not failed: {error}"
        );
        let _ = fs::remove_dir_all(&dir);
        let _ = fs::remove_dir_all(&outside);
    }
}
